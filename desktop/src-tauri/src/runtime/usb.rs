//! Native accessory selection. Private streams retain end-to-end pinned authentication.
use super::*;
use extend_computer_agent::{display::Transport, session::display::VideoCodec};

#[cfg(target_os = "macos")]
pub(super) struct Connection {
    peer: String,
    host: extend_computer_agent::display::usb_host::NativeUsbHost,
}
#[derive(Clone)]
pub(super) struct Route {
    pub(super) peer: String,
    epoch: u64,
    #[cfg(target_os = "macos")]
    hub: Arc<extend_computer_agent::display::usb_link::UsbHub>,
}
impl Route {
    pub(super) fn current(&self, app: &Desktop) -> bool {
        if app.closing.load(Ordering::Acquire)
            || app.usb_epoch.load(Ordering::Acquire) != self.epoch
            || !app
                .store()
                .and_then(|store| store.peer(&self.peer))
                .is_ok_and(|p| p.is_some())
        {
            return false;
        }
        #[cfg(target_os = "macos")]
        return app.usb.lock().unwrap().as_ref().is_some_and(|c| {
            c.peer == self.peer
                && c.host.snapshot()["state"] == "connected"
                && Arc::ptr_eq(&c.host.hub(), &self.hub)
        });
        #[cfg(not(target_os = "macos"))]
        false
    }
    pub(super) fn transport(&self, app: &Desktop, codec: VideoCodec) -> Result<Transport> {
        ensure!(self.current(app), "USB connection was cancelled");
        #[cfg(target_os = "macos")]
        {
            Transport::with_packet_link(codec, Box::new(self.hub.endpoint()?))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = codec;
            anyhow::bail!("Native USB requires macOS")
        }
    }
    pub(super) fn open(
        &self,
        app: &Desktop,
        service: extend_computer_agent::display::usb_stream::UsbService,
        cancelled: &AtomicBool,
    ) -> Result<TcpStream> {
        ensure!(
            self.current(app) && !cancelled.load(Ordering::Acquire),
            "USB connection was cancelled"
        );
        #[cfg(target_os = "macos")]
        {
            let stream = self.hub.open_stream(
                service,
                cancelled,
                Instant::now() + Duration::from_secs(3),
            )?;
            if !self.current(app) || cancelled.load(Ordering::Acquire) {
                let _ = stream.shutdown(Shutdown::Both);
                anyhow::bail!("USB connection was cancelled");
            }
            Ok(stream)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (service, cancelled);
            anyhow::bail!("Native USB requires macOS")
        }
    }
}
struct Connecting<'a>(&'a AtomicBool);
impl Drop for Connecting<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
#[derive(serde::Deserialize, serde::Serialize)]
struct UsbDevice {
    vendor: String,
    serial: String,
    name: String,
    accessory: bool,
}
fn inventory_result(bytes: &[u8]) -> Result<serde_json::Value> {
    ensure!(bytes.len() <= 16_384, "USB discovery output is too large");
    let devices: Vec<UsbDevice> = serde_json::from_slice(bytes)?;
    ensure!(devices.len() <= 16, "Too many USB devices");
    for device in &devices {
        ensure!(
            device.vendor.len() == 4 && device.vendor.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid USB vendor"
        );
        ensure!(
            !device.serial.is_empty()
                && device.serial.len() <= 127
                && device
                    .serial
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "Invalid USB serial"
        );
        ensure!(
            !device.name.is_empty()
                && device.name.len() <= 512
                && !device.name.chars().any(char::is_control),
            "Invalid USB device name"
        );
    }
    Ok(serde_json::to_value(devices)?)
}
impl Desktop {
    pub(super) fn receiving_permissions(&self, permissions: &Permissions) -> bool {
        permissions.can_receive()
            || (permissions.available
                && permissions.post
                && (self.usb_receiving_available() || self.usb_network_receiving_available()))
    }
    pub(super) fn usb_network_receiving_available(&self) -> bool {
        // Unit-test desktops describe their own attached hardware. A developer's
        // connected cable must not change unrelated permission fixtures.
        #[cfg(test)]
        return self.test_usb_network.load(Ordering::Acquire);
        #[cfg(not(test))]
        return !wired::interfaces().is_empty();
    }
    fn usb_receiving_available(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self
            .usb
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|c| c.host.snapshot()["state"] == "connected");
        #[cfg(not(target_os = "macos"))]
        false
    }
    pub(super) fn selected_usb_route(&self, peer: &str) -> Option<Route> {
        #[cfg(target_os = "macos")]
        return self
            .usb
            .lock()
            .unwrap()
            .as_ref()
            .filter(|c| c.peer == peer)
            .map(|c| Route {
                peer: peer.into(),
                epoch: self.usb_epoch.load(Ordering::Acquire),
                hub: c.host.hub(),
            });
        #[cfg(not(target_os = "macos"))]
        {
            let _ = peer;
            None
        }
    }
    #[cfg(target_os = "macos")]
    fn start_usb_streams(
        self: &Arc<Self>,
        peer: String,
        epoch: u64,
        hub: Arc<extend_computer_agent::display::usb_link::UsbHub>,
    ) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            let route = Route {
                peer: peer.clone(),
                epoch,
                hub: hub.clone(),
            };
            loop {
                let Some(app) = weak.upgrade() else { break };
                if !route.current(&app) {
                    break;
                }
                match hub.accept_stream() {
                    Ok(Some((service, socket))) => {
                        let route = route.clone();
                        std::thread::spawn(move || {
                            if !route.current(&app) {
                                let _ = socket.shutdown(Shutdown::Both);
                                return;
                            }
                            let result = match service {
                                extend_computer_agent::display::usb_stream::UsbService::Presence =>
                                    app.serve_usb_presence(socket, &route),
                                extend_computer_agent::display::usb_stream::UsbService::Control =>
                                    app.serve_usb_control(socket, &route),
                                extend_computer_agent::display::usb_stream::UsbService::Display =>
                                    app.serve_usb_display(socket, &route),
                            };
                            if let Err(error) = result {
                                eprintln!("USB signaling ended: {error:#}");
                            }
                        });
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(_) => break,
                }
            }
            hub.close();
        });
    }
    pub(super) fn serve_usb_presence(&self, socket: TcpStream, route: &Route) -> Result<()> {
        ensure!(route.current(self), "USB connection was cancelled");
        extend_computer_agent::session::serve_presence_pinned(
            socket,
            self.identity()?.as_ref(),
            &self.store()?,
            &route.peer,
            || {
                let current = route.current(self);
                let inner = self.inner.lock().unwrap();
                extend_computer_agent::session::Presence {
                    receiving: current
                        && inner.receiving_enabled
                        && inner.permissions.available
                        && inner.permissions.post,
                    port: 48177,
                    name: self.local_name(),
                }
            },
        )
    }
    pub(super) fn resolve_usb_presence(
        &self,
        peer: &str,
        identity: &Identity,
        route: &Route,
    ) -> Result<bool> {
        let cancelled = AtomicBool::new(false);
        let epoch = self.inner.lock().unwrap().next;
        let result = route
            .open(
                self,
                extend_computer_agent::display::usb_stream::UsbService::Presence,
                &cancelled,
            )
            .and_then(|socket| {
                extend_computer_agent::session::query_presence_with_device_info(
                    socket, identity, peer,
                )
            });
        match result {
            Err(error) => {
                if !route.current(self) {
                    return Err(error);
                }
                let mut health = self.health.lock().unwrap();
                let state = health.entry(peer.into()).or_default();
                if error.downcast_ref::<extend_computer_agent::error::EngineError>()
                    == Some(&extend_computer_agent::error::EngineError::ProtocolIncompatible)
                {
                    state.incompatible();
                } else {
                    state.failure();
                }
                Err(error)
            }
            Ok(Some(reply)) if route.current(self) => {
                let status = reply.status;
                self.update_peer_name(peer, &status.name)?;
                if let Some(info) = reply.device_info {
                    let _ = self.update_peer_device_info(peer, info);
                }
                let returned = self
                    .health
                    .lock()
                    .unwrap()
                    .entry(peer.into())
                    .or_default()
                    .usb_success(status.receiving);
                if returned {
                    self.peer_returned(peer);
                }
                Ok(true)
            }
            Ok(None) if route.current(self) => {
                self.apply_peer_unpaired(peer, epoch)?;
                Ok(false)
            }
            _ => anyhow::bail!("USB connection was cancelled"),
        }
    }
    #[cfg(target_os = "macos")]
    fn usb_helper(&self) -> Result<PathBuf> {
        let helper = self
            .helper
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .context("Missing native helper directory")?
            .join("ExtendComputerUsb/ExtendComputerUsb");
        ensure!(helper.is_file(), "Native USB helper isn't installed");
        Ok(helper)
    }
    pub fn usb_devices(&self) -> Result<serde_json::Value> {
        #[cfg(not(target_os = "macos"))]
        anyhow::bail!("Native USB host currently requires macOS");
        #[cfg(target_os = "macos")]
        {
            ensure!(
                !self.closing.load(Ordering::Acquire),
                "Application is closing"
            );
            // File-backed output avoids waiting on an unread pipe. A bounded
            // helper deadline also covers slow OS USB permission responses.
            let output = tempfile::NamedTempFile::new()?;
            let mut child = std::process::Command::new(self.usb_helper()?)
                .arg("--list")
                .stdin(std::process::Stdio::null())
                .stdout(output.reopen()?)
                .stderr(std::process::Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(8);
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None)
                        if Instant::now() < deadline && !self.closing.load(Ordering::Acquire) =>
                    {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    result => {
                        let _ = child.kill();
                        let _ = child.wait();
                        if let Err(error) = result {
                            return Err(error.into());
                        }
                        anyhow::bail!("USB discovery timed out. Check the cable and allow this accessory on your Mac.");
                    }
                }
            };
            ensure!(status.success(), "Could not check connected USB devices");
            ensure!(
                output.as_file().metadata()?.len() <= 16_384,
                "USB discovery output is too large"
            );
            inventory_result(&std::fs::read(output.path())?)
        }
    }

    #[cfg(any(debug_assertions, feature = "display-benchmark"))]
    pub fn start_usb_media_test(
        self: &Arc<Self>,
        peer: String,
        vendor: String,
        serial: String,
        receipt: PathBuf,
        seconds: u64,
    ) {
        let app = self.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                ensure!((30..=600).contains(&seconds), "Invalid USB test duration");
                app.connect_usb(&peer, &vendor, &serial)?;
                let deadline = Instant::now() + Duration::from_secs(seconds);
                while !app.closing.load(Ordering::Acquire) && Instant::now() < deadline {
                    let status = app.usb_status();
                    let directory = receipt.parent().context("Missing receipt directory")?;
                    let file = tempfile::NamedTempFile::new_in(directory)?;
                    std::fs::write(
                        file.path(),
                        serde_json::to_vec(
                            &serde_json::json!({"status":"ready","peer":peer,"local_identity":app.identity()?.fingerprint(),"usb":status}),
                        )?,
                    )?;
                    file.persist(&receipt)?;
                    if status["state"] != "connected" {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("USB media test failed: {error:#}");
                let _ = std::fs::write(
                    &receipt,
                    serde_json::json!({"status":"failed","error":format!("{error:#}")}).to_string(),
                );
            }
            app.disconnect_usb();
        });
    }
    pub fn connect_usb(self: &Arc<Self>, peer: &str, vendor: &str, serial: &str) -> Result<()> {
        ensure!(
            peer.len() == 64
                && peer
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid device identity"
        );
        ensure!(
            !vendor.is_empty()
                && vendor.len() <= 4
                && vendor.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid USB vendor"
        );
        ensure!(
            !serial.is_empty()
                && serial.len() <= 127
                && serial
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "Invalid USB serial"
        );
        ensure!(
            self.store()?.peer(peer)?.is_some(),
            "Pair this device or sign in first"
        );
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "Application is closing"
        );
        let epoch = self.usb_epoch.load(Ordering::Acquire);
        ensure!(
            self.usb_connecting
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "USB connection is already starting"
        );
        let _connecting = Connecting(&self.usb_connecting);
        ensure!(
            self.inner.lock().unwrap().job.is_none(),
            "Disconnect the current session before connecting USB"
        );
        #[cfg(not(target_os = "macos"))]
        let _ = epoch;
        #[cfg(not(target_os = "macos"))]
        anyhow::bail!("Native USB host currently requires macOS");
        #[cfg(target_os = "macos")]
        {
            ensure!(
                self.usb.lock().unwrap().is_none(),
                "Disconnect the current USB connection first"
            );
            let helper = self.usb_helper()?;
            let identity = self.identity()?;
            let host = extend_computer_agent::display::usb_host::NativeUsbHost::connect(
                &helper,
                vendor,
                serial,
                &identity.fingerprint(),
            )?;
            // The cable switch can take seconds. Never install a late connection
            // after shutdown or after its authenticated peer was removed.
            let hub = host.hub();
            hub.enable_streams()?;
            let mut connection = self.usb.lock().unwrap();
            ensure!(
                !self.closing.load(Ordering::Acquire)
                    && self.usb_epoch.load(Ordering::Acquire) == epoch
                    && self.store()?.peer(peer)?.is_some()
                    && self.inner.lock().unwrap().job.is_none(),
                "USB connection was cancelled"
            );
            self.health.lock().unwrap().remove(peer);
            *connection = Some(Connection {
                peer: peer.into(),
                host,
            });
            drop(connection);
            self.start_usb_streams(peer.to_owned(), epoch, hub.clone());
            let route = Route {
                peer: peer.into(),
                epoch,
                hub,
            };
            let verified = self.resolve_usb_presence(peer, &identity, &route);
            match verified {
                Ok(true) if route.current(self) => {
                    // A readiness pause does not revoke the user's saved preference.
                    // Recover only after authenticating this carrier; explicit Off stays Off.
                    let mut preference = self.receiving_preference.lock().unwrap();
                    if *preference {
                        if let Err(error) =
                            self.receive_with_preference(false, 48177, &mut preference)
                        {
                            eprintln!("USB receiving remains paused: {error:#}");
                        }
                    }
                    Ok(())
                }
                result => {
                    if self.usb_epoch.load(Ordering::Acquire) == epoch {
                        self.disconnect_usb_peer(peer);
                    }
                    match result {
                        Err(error) => Err(error),
                        _ => anyhow::bail!("USB device identity could not be verified"),
                    }
                }
            }
        }
    }
    pub fn disconnect_usb(&self) {
        self.usb_epoch.fetch_add(1, Ordering::AcqRel);
        #[cfg(target_os = "macos")]
        {
            // Drop outside the mutex: shutting down the worker joins its thread.
            let connection = self.usb.lock().unwrap().take();
            if let Some(connection) = &connection {
                self.health.lock().unwrap().remove(&connection.peer);
            }
            drop(connection);
        }
    }
    pub(super) fn disconnect_usb_peer(&self, peer: &str) {
        #[cfg(target_os = "macos")]
        {
            let mut current = self.usb.lock().unwrap();
            if current
                .as_ref()
                .is_some_and(|connection| connection.peer == peer)
            {
                self.usb_epoch.fetch_add(1, Ordering::AcqRel);
                let removed = current.take();
                drop(current);
                drop(removed);
            }
        }
        let _ = peer;
    }
    pub fn usb_status(&self) -> serde_json::Value {
        if self.usb_connecting.load(Ordering::Acquire) {
            return serde_json::json!({"state":"connecting","signaling_route":"usb"});
        }
        #[cfg(target_os = "macos")]
        if let Some(connection) = self.usb.lock().unwrap().as_ref() {
            let mut value = connection.host.snapshot();
            extend_computer_agent::display::metrics::usb_queue_snapshot(&value["queue_metrics"]);
            value["peer"] = connection.peer.clone().into();
            value["signaling_route"] = "usb".into();
            return value;
        }
        serde_json::json!({"state":"disconnected"})
    }
    pub(super) fn set_display_route(&self, id: u64, route: &'static str) {
        if let Some(job) = self
            .inner
            .lock()
            .unwrap()
            .job
            .as_mut()
            .filter(|job| job.view.id == id)
        {
            job.view.route = Some(route);
            eprintln!("Display media transport: {route}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_network_receiving_requires_native_input_permissions_without_wifi() {
        let files = tempfile::tempdir().unwrap();
        let app =
            Desktop::new(files.path().join("desktop"), PathBuf::from("unused-helper")).unwrap();
        let mut permissions = Permissions {
            available: true,
            post: true,
            ..Default::default()
        };
        assert!(!app.receiving_permissions(&permissions));
        app.test_usb_network.store(true, Ordering::Release);
        assert!(app.receiving_permissions(&permissions));
        permissions.post = false;
        assert!(!app.receiving_permissions(&permissions));
        permissions.post = true;
        permissions.available = false;
        assert!(!app.receiving_permissions(&permissions));
        app.test_usb_network.store(false, Ordering::Release);
        permissions.available = true;
        assert!(!app.receiving_permissions(&permissions));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn usb_presence_precedes_invalid_lan_and_detach_invalidates_authenticated_health() {
        use extend_computer_agent::display::{
            usb_host::NativeUsbHost,
            usb_link::{run_io, UsbHub},
            usb_stream::UsbService,
        };
        use std::{net::TcpListener, os::unix::fs::PermissionsExt};
        let files = tempfile::tempdir().unwrap();
        let raw = TcpListener::bind("127.0.0.1:0").unwrap();
        let helper = files.path().join("usb-fixture");
        // Test-only carrier replacement: forwards opaque bytes to the other local hub.
        // No network peer addresses enter application metadata or protocol policy.
        std::fs::write(
            &helper,
            format!(
                r#"#!/usr/bin/python3
import socket,select,sys
left=socket.socket(socket.AF_UNIX); left.connect(sys.argv[4])
right=socket.create_connection(('127.0.0.1',{}))
while True:
 ready,_,_=select.select([left,right],[],[],1)
 for src in ready:
  data=src.recv(65536)
  if not data: sys.exit(0)
  (right if src is left else left).sendall(data)
"#,
                raw.local_addr().unwrap().port()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let app = Desktop::new(
            files.path().join("desktop"),
            files.path().join("missing-input-helper"),
        )
        .unwrap();
        app.set_test_identity(Identity::generate());
        let identity = app.identity().unwrap();
        let remote = Identity::generate();
        let peer = remote.fingerprint();
        app.store().unwrap().remember(&peer).unwrap();
        let store = TrustStore::open(files.path().join("remote-trust")).unwrap();
        store.remember(&identity.fingerprint()).unwrap();
        let (hub, driver) =
            UsbHub::new("192.0.2.2".parse().unwrap(), "192.0.2.1".parse().unwrap()).unwrap();
        hub.enable_streams().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let end = stopped.clone();
        let carrier = std::thread::spawn(move || {
            let stream = raw.accept().unwrap().0;
            stream.set_nonblocking(true).unwrap();
            run_io(driver, stream.try_clone().unwrap(), stream, &end)
        });
        let host =
            NativeUsbHost::connect(&helper, "0000", "local-fixture", &identity.fingerprint())
                .unwrap();
        host.hub().enable_streams().unwrap();
        *app.usb.lock().unwrap() = Some(Connection {
            peer: peer.clone(),
            host,
        });
        let route = app.selected_usb_route(&peer).unwrap();
        let remote_hub = hub.clone();
        let pin = identity.fingerprint();
        let server = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(5);
            let (service, socket) = loop {
                if let Some(stream) = remote_hub.accept_stream().unwrap() {
                    break stream;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(5));
            };
            assert_eq!(service, UsbService::Presence);
            extend_computer_agent::session::serve_presence_pinned(
                socket,
                &remote,
                &store,
                &pin,
                || extend_computer_agent::session::Presence {
                    receiving: false,
                    port: 48177,
                    name: "Test tablet".into(),
                },
            )
            .unwrap();
        });
        let broken_lan = Device {
            name: "Tablet".into(),
            address: "not-an-address".into(),
            edge: "left".into(),
        };
        assert!(app
            .resolve_presence(&peer, &broken_lan, &identity, &[])
            .unwrap());
        server.join().unwrap();
        let snapshot = app.snapshot().unwrap();
        assert!(matches!(
            snapshot.peers[0].availability,
            presence::Availability::ReceivingOff
        ));
        assert_eq!(snapshot.peers[0].route, Some("usb"));
        assert_eq!(
            snapshot.peers[0].device_info,
            Some(extend_computer_agent::device_info::local_info())
        );
        assert!(app.devices.lock().unwrap().is_empty());
        app.disconnect_usb();
        assert!(!route.current(&app));
        assert!(route
            .open(&app, UsbService::Control, &AtomicBool::new(false))
            .is_err());
        assert_eq!(app.snapshot().unwrap().peers[0].route, None);
        assert!(!matches!(
            app.snapshot().unwrap().peers[0].availability,
            presence::Availability::Online
        ));
        stopped.store(true, Ordering::Release);
        hub.close();
        let _ = carrier.join().unwrap();
    }
    #[test]
    fn inventory_requires_bounded_typed_exact_device_descriptors() {
        let item = serde_json::json!({"vendor":"2717","serial":"tablet-1","name":"Tablet","accessory":false});
        assert_eq!(
            inventory_result(
                serde_json::to_string(&vec![item.clone()])
                    .unwrap()
                    .as_bytes()
            )
            .unwrap()[0]["serial"],
            "tablet-1"
        );
        assert!(inventory_result(b"{}").is_err());
        assert!(inventory_result(b"[{}]").is_err());
        let mut malformed = item.clone();
        malformed["serial"] = "bad/serial".into();
        assert!(
            inventory_result(serde_json::to_string(&vec![malformed]).unwrap().as_bytes()).is_err()
        );
        assert!(
            inventory_result(serde_json::to_string(&vec![item; 17]).unwrap().as_bytes()).is_err()
        );
        assert!(inventory_result(&vec![b' '; 16_385]).is_err());
    }
    #[test]
    fn unknown_peer_never_starts_usb_helper_or_requests_identity() {
        let files = tempfile::tempdir().unwrap();
        let desktop = Desktop::new(
            files.path().join("state"),
            files.path().join("missing-helper"),
        )
        .unwrap();
        let unknown = Identity::generate().fingerprint();
        assert!(desktop.store().unwrap().peer(&unknown).unwrap().is_none());
        let result = desktop.connect_usb(&unknown, "2717", "test-tablet");
        assert!(result.unwrap_err().to_string().contains("Pair this device"));
        assert!(desktop.identity.lock().unwrap().is_none());
        assert!(!desktop.usb_connecting.load(Ordering::Acquire));
        assert_eq!(desktop.usb_status()["state"], "disconnected");
    }
}
