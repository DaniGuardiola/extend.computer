//! Incoming listener and the GUI consent adapter for native input.
use super::*;
use extend_computer_agent::{
    cursor::NativeSink,
    low_jitter::ManagedCursor,
    session::{self, ControlApproval, CursorSink, Decision, DisplaySize},
};
use std::net::TcpListener;

#[derive(Debug)]
pub(super) struct ReceivingPermissionRequired;
impl std::fmt::Display for ReceivingPermissionRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Allow Accessibility and Wi-Fi optimization in Settings before allowing connections.")
    }
}
impl std::error::Error for ReceivingPermissionRequired {}

impl Desktop {
    pub fn receive(self: &Arc<Self>, pairing: bool) -> Result<()> {
        self.receive_at(pairing, 48177)
    }
    pub(super) fn receive_at(self: &Arc<Self>, pairing: bool, port: u16) -> Result<()> {
        let mut preference = self.receiving_preference.lock().unwrap();
        self.receive_with_preference(pairing, port, &mut preference)
    }
    pub(super) fn receive_with_preference(
        self: &Arc<Self>,
        pairing: bool,
        port: u16,
        preference: &mut bool,
    ) -> Result<()> {
        ensure!(
            !self.closing.load(Ordering::SeqCst),
            "extend.computer is closing."
        );
        if !pairing {
            let permissions = self.refresh_permissions()?;
            if !permissions.can_receive() {
                return Err(ReceivingPermissionRequired.into());
            }
        }
        let identity = self.identity()?;
        let name = self.local_name();
        let mut inner = self.inner.lock().unwrap();
        if pairing {
            ensure!(
                inner.job.is_none(),
                "Finish the current connection before creating a new code."
            );
        }
        if let Some(stop) = &inner.listener {
            ensure!(
                !stop.load(Ordering::SeqCst),
                "Incoming connections are stopping. Try again shortly."
            );
            if !pairing {
                preferences::save(&self.root, true)?;
                *preference = true;
                inner.receiving_enabled = true;
            }
            if pairing {
                inner.pairing_open = true;
                inner.code = Some(PairCode::new());
                refresh_pairing(&mut inner, &name, &self.discovery_id)?;
            }
            return Ok(());
        }
        let listener = TcpListener::bind(("0.0.0.0", port)).context(
            "extend.computer's receiving port is in use. Close the other extend.computer receiver and try again.",
        )?;
        listener.set_nonblocking(true)?;
        if !pairing {
            preferences::save(&self.root, true)?;
            *preference = true;
        }
        let stop = Arc::new(AtomicBool::new(false));
        inner.listener_port = port;
        inner.pairing_open = pairing;
        refresh_pairing(&mut inner, &name, &self.discovery_id)?;
        inner.listener = Some(stop.clone());
        inner.receiving_enabled = !pairing;
        if !pairing {
        }
        inner.error = None;
        drop(inner);
        let app = self.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                {
                    let mut inner = app.inner.lock().unwrap();
                    if let Err(error) = refresh_pairing(&mut inner, &app.local_name(), &app.discovery_id) {
                        inner.error = Some(format!("Could not make this device visible: {error}"));
                        inner.pairing_open = false;
                        inner.code = None;
                        inner.advertisement = None;
                    }
                }
                match listener.accept() {
                    Ok((socket, address)) => {
                        if stop.load(Ordering::SeqCst) {
                            let _ = socket.shutdown(Shutdown::Both);
                            break;
                        }
                        let Ok((id, cancelled)) = app.reserve(SessionKind::Incoming, None) else {
                            let _ = socket.shutdown(Shutdown::Both);
                            continue;
                        };
                        let result = (|| -> Result<()> {
                            // macOS can inherit O_NONBLOCK from the listener. The encrypted engine uses deadline-bounded blocking reads.
                            socket.set_nonblocking(false)?;
                            app.set_socket(id, &socket)?;
                            let code = app.inner.lock().unwrap().code.take();
                            let expires = code.as_ref().map(|c| c.expires);
                            let mut window = code.map(|c| c.window);
                            let mut sink = ManagedCursor::required(
                                GuiSink {
                                    inner: NativeSink::new(Some(app.helper.clone())),
                                    app: app.clone(),
                                    job: id,
                                    cancelled: cancelled.clone(),
                                },
                                address.ip(),
                            );
                            let store = app.store()?;
                            let served = session::serve_connection_with_verification(
                                socket,
                                &identity,
                                &store,
                                &mut window,
                                |peer, known| {
                                    if known {
                                        app.stage(id, Phase::Connecting, Some(peer));
                                        return Decision::Once;
                                    }
                                    app.stage(id, Phase::Approval, Some(peer));
                                    let answer = app.approvals.ask("pair", peer, || {
                                        app.cancelled(id, &cancelled) || stop.load(Ordering::SeqCst)
                                    });
                                    if answer == Answer::Deny {
                                        return Decision::Deny;
                                    }
                                    Decision::Remember
                                },
                                &mut sink,
                                |peer, symbols| {
                                    app.stage(id, Phase::Approval, Some(peer));
                                    app.approvals.ask_verified(
                                        "verify",
                                        peer,
                                        Some(*symbols),
                                        || {
                                            app.cancelled(id, &cancelled)
                                                || stop.load(Ordering::SeqCst)
                                        },
                                    ) == Answer::Remember
                                },
                            );
                            if let (Some(window), Some(expires)) = (window, expires) {
                                if window.code().is_some()
                                    && expires > Instant::now()
                                    && !stop.load(Ordering::SeqCst)
                                {
                                    let mut inner = app.inner.lock().unwrap();
                                    if inner.pairing_open
                                        && inner.code.is_none()
                                        && !cancelled.load(Ordering::SeqCst)
                                    {
                                        inner.code = Some(PairCode { window, expires });
                                    }
                                }
                            }
                            if let Some(peer) = app
                                .inner
                                .lock()
                                .unwrap()
                                .job
                                .as_ref()
                                .and_then(|j| j.view.peer.clone())
                            {
                                if store.peer(&peer).ok().flatten().is_some() {
                                    app.clear_removal(&peer)?;
                                }
                                if store.peer(&peer).ok().flatten().is_some()
                                    && !app.devices.lock().unwrap().contains_key(&peer)
                                {
                                    app.save_device(
                                        &peer,
                                        Device {
                                            name: format!("Device · {}", address.ip()),
                                            address: SocketAddr::new(address.ip(), 48177)
                                                .to_string(),
                                            edge: "left".into(),
                                        },
                                    )?;
                                }
                            }
                            served
                        })();
                        app.finish(id, result);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    Err(e) => {
                        app.inner.lock().unwrap().error =
                            Some(format!("Could not receive a connection: {e}"));
                        break;
                    }
                }
            }
            drop(listener);
            let mut inner = app.inner.lock().unwrap();
            if inner
                .listener
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &stop))
            {
                inner.listener = None;
                inner.receiving_enabled = false;
                inner.pairing_open = false;
                inner.advertisement = None;
                inner.code = None;
            }
        });
        Ok(())
    }
}
// Discovery is a dialog-scoped presence, separate from the trusted-peer listener.
fn refresh_pairing(inner: &mut Inner, name: &str, discovery_id: &str) -> Result<()> {
    if !inner.pairing_open || inner.job.is_some() {
        inner.advertisement = None;
        return Ok(());
    }
    if inner
        .code
        .as_ref()
        .is_none_or(|code| code.expires <= Instant::now() || code.window.code().is_none())
    {
        inner.code = Some(PairCode::new());
    }
    if inner.advertisement.is_none() {
        inner.advertisement = Some(
            extend_computer_agent::discovery::Advertisement::start_pairing(
                inner.listener_port,
                name,
                discovery_id,
            )?,
        );
    }
    Ok(())
}

struct GuiSink {
    inner: NativeSink,
    app: Arc<Desktop>,
    job: u64,
    cancelled: Arc<AtomicBool>,
}
impl CursorSink for GuiSink {
    fn device_name(&self) -> Option<String> {
        Some(self.app.local_name())
    }
    fn peer_name(&mut self, peer: &str, name: &str) -> Result<()> {
        self.app.update_peer_name(peer, name)
    }

    fn unpaired(&mut self, peer: &str) -> Result<()> {
        self.app.received_unpair(peer)
    }
    fn approve(&mut self, _: &str) -> Result<bool> {
        Ok(false)
    }
    fn approve_control(&mut self, peer: &str, _remembered: bool) -> Result<ControlApproval> {
        let permissions = self.app.refresh_permissions()?;
        if !self.app.inner.lock().unwrap().receiving_enabled
            || !permissions.can_receive()
        {
            return Ok(ControlApproval::Deny);
        }
        // Pairing authorizes control. Still require an authenticated, paired peer,
        // receiving enabled, OS permissions, and a live session.
        if self.app.store()?.peer(peer)?.is_none()
            || self.app.cancelled(self.job, &self.cancelled)
        {
            return Ok(ControlApproval::Deny);
        }
        self.app.stage(self.job, Phase::Connecting, Some(peer));
        self.inner.start_approved_control()?;
        Ok(ControlApproval::Remember)
    }
    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.inner.move_to(x, y)
    }
    fn input(&mut self, e: &extend_computer_agent::input::InputEvent) -> Result<()> {
        self.inner.input(e)
    }
    fn heartbeat(&mut self) -> Result<()> {
        self.inner.heartbeat()
    }
    fn display_size(&self) -> Result<DisplaySize> {
        let size = self.inner.display_size()?;
        self.app.stage(self.job, Phase::Connected, None);
        Ok(size)
    }
    fn finish(&mut self) {
        self.inner.finish();
    }
}
