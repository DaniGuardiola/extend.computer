//! Display input never opens a second control session. A bounded worker owns
//! the local injector; media polling only queues commands and drains replies.
use super::*;
use extend_computer_agent::{
    display::input::{Binding, Command as PointerCommand, Message, Reason},
    helper::{display::CaptureTarget, input::Applied},
    input::{InputEvent, ShortcutPolicy},
};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, Weak},
};

#[cfg(target_os = "macos")]
use std::{
    os::fd::AsRawFd,
    process::{Command, Stdio},
};

struct Driver {
    text_supported: bool,
    shortcut_policy: Option<ShortcutPolicy>,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    buffer: Vec<u8>,
    binding: Binding,
    target: CaptureTarget,
    sequence: u64,
}
impl Driver {
    #[cfg(not(target_os = "macos"))]
    fn start(_: &std::path::Path, _: Binding, _: CaptureTarget) -> Result<Self> {
        anyhow::bail!("Display input requires a supported native source adapter")
    }
    #[cfg(target_os = "macos")]
    fn start(path: &std::path::Path, binding: Binding, target: CaptureTarget) -> Result<Self> {
        use extend_computer_agent::helper::input::{DisplayReady, DisplayTarget};
        let descriptor = DisplayTarget::new(&binding, &target);
        let mut child = Command::new(path)
            .arg("inject-display")
            .arg(descriptor.to_arg()?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().context("Cursor input unavailable")?;
        let stdout = child.stdout.take().context("Cursor output unavailable")?;
        for fd in [stdin.as_raw_fd(), stdout.as_raw_fd()] {
            // Small atomic pipe writes must not wedge the media worker's barrier.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("Cannot bound Cursor helper I/O");
            }
        }
        let mut driver = Self {
            text_supported: false,
            shortcut_policy: None,
            child,
            stdin: Some(stdin),
            stdout,
            buffer: Vec::new(),
            binding,
            target,
            sequence: 0,
        };
        let ready = DisplayReady::parse_line(&driver.line(Duration::from_secs(2))?)?;
        ensure!(
            ready.target == descriptor,
            "Cursor helper accepted a different capture target"
        );
        driver.text_supported = ready.text_supported.unwrap_or(false);
        driver.shortcut_policy = ready.shortcut_policy;
        Ok(driver)
    }
    fn line(&mut self, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                ensure!(end <= 4096, "Cursor helper reply too large");
                let bytes: Vec<_> = self.buffer.drain(..=end).collect();
                return Ok(String::from_utf8(bytes[..end].to_vec())?);
            }
            ensure!(self.buffer.len() <= 4096, "Cursor helper reply too large");
            ensure!(
                Instant::now() < deadline,
                "Cursor helper acknowledgement timed out"
            );
            let mut bytes = [0; 512];
            match self.stdout.read(&mut bytes) {
                Ok(0) => anyhow::bail!("Cursor helper closed"),
                Ok(n) => self.buffer.extend_from_slice(&bytes[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    fn apply(&mut self, sequence: u64, command: &PointerCommand) -> Result<()> {
        if matches!(
            command,
            PointerCommand::Input {
                event: InputEvent::Text { .. },
                ..
            }
        ) {
            ensure!(self.text_supported, "Native helper does not support text");
        }
        let bytes = self
            .target
            .event(&self.binding, sequence, command)?
            .to_bytes()?;
        ensure!(bytes.len() <= 1024, "Cursor helper command too large");
        let input = self.stdin.as_mut().context("Cursor helper stopped")?;
        input.write_all(&bytes)?;
        input.write_all(b"\n")?;
        let ack = Applied::parse_line(&self.line(Duration::from_millis(400))?)?;
        ensure!(
            ack == Applied::new(&self.binding, sequence),
            "Cursor helper acknowledgement binding differs"
        );
        self.sequence = sequence;
        Ok(())
    }
    fn release(&mut self) -> Result<()> {
        if self.stdin.is_none() {
            return Ok(());
        }
        let outcome = self.apply(
            self.sequence
                .checked_add(1)
                .context("Cursor sequence exhausted")?,
            &PointerCommand::Stop,
        );
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                return outcome;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        terminate(&mut self.child);
        anyhow::bail!("Cursor helper failed to stop after release")
    }
}
fn terminate(child: &mut Child) {
    #[cfg(target_os = "macos")]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = child.kill();
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.release();
        if self.child.try_wait().ok().flatten().is_none() {
            terminate(&mut self.child);
            let until = Instant::now() + Duration::from_millis(500);
            while Instant::now() < until && self.child.try_wait().ok().flatten().is_none() {
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn permission_result_current(
    result_epoch: u64,
    epoch: u64,
    has_driver: bool,
    current_session: bool,
) -> bool {
    has_driver && current_session && result_epoch == epoch
}

struct Work {
    received: Instant,
    message: Message,
    target: Option<CaptureTarget>,
    config: Config,
    transport_epoch: u64,
}
struct Shared {
    work: VecDeque<Work>,
    replies: VecDeque<Message>,
    closed: bool,
    transport_epoch: u64,
    transport_recovering: bool,
    resume_after: Option<Instant>,
}
impl Shared {
    fn transport_blocked(&self, now: Instant) -> bool {
        self.transport_recovering || self.resume_after.is_some_and(|until| now < until)
    }

    fn work_current(&self, epoch: u64, now: Instant) -> bool {
        epoch == self.transport_epoch && !self.transport_blocked(now)
    }
}
pub struct Worker {
    shared: Arc<Mutex<Shared>>,
    barrier: mpsc::SyncSender<mpsc::SyncSender<Result<()>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        app: Weak<Desktop>,
        id: u64,
        peer: String,
        text_negotiated: bool,
        shortcuts_negotiated: bool,
    ) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            work: VecDeque::new(),
            replies: VecDeque::new(),
            closed: false,
            transport_epoch: 0,
            transport_recovering: false,
            resume_after: None,
        }));
        let active = shared.clone();
        let (barrier, barriers) = mpsc::sync_channel::<mpsc::SyncSender<Result<()>>>(2);
        // A single bounded probe worker per display session keeps subprocess/XPC
        // waits off the injector. Results carry both permission and driver epochs.
        let (permission_request, permission_requests) = mpsc::sync_channel::<u64>(1);
        let (permission_result, permission_results) = mpsc::channel();
        let permission_app = app.clone();
        std::thread::spawn(move || {
            while let Ok(epoch) = permission_requests.recv() {
                let Some(app) = permission_app.upgrade() else {
                    break;
                };
                if app.closing.load(Ordering::SeqCst) {
                    break;
                }
                let ticket = app.begin_permission_check();
                let result = native::permissions_for_route(&app.helper, app.usb_session(id));
                if permission_result.send((epoch, ticket, result)).is_err() {
                    break;
                }
            }
        });
        let thread = std::thread::spawn(move || {
            let mut permission_pending = false;
            let mut driver: Option<Driver> = None;
            let mut epoch = 0u64;
            let mut last_input = Instant::now();
            let mut checked = Instant::now() - Duration::from_secs(2);
            let revoked = |driver: &mut Option<Driver>, reason: Reason| {
                if let Some(mut owned) = driver.take() {
                    let binding = owned.binding.clone();
                    let result = owned.release();
                    if let Err(error) = &result {
                        eprintln!("Display input release failed: {error:#}");
                    }
                    let mut state = active.lock().unwrap();
                    state.work.clear();
                    if state.replies.len() < 64 {
                        state
                            .replies
                            .push_back(Message::Revoked { binding, reason });
                    }
                    result
                } else {
                    Ok(())
                }
            };
            loop {
                while let Ok(reply) = barriers.try_recv() {
                    let result = revoked(&mut driver, Reason::Changed);
                    active.lock().unwrap().work.clear();
                    let _ = reply.send(result);
                }
                let Some(app) = app.upgrade() else { break };
                if active.lock().unwrap().closed || app.closing.load(Ordering::SeqCst) {
                    break;
                }
                while let Ok((result_epoch, ticket, result)) = permission_results.try_recv() {
                    permission_pending = false;
                    let current_session =
                        app.inner.lock().unwrap().job.as_ref().is_some_and(|j| {
                            j.view.id == id && !j.cancelled.load(Ordering::SeqCst)
                        });
                    if permission_result_current(
                        result_epoch,
                        epoch,
                        driver.is_some(),
                        current_session,
                    ) {
                        let _ = app.apply_permissions_for_session(ticket, result, Some(id));
                    }
                }
                if driver.is_some()
                    && !permission_pending
                    && checked.elapsed() >= Duration::from_secs(1)
                {
                    permission_pending = permission_request.try_send(epoch).is_ok();
                    checked = Instant::now();
                }
                if driver.is_some()
                    && (policy(&app, id, &peer).is_err()
                        || last_input.elapsed() >= Duration::from_secs(2))
                {
                    let reason = policy(&app, id, &peer).err().unwrap_or(Reason::Timeout);
                    let _ = revoked(&mut driver, reason);
                }
                let work = active.lock().unwrap().work.pop_front();
                let Some(work) = work else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                if work.received.elapsed() >= Duration::from_secs(2) {
                    let _ = revoked(&mut driver, Reason::Timeout);
                    continue;
                }
                let result = (|| -> Result<Option<Message>> {
                    match work.message {
                        Message::Request { request } => {
                            if !active
                                .lock()
                                .unwrap()
                                .work_current(work.transport_epoch, Instant::now())
                            {
                                return Ok(Some(Message::Denied {
                                    request,
                                    reason: Reason::Changed,
                                }));
                            }
                            let _ = app.refresh_permissions_for_route(app.usb_session(id));
                            if let Err(reason) = policy(&app, id, &peer) {
                                return Ok(Some(Message::Denied { request, reason }));
                            }
                            if driver.is_some() {
                                return Ok(Some(Message::Denied {
                                    request,
                                    reason: Reason::NotReady,
                                }));
                            }
                            let Some(target) = work.target else {
                                return Ok(Some(Message::Denied {
                                    request,
                                    reason: Reason::NotReady,
                                }));
                            };
                            target.validate()?;
                            let geometry = target.geometry()?;
                            geometry.validate_for_config(&work.config)?;
                            epoch = epoch
                                .checked_add(1)
                                .context("Display input epoch exhausted")?;
                            let binding = Binding::new(epoch, target.configuration_id);
                            let start = Driver::start(&app.helper, binding.clone(), target);
                            let Ok(owned) = start else {
                                return Ok(Some(Message::Denied {
                                    request,
                                    reason: Reason::Permission,
                                }));
                            };
                            let text_supported = text_negotiated && owned.text_supported;
                            let shortcut_policy = if shortcuts_negotiated {
                                owned.shortcut_policy
                            } else {
                                None
                            };
                            driver = Some(owned);
                            if !active
                                .lock()
                                .unwrap()
                                .work_current(work.transport_epoch, Instant::now())
                            {
                                let _ = revoked(&mut driver, Reason::Changed);
                                return Ok(Some(Message::Denied {
                                    request,
                                    reason: Reason::Changed,
                                }));
                            }
                            if policy(&app, id, &peer).is_err() {
                                let _ = revoked(&mut driver, Reason::Stopped);
                                return Ok(Some(Message::Denied {
                                    request,
                                    reason: Reason::Stopped,
                                }));
                            }
                            last_input = work.received;
                            Ok(Some(Message::Granted {
                                request,
                                binding,
                                geometry,
                                text_supported,
                                shortcut_policy,
                            }))
                        }
                        Message::Command {
                            binding,
                            sequence,
                            command,
                        } => {
                            if !active
                                .lock()
                                .unwrap()
                                .work_current(work.transport_epoch, Instant::now())
                            {
                                return Ok(None);
                            }
                            let Some(owned) = driver.as_mut() else {
                                return Ok(None);
                            };
                            if binding != owned.binding || sequence <= owned.sequence {
                                return Ok(None);
                            }
                            if let Err(reason) = policy(&app, id, &peer) {
                                let _ = revoked(&mut driver, reason);
                                return Ok(None);
                            }
                            if !active
                                .lock()
                                .unwrap()
                                .work_current(work.transport_epoch, Instant::now())
                            {
                                return Ok(None);
                            }
                            owned.apply(sequence, &command)?;
                            last_input = work.received;
                            if matches!(command, PointerCommand::Stop) {
                                let _ = revoked(&mut driver, Reason::Stopped);
                            }
                            Ok(Some(Message::Applied { binding, sequence }))
                        }
                        _ => anyhow::bail!("Wrong display input direction"),
                    }
                })();
                match result {
                    Ok(Some(message)) => {
                        let mut state = active.lock().unwrap();
                        if let Message::Granted { request, .. } = &message {
                            if !state.work_current(work.transport_epoch, Instant::now()) {
                                let request = *request;
                                drop(state);
                                let _ = revoked(&mut driver, Reason::Changed);
                                let mut state = active.lock().unwrap();
                                if state.replies.len() < 64 {
                                    state.replies.push_back(Message::Denied {
                                        request,
                                        reason: Reason::Changed,
                                    });
                                }
                                continue;
                            }
                        }
                        if state.replies.len() < 64 {
                            state.replies.push_back(message)
                        } else {
                            drop(state);
                            let _ = revoked(&mut driver, Reason::Stopped);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("Display input stopped: {error:#}");
                        let _ = revoked(&mut driver, Reason::Stopped);
                    }
                }
            }
            let _ = revoked(&mut driver, Reason::Stopped);
        });
        Self {
            shared,
            barrier,
            thread: Some(thread),
        }
    }
    pub fn enqueue(
        &self,
        message: Message,
        target: Option<CaptureTarget>,
        config: &Config,
    ) -> Result<()> {
        message.validate_sender(extend_computer_agent::display::input::Role::Receiver)?;
        let mut state = self.shared.lock().unwrap();
        ensure!(!state.closed, "Display input worker stopped");
        if state.transport_blocked(Instant::now()) {
            if let Message::Request { request } = message {
                ensure!(
                    state.replies.len() < 64,
                    "Display input reply queue overflow"
                );
                state.replies.push_back(Message::Denied {
                    request,
                    reason: Reason::Changed,
                });
            }
            return Ok(());
        }
        if let Message::Command {
            binding,
            sequence,
            command,
        } = &message
        {
            if state.work.iter().any(|work| matches!(&work.message, Message::Command {binding: old, sequence: previous,..} if old==binding && previous>=sequence)) { return Ok(()) }
            if matches!(command, PointerCommand::Stop | PointerCommand::Input {event:InputEvent::Release,..}) {
                state.work.retain(|work| !matches!(&work.message, Message::Command {binding: old,..} if old==binding));
            } else if matches!(command, PointerCommand::Move {..}) && state.work.back().is_some_and(|work| matches!(&work.message, Message::Command {binding: old, command:PointerCommand::Move {..},..} if old==binding)) {
                state.work.pop_back();
            }
        }
        if state.work.len() >= 64 {
            state.work.clear();
            drop(state);
            let (tx, _) = mpsc::sync_channel(1);
            let _ = self.barrier.try_send(tx);
            anyhow::bail!("Display input queue overflow")
        }
        let transport_epoch = state.transport_epoch;
        state.work.push_back(Work {
            received: Instant::now(),
            message,
            target,
            config: config.clone(),
            transport_epoch,
        });
        Ok(())
    }
    pub fn reply(&self) -> Option<Message> {
        let mut state = self.shared.lock().unwrap();
        while let Some(message) = state.replies.pop_front() {
            if state.transport_blocked(Instant::now())
                && matches!(message, Message::Granted { .. } | Message::Applied { .. })
            {
                continue;
            }
            return Some(message);
        }
        None
    }
    pub fn restore_reply(&self, message: Message) {
        let mut state = self.shared.lock().unwrap();
        if state.transport_blocked(Instant::now())
            && matches!(message, Message::Granted { .. } | Message::Applied { .. })
        {
            return;
        }
        if state.replies.len() >= 64 {
            state.replies.clear();
            drop(state);
            self.revoke();
        } else {
            state.replies.push_front(message);
        }
    }
    pub fn revoke(&self) {
        let (tx, _) = mpsc::sync_channel(1);
        let _ = self.barrier.try_send(tx);
    }
    /// Retire all pre-outage work and grants. Two seconds after the exact
    /// successful retry also bounds every receiver's original request lifetime.
    pub fn transport_recovery(&self, epoch: u64, ended: Option<Instant>) -> Result<()> {
        let mut state = self.shared.lock().unwrap();
        ensure!(
            epoch >= state.transport_epoch,
            "Stale input transport epoch"
        );
        let changed = epoch != state.transport_epoch;
        state.transport_epoch = epoch;
        state.transport_recovering = ended.is_none();
        state.resume_after = ended.map(|at| at + super::receiver::TIMEOUT);
        if changed {
            let pending = std::mem::take(&mut state.work);
            let replies = std::mem::take(&mut state.replies);
            for message in pending.into_iter().map(|work| work.message).chain(replies) {
                let retained = match message {
                    Message::Request { request } | Message::Granted { request, .. } => {
                        Some(Message::Denied {
                            request,
                            reason: Reason::Changed,
                        })
                    }
                    message @ (Message::Denied { .. } | Message::Revoked { .. }) => Some(message),
                    _ => None,
                };
                if let Some(message) = retained {
                    if state.replies.len() < 64 {
                        state.replies.push_back(message);
                    }
                }
            }
        }
        drop(state);
        if changed {
            self.revoke();
        }
        Ok(())
    }
    pub fn suspend(&self) -> Result<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.barrier.send(tx)?;
        rx.recv_timeout(Duration::from_secs(5))
            .context("Display input release barrier timed out")??;
        Ok(())
    }
}
fn policy(app: &Desktop, id: u64, peer: &str) -> std::result::Result<(), Reason> {
    let inner = app.inner.lock().unwrap();
    if !inner.receiving_enabled {
        return Err(Reason::ReceivingOff);
    }
    let usb = inner
        .job
        .as_ref()
        .is_some_and(|job| job.view.id == id && job.view.route == Some("usb"));
    if !(if usb {
        inner.permissions.available && inner.permissions.post
    } else {
        inner.permissions.can_receive()
    }) {
        return Err(Reason::Permission);
    }
    if !inner.job.as_ref().is_some_and(|job| {
        job.view.id == id
            && job.view.peer.as_deref() == Some(peer)
            && job.view.kind == SessionKind::Outgoing
            && job.view.phase == Phase::Connected
            && !job.cancelled.load(Ordering::SeqCst)
    }) {
        return Err(Reason::NotReady);
    }
    drop(inner);
    if app
        .store()
        .and_then(|s| s.peer(peer))
        .ok()
        .flatten()
        .is_none()
    {
        return Err(Reason::Stopped);
    }
    Ok(())
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.lock().unwrap().closed = true;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use extend_computer_agent::{display::input::Size, helper::display::Bounds};
    #[test]
    fn recovery_flushes_multiple_requests_grants_and_blocks_stale_worker_epochs() {
        let now = Instant::now();
        let (barrier, _barriers) = mpsc::sync_channel(2);
        let worker = Worker {
            shared: Arc::new(Mutex::new(Shared {
                work: VecDeque::new(),
                replies: VecDeque::new(),
                closed: false,
                transport_epoch: 0,
                transport_recovering: false,
                resume_after: None,
            })),
            barrier,
            thread: None,
        };
        let config = Config {
            mode: extend_computer_agent::session::display::Mode::Mirror,
            width: 1000,
            height: 800,
            fps: 60,
            bitrate: 8_000_000,
            retina: true,
            auto_size: false,
            codec: extend_computer_agent::session::display::VideoCodec::Hevc,
            separate_cursor: true,
        };
        for request in [1, 2] {
            worker
                .enqueue(Message::Request { request }, None, &config)
                .unwrap();
        }
        let grant = Message::Granted {
            request: 1,
            binding: Binding::new(1, 3),
            geometry: target().geometry().unwrap(),
            text_supported: false,
            shortcut_policy: None,
        };
        worker
            .shared
            .lock()
            .unwrap()
            .replies
            .push_back(grant.clone());
        worker.transport_recovery(1, None).unwrap();
        assert!(worker.shared.lock().unwrap().work.is_empty());
        let replies: Vec<_> = std::iter::from_fn(|| worker.reply()).collect();
        for request in [1, 2] {
            assert!(replies.iter().any(|m| matches!(m, Message::Denied {request: id, reason:Reason::Changed} if *id==request)));
        }
        assert!(!replies.iter().any(|m| matches!(m, Message::Granted { .. })));
        worker.restore_reply(grant);
        assert!(worker.reply().is_none());
        worker
            .enqueue(Message::Request { request: 3 }, None, &config)
            .unwrap();
        assert!(matches!(
            worker.reply(),
            Some(Message::Denied {
                request: 3,
                reason: Reason::Changed
            })
        ));
        assert!(worker.shared.lock().unwrap().work.is_empty());

        worker.transport_recovery(1, Some(now)).unwrap();
        let state = worker.shared.lock().unwrap();
        let boundary = now + Duration::from_secs(2);
        assert!(state.transport_blocked(boundary - Duration::from_nanos(1)));
        assert!(!state.transport_blocked(boundary));
        assert!(!state.work_current(0, boundary));
        assert!(state.work_current(1, boundary));
        drop(state);
        worker.shared.lock().unwrap().resume_after = Some(now - Duration::from_nanos(1));
        worker
            .enqueue(Message::Request { request: 4 }, None, &config)
            .unwrap();
        assert_eq!(worker.shared.lock().unwrap().work.len(), 1);
        assert!(worker.reply().is_none());
    }

    fn target() -> CaptureTarget {
        CaptureTarget {
            configuration_id: 3,
            display_id: 42,
            bounds: Bounds {
                x: -2000.0,
                y: -100.0,
                width: 2000.0,
                height: 1000.0,
            },
            canvas: Size {
                width: 1000,
                height: 800,
            },
            raster: Size {
                width: 2000,
                height: 1600,
            },
            content: Bounds {
                x: 0.0,
                y: 150.0,
                width: 1000.0,
                height: 500.0,
            },
        }
    }
    #[test]
    fn worker_queue_coalesces_only_moves_and_always_admits_owned_release() {
        let (barrier, _barriers) = mpsc::sync_channel(2);
        let worker = Worker {
            shared: Arc::new(Mutex::new(Shared {
                work: VecDeque::new(),
                replies: VecDeque::new(),
                closed: false,
                transport_epoch: 0,
                transport_recovering: false,
                resume_after: None,
            })),
            barrier,
            thread: None,
        };
        let config = Config {
            mode: extend_computer_agent::session::display::Mode::Mirror,
            width: 1000,
            height: 800,
            fps: 30,
            bitrate: 8_000_000,
            retina: true,
            auto_size: false,
            codec: extend_computer_agent::session::display::VideoCodec::H264,
            separate_cursor: true,
        };
        let binding = Binding::new(1, 3);
        let message = |sequence, command| Message::Command {
            binding: binding.clone(),
            sequence,
            command,
        };
        let motion = || PointerCommand::Move {
            point: extend_computer_agent::display::input::Point { x: 0.5, y: 0.5 },
        };
        worker.enqueue(message(1, motion()), None, &config).unwrap();
        worker.enqueue(message(2, motion()), None, &config).unwrap();
        assert_eq!(worker.shared.lock().unwrap().work.len(), 1);
        worker
            .enqueue(
                message(
                    3,
                    PointerCommand::Input {
                        point: Some(extend_computer_agent::display::input::Point {
                            x: 0.5,
                            y: 0.5,
                        }),
                        event: InputEvent::Button {
                            button: 0,
                            down: true,
                            clicks: 1,
                        },
                    },
                ),
                None,
                &config,
            )
            .unwrap();
        worker.enqueue(message(4, motion()), None, &config).unwrap();
        assert_eq!(worker.shared.lock().unwrap().work.len(), 3);
        for sequence in 5..=65 {
            worker
                .enqueue(message(sequence, PointerCommand::Alive), None, &config)
                .unwrap();
        }
        assert_eq!(worker.shared.lock().unwrap().work.len(), 64);
        worker
            .enqueue(
                message(
                    66,
                    PointerCommand::Input {
                        point: None,
                        event: InputEvent::Release,
                    },
                ),
                None,
                &config,
            )
            .unwrap();
        assert_eq!(worker.shared.lock().unwrap().work.len(), 1);
        worker
            .enqueue(message(65, motion()), None, &config)
            .unwrap();
        assert_eq!(worker.shared.lock().unwrap().work.len(), 1);
        for sequence in 67..=129 {
            worker
                .enqueue(message(sequence, PointerCommand::Alive), None, &config)
                .unwrap();
        }
        assert!(worker
            .enqueue(message(130, PointerCommand::Alive), None, &config)
            .is_err());
        assert!(worker.shared.lock().unwrap().work.is_empty());
    }
    fn fake_driver(no_ack: bool) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cursor");
        let body=format!("#!/usr/bin/python3\nimport sys,json\nd=json.loads(sys.argv[2])\nfor key in ['bounds','content']:\n d[key]={{k:int(v) if float(v).is_integer() else v for k,v in d[key].items()}}\nprint('READY '+json.dumps(d),flush=True)\nfor line in sys.stdin:\n v=json.loads(line)\n if {}: continue\n print('APPLIED '+json.dumps({{k:v[k] for k in ['session','epoch','revision','sequence']}}),flush=True)\n",if no_ack {"True"} else {"False"});
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (temp, path)
    }
    #[test]
    fn scoped_shortcut_policy_requires_exact_helper_ready_and_rejects_unknown_value() {
        for (policy, valid) in [("command", true), ("meta", false)] {
            let (_temp, path) = fake_driver(false);
            let body = std::fs::read_to_string(&path).unwrap().replace(
                "print('READY '+json.dumps(d),flush=True)",
                &format!(
                    "d['shortcut_policy']='{policy}'\nprint('READY '+json.dumps(d),flush=True)"
                ),
            );
            std::fs::write(&path, body).unwrap();
            let result = Driver::start(&path, Binding::new(1, 3), target());
            assert_eq!(result.is_ok(), valid);
            if let Ok(driver) = result {
                assert_eq!(driver.shortcut_policy, Some(ShortcutPolicy::Command));
            }
        }
    }
    #[test]
    fn legacy_scoped_helper_never_receives_text() {
        let (_temp, path) = fake_driver(false);
        let mut driver = Driver::start(&path, Binding::new(1, 3), target()).unwrap();
        assert!(!driver.text_supported);
        assert_eq!(driver.shortcut_policy, None);
        assert!(driver
            .apply(
                1,
                &PointerCommand::Input {
                    point: None,
                    event: InputEvent::Text { value: "é".into() }
                }
            )
            .is_err());
        assert_eq!(driver.sequence, 0);
        driver.apply(1, &PointerCommand::Alive).unwrap();
        driver.release().unwrap();
    }
    #[test]
    fn helper_driver_acknowledges_exact_binding_then_releases_and_exits() {
        let (_temp, path) = fake_driver(false);
        let mut driver = Driver::start(&path, Binding::new(1, 3), target()).unwrap();
        driver.apply(4, &PointerCommand::Alive).unwrap();
        assert_eq!(driver.sequence, 4);
        driver.release().unwrap();
        assert!(driver.child.try_wait().unwrap().is_some());
        driver.release().unwrap();
    }
    #[test]
    fn helper_ack_timeout_does_not_leave_a_child_running() {
        let (_temp, path) = fake_driver(true);
        let mut driver = Driver::start(&path, Binding::new(1, 3), target()).unwrap();
        let started = Instant::now();
        assert!(driver.apply(1, &PointerCommand::Alive).is_err());
        assert!(driver.release().is_err());
        assert!(driver.child.try_wait().unwrap().is_some());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

#[cfg(test)]
mod permission_refresh_tests {
    use super::*;
    #[test]
    fn delayed_permission_result_cannot_affect_replacement_or_released_driver() {
        assert!(permission_result_current(7, 7, true, true));
        assert!(!permission_result_current(6, 7, true, true));
        assert!(!permission_result_current(7, 7, false, true));
        assert!(!permission_result_current(7, 7, true, false));
    }

    #[test]
    fn cable_permission_probe_requires_current_uncancelled_usb_session() {
        let files = tempfile::tempdir().unwrap();
        let app = Desktop::new(files.path().join("desktop"), "unused-helper".into()).unwrap();
        let (id, cancelled) = app.reserve(SessionKind::Outgoing, None).unwrap();
        assert!(!app.usb_session(id));
        app.inner.lock().unwrap().job.as_mut().unwrap().view.route = Some("local");
        assert!(!app.usb_session(id));
        app.inner.lock().unwrap().job.as_mut().unwrap().view.route = Some("usb");
        assert!(app.usb_session(id));
        assert!(!app.usb_session(id + 1));
        cancelled.store(true, Ordering::SeqCst);
        assert!(!app.usb_session(id));
        app.inner.lock().unwrap().job = None;
        assert!(!app.usb_session(id));
    }
}
