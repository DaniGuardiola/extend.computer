//! Incoming listener and platform readiness checks for native input.
use super::*;
use extend_computer_agent::{
    cursor::NativeSink,
    low_jitter::ManagedCursor,
    session::{self, CursorSink, Decision, DisplaySize},
};
#[cfg(test)]
use std::net::TcpListener;

#[derive(Debug)]
pub(super) struct ReceivingPermissionRequired;
impl std::fmt::Display for ReceivingPermissionRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "Allow Accessibility and Wi-Fi optimization in Settings before allowing connections.",
        )
    }
}
impl std::error::Error for ReceivingPermissionRequired {}

impl Desktop {
    pub(super) fn serve_usb_control(
        self: &Arc<Self>,
        socket: TcpStream,
        route: &usb::Route,
    ) -> Result<()> {
        self.serve_pinned_control(socket, &route.peer, || route.current(self))
    }
    fn serve_pinned_control(
        self: &Arc<Self>,
        socket: TcpStream,
        peer: &str,
        current: impl Fn() -> bool,
    ) -> Result<()> {
        ensure!(current(), "USB connection was cancelled");
        ensure!(
            self.inner.lock().unwrap().receiving_enabled,
            "Incoming connections are disabled"
        );
        let (id, cancelled) = self.reserve(SessionKind::Incoming, Some(peer.to_owned()))?;
        let result = (|| {
            let _activity = display::session_activity();
            self.set_socket_route(id, &socket, Some("usb"))?;
            ensure!(current(), "USB connection was cancelled");
            let mut sink = GuiSink {
                inner: NativeSink::new(Some(self.helper.clone())),
                app: self.clone(),
                job: id,
                cancelled,
                usb: true,
                last_heartbeat: None,
            };
            session::serve_connection_with_cursor_pinned(
                socket,
                self.identity()?.as_ref(),
                &self.store()?,
                peer,
                &mut sink,
            )
        })();
        self.finish(id, result);
        Ok(())
    }
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
            let permissions =
                self.refresh_permissions_for_route(self.usb_network_receiving_available())?;
            if !self.receiving_permissions(&permissions) {
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
        let mut listener = wired::Listeners::bind(port).context(
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
        inner.error = None;
        drop(inner);
        let app = self.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                {
                    let mut inner = app.inner.lock().unwrap();
                    if let Err(error) =
                        refresh_pairing(&mut inner, &app.local_name(), &app.discovery_id)
                    {
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
                            let _activity = display::session_activity();
                            // macOS can inherit O_NONBLOCK from the listener. The encrypted engine uses deadline-bounded blocking reads.
                            socket.set_nonblocking(false)?;
                            extend_computer_agent::local_network::bind_stream_to_local_interface(
                                &socket,
                            )?;
                            let cable = wired::socket_route(&socket) == Some("usb");
                            app.set_socket(id, &socket)?;
                            let code = app.inner.lock().unwrap().code.take();
                            let expires = code.as_ref().map(|c| c.expires);
                            let mut window = code.map(|c| c.window);
                            let sink = GuiSink {
                                inner: NativeSink::new(Some(app.helper.clone())),
                                app: app.clone(),
                                job: id,
                                cancelled: cancelled.clone(),
                                usb: cable,
                                last_heartbeat: None,
                            };
                            let mut sink = if cable {
                                ManagedCursor::new(sink, address.ip(), false)
                            } else {
                                ManagedCursor::required(sink, address.ip())
                            };
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
                        let mut inner = app.inner.lock().unwrap();
                        if inner
                            .listener
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &stop))
                        {
                            inner.receiving_paused(format!(
                                "Incoming connections paused. The receiving listener failed: {e}. Retry Allow connections."
                            ));
                        }
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
    usb: bool,
    last_heartbeat: Option<Instant>,
}
impl CursorSink for GuiSink {
    fn shortcut_policy(&self) -> Option<extend_computer_agent::input::ShortcutPolicy> {
        self.inner.shortcut_policy()
    }
    fn supports_text_input(&self) -> bool {
        self.inner.supports_text_input()
    }
    fn supports_modifier_sides(&self) -> bool {
        self.inner.supports_modifier_sides()
    }
    fn peer_link(&mut self, link: Option<extend_computer_agent::link::Link>) {
        self.app.set_peer_link(self.job, link);
    }
    fn device_name(&self) -> Option<String> {
        Some(self.app.local_name())
    }
    fn peer_name(&mut self, peer: &str, name: &str) -> Result<()> {
        self.app.update_peer_name(peer, name)
    }

    fn unpaired(&mut self, peer: &str) -> Result<()> {
        self.app.received_unpair(peer)
    }
    fn start_cursor(&mut self, _: &str) -> Result<bool> {
        Ok(false)
    }
    fn start_control(&mut self, peer: &str) -> Result<bool> {
        let permissions = self.app.refresh_permissions_for_route(self.usb)?;
        if !self.app.inner.lock().unwrap().receiving_enabled
            || !(if self.usb {
                permissions.available && permissions.post
            } else {
                permissions.can_receive()
            })
        {
            return Ok(false);
        }
        if self.app.store()?.peer(peer)?.is_none() || self.app.cancelled(self.job, &self.cancelled)
        {
            return Ok(false);
        }
        self.app.stage(self.job, Phase::Connecting, Some(peer));
        self.inner.start_control(peer)
    }
    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.inner.move_to(x, y)
    }
    fn input(&mut self, e: &extend_computer_agent::input::InputEvent) -> Result<()> {
        self.inner.input(e)
    }
    fn heartbeat(&mut self) -> Result<()> {
        // The sharing device pings at least every 500 ms, so longer gaps
        // mean its input is arriving late.
        let now = Instant::now();
        if let Some(last) = self.last_heartbeat.replace(now) {
            self.app
                .record_quality(self.job, quality::Sample::Gap(now - last));
        }
        self.inner.heartbeat()
    }
    fn ending(&self) -> bool {
        self.app.ended_locally(self.job)
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

#[cfg(all(test, unix))]
mod usb_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn pinned_usb_control_uses_native_ready_without_wifi_or_lan_metadata() {
        for wrong_peer in [false, true] {
            let files = tempfile::tempdir().unwrap();
            let helper = files.path().join("helper");
            std::fs::write(&helper, r#"#!/bin/sh
case "$1" in
status) echo 'listen=true post=true wifi=false';;
inject-control) echo 'READY 100 100 input-text-v1'; while IFS= read -r line; do echo "$line" >> "$(dirname "$0")/events"; echo OK; done;;
*) exit 1;;
esac
"#).unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            let app = Desktop::new(files.path().join("receiver"), helper).unwrap();
            app.set_test_identity(Identity::generate());
            let expected = Identity::generate();
            let other = Identity::generate();
            let identity = app.identity().unwrap();
            app.store()
                .unwrap()
                .remember(&expected.fingerprint())
                .unwrap();
            app.store().unwrap().remember(&other.fingerprint()).unwrap();
            app.inner.lock().unwrap().receiving_enabled = true;
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let accepted = listener.accept().unwrap().0;
            let receiver = app.clone();
            let peer = expected.fingerprint();
            let worker =
                std::thread::spawn(move || receiver.serve_pinned_control(accepted, &peer, || true));
            let sender = if wrong_peer { &other } else { &expected };
            let trust = TrustStore::open(files.path().join("sender-trust")).unwrap();
            trust.remember(&identity.fingerprint()).unwrap();
            let client = extend_computer_agent::session::Client::connect(
                socket,
                sender,
                &trust,
                None,
                Some(&identity.fingerprint()),
                |_, _| Decision::Once,
            );
            if wrong_peer {
                assert!(client.is_err());
            } else {
                let mut client = client.unwrap();
                assert_eq!(client.request_control().unwrap().width, 100.);
                assert!(client.supports_text_input());
                client
                    .queue_input(extend_computer_agent::input::InputEvent::Text {
                        value: "Café 👩‍💻".into(),
                    })
                    .unwrap();
                client.probe().unwrap();
                drop(client); // EOF exercises native helper teardown after actual READY.
            }
            worker.join().unwrap().unwrap();
            assert!(app.inner.lock().unwrap().job.is_none());
            assert!(app.devices.lock().unwrap().is_empty());
            assert!(app
                .store()
                .unwrap()
                .peer(&expected.fingerprint())
                .unwrap()
                .is_some());
            assert!(app
                .store()
                .unwrap()
                .peer(&other.fingerprint())
                .unwrap()
                .is_some());
            if wrong_peer {
                assert!(!files.path().join("events").exists());
            } else {
                assert!(app.inner.lock().unwrap().permissions.post);
                assert!(!app.inner.lock().unwrap().permissions.wifi);
                assert!(std::fs::read_to_string(files.path().join("events"))
                    .unwrap()
                    .contains("Café 👩‍💻"));
            }
        }
    }
}
