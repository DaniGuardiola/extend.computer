//! Anonymous discovery suggests endpoints; pinned encrypted replies establish presence.
use super::*;
use extend_computer_agent::{
    discovery::Advertisement,
    error::EngineError,
    session::{query_presence_with_device_info, serve_presence, Presence},
};
#[cfg(test)]
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Checking,
    Online,
    ReceivingOff,
    Offline,
    UpdateRequired,
}
#[derive(Default)]
pub(super) struct Health {
    last: Option<(Instant, bool)>,
    failures: u8,
    incompatible: bool,
    route: Option<&'static str>,
}
impl Health {
    pub(super) fn success(&mut self, receiving: bool) {
        self.route = None;
        self.last = Some((Instant::now(), receiving));
        self.failures = 0;
        self.incompatible = false;
    }
    pub(super) fn usb_success(&mut self, receiving: bool) {
        self.success(receiving);
        self.route = Some("usb");
    }
    pub(super) fn route(&self) -> Option<&'static str> {
        matches!(
            self.state(),
            Availability::Online | Availability::ReceivingOff
        )
        .then_some(self.route)
        .flatten()
    }
    pub(super) fn incompatible(&mut self) {
        self.last = Some((Instant::now(), false));
        self.failures = 0;
        self.incompatible = true;
    }
    pub(super) fn failure(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }
    pub fn state(&self) -> Availability {
        if let Some((at, receiving)) = self.last {
            if at.elapsed() < Duration::from_secs(25) && self.failures < 3 {
                if self.incompatible {
                    return Availability::UpdateRequired;
                }
                return if receiving {
                    Availability::Online
                } else {
                    Availability::ReceivingOff
                };
            }
        }
        if self.last.is_none() && self.failures < 3 {
            Availability::Checking
        } else {
            Availability::Offline
        }
    }
}
impl Desktop {
    pub fn start_presence(self: &Arc<Self>) -> Result<()> {
        let mut listener = wired::Listeners::bind(48178).or_else(|_| wired::Listeners::bind(0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        *self.presence_port.lock().unwrap() = port;
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            let mut advertisement = None;
            let mut advertised_at = Instant::now();
            let mut next_ad_check = Instant::now();
            let workers = Arc::new(AtomicUsize::new(0));
            loop {
                let Some(app) = weak.upgrade() else { break };
                if app.closing.load(Ordering::SeqCst) {
                    break;
                }
                if Instant::now() >= next_ad_check {
                    next_ad_check = Instant::now() + Duration::from_secs(5);
                    if app.inner.lock().unwrap().receiving_enabled {
                        let _ = app.refresh_background_permissions();
                    }
                    let paired = app
                        .store()
                        .and_then(|s| s.load())
                        .is_ok_and(|r| !r.peers.is_empty());
                    if !paired {
                        advertisement = None;
                    } else if advertisement.is_none()
                        || advertised_at.elapsed() > Duration::from_secs(300)
                    {
                        advertisement = Advertisement::start_presence(port).ok();
                        advertised_at = Instant::now();
                    }
                }
                if let Ok((socket, _)) = listener.accept() {
                    if workers.load(Ordering::SeqCst) < 4 {
                        if let Some(identity) = app.identity.lock().unwrap().clone() {
                            let app = app.clone();
                            let workers = workers.clone();
                            workers.fetch_add(1, Ordering::SeqCst);
                            std::thread::spawn(move || {
                                let _ = socket.set_nonblocking(false);
                                if extend_computer_agent::local_network::bind_stream_to_local_interface(
                                    &socket,
                                ).is_err() {
                                    workers.fetch_sub(1, Ordering::SeqCst);
                                    return;
                                }
                                let cable = wired::socket_route(&socket) == Some("usb");
                                if let Ok(store) = app.store() {
                                    let _ = serve_presence(socket, &identity, &store, || {
                                        let inner = app.inner.lock().unwrap();
                                        Presence {
                                            receiving: inner.receiving_enabled
                                                && (inner.permissions.can_receive()
                                                    || (cable
                                                        && inner.permissions.available
                                                        && inner.permissions.post))
                                                && inner.listener.as_ref().is_some_and(|stop| {
                                                    !stop.load(Ordering::SeqCst)
                                                }),
                                            port: if inner.listener_port == 0 {
                                                48177
                                            } else {
                                                inner.listener_port
                                            },
                                            name: app.local_name(),
                                        }
                                    });
                                }
                                workers.fetch_sub(1, Ordering::SeqCst);
                            });
                        }
                    }
                }
                drop(app);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Ok(())
    }
    pub(super) fn resolve_presence(
        &self,
        peer: &str,
        device: &Device,
        identity: &Identity,
        candidates: &[SocketAddr],
    ) -> Result<bool> {
        if let Some(route) = self.selected_usb_route(peer) {
            return self.resolve_usb_presence(peer, identity, &route);
        }
        let mut endpoints = Vec::new();
        if let Some(address) = self.presence_addresses.lock().unwrap().get(peer).copied() {
            endpoints.push(address);
        }
        if let Ok(address) = device.address.parse::<SocketAddr>() {
            endpoints.push(SocketAddr::new(address.ip(), 48178));
        }
        endpoints.extend_from_slice(candidates);
        if self.resolve_local_presence(peer, identity, endpoints)? {
            return Ok(true);
        }
        self.resolve_relay_presence(peer, identity)
    }

    pub(super) fn refresh_local_route(
        &self,
        peer: &str,
        device: &Device,
        identity: &Identity,
    ) -> Result<bool> {
        if let Some(route) = self.selected_usb_route(peer) {
            return self.resolve_usb_presence(peer, identity, &route);
        }
        let mut endpoints = Vec::new();
        if let Some(address) = self.presence_addresses.lock().unwrap().get(peer).copied() {
            endpoints.push(address);
        }
        if let Ok(address) = device.address.parse::<SocketAddr>() {
            endpoints.push(SocketAddr::new(address.ip(), 48178));
        }
        endpoints.extend(self.discovered_presence.lock().unwrap().iter().copied());
        self.resolve_local_presence(peer, identity, endpoints)
    }

    /// Local checks never contact the account backend. Used before every connection attempt.
    pub(super) fn resolve_local_presence(
        &self,
        peer: &str,
        identity: &Identity,
        endpoints: Vec<SocketAddr>,
    ) -> Result<bool> {
        let epoch = self.inner.lock().unwrap().next;
        let result = routes::race(
            endpoints,
            || {
                let inner = self.inner.lock().unwrap();
                self.closing.load(Ordering::SeqCst)
                    || inner.next != epoch
                    || inner
                        .job
                        .as_ref()
                        .is_some_and(|job| job.cancelled.load(Ordering::SeqCst))
            },
            |socket| query_presence_with_device_info(socket, identity, peer),
        );
        match result {
            Ok((address, Some((status, info)), route)) => {
                // Re-check trust and update just the endpoint, preserving simultaneous UI edits.
                if self.store()?.peer(peer)?.is_none() {
                    return Ok(false);
                }
                let _ = self.update_peer_name(peer, &status.name);
                if let Some(info) = info {
                    let _ = self.update_peer_device_info(peer, info);
                }
                let endpoint = SocketAddr::new(address.ip(), status.port).to_string();
                let mut devices = self.devices.lock().unwrap();
                if devices.get(peer).is_some_and(|d| d.address != endpoint) {
                    let mut next = devices.clone();
                    next.get_mut(peer).unwrap().address = endpoint;
                    peers::save(&self.root, &next)?;
                    *devices = next;
                }
                drop(devices);
                self.presence_addresses
                    .lock()
                    .unwrap()
                    .insert(peer.into(), address);
                let mut health = self.health.lock().unwrap();
                let health = health.entry(peer.into()).or_default();
                if route == Some("usb") {
                    health.usb_success(status.receiving);
                } else {
                    health.success(status.receiving);
                }
                return Ok(true);
            }
            Ok((_, None, _)) => {
                self.apply_peer_unpaired(peer, epoch)?;
                return Ok(false);
            }
            Err(error)
                if error.downcast_ref::<EngineError>()
                    == Some(&EngineError::ProtocolIncompatible) =>
            {
                // query_presence validates the pinned identity before
                // protocol negotiation. Discovery cannot set this state.
                self.health
                    .lock()
                    .unwrap()
                    .entry(peer.into())
                    .or_default()
                    .incompatible();
                return Ok(false);
            }
            Err(_) => {}
        }
        Ok(false)
    }

    fn resolve_relay_presence(&self, peer: &str, identity: &Identity) -> Result<bool> {
        if self.account_relay.online() && self.account_devices.lock().unwrap().contains_key(peer) {
            let result = self
                .account_tunnel(peer, "presence")
                .and_then(|socket| query_presence_with_device_info(socket, identity, peer));
            if result
                .as_ref()
                .err()
                .and_then(|e| e.downcast_ref::<EngineError>())
                == Some(&EngineError::ProtocolIncompatible)
            {
                self.health
                    .lock()
                    .unwrap()
                    .entry(peer.into())
                    .or_default()
                    .incompatible();
                return Ok(false);
            }
            if let Ok(Some((status, info))) = result {
                if self.store()?.peer(peer)?.is_some() {
                    if let Some(info) = info {
                        let _ = self.update_peer_device_info(peer, info);
                    }
                    self.health
                        .lock()
                        .unwrap()
                        .entry(peer.into())
                        .or_default()
                        .success(status.receiving);
                    return Ok(true);
                }
            }
        }
        self.health
            .lock()
            .unwrap()
            .entry(peer.into())
            .or_default()
            .failure();
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_route_requires_fresh_authenticated_presence_not_lan_or_failed_checks() {
        let mut health = Health::default();
        assert_eq!(health.route(), None);
        health.success(true);
        assert_eq!(health.route(), None);
        health.usb_success(false);
        assert_eq!(health.state(), Availability::ReceivingOff);
        assert_eq!(health.route(), Some("usb"));
        health.usb_success(true);
        health.failure();
        health.failure();
        assert_eq!(health.route(), Some("usb"));
        health.failure();
        assert_eq!(health.route(), None);
        health.usb_success(true);
        health.last = Some((Instant::now() - Duration::from_secs(26), true));
        assert_eq!(health.route(), None);
        health.usb_success(true);
        health.incompatible();
        assert_eq!(health.route(), None);
    }
    #[test]
    fn incompatible_peer_requires_update_until_recovery_or_expiry() {
        let mut h = Health::default();
        h.incompatible();
        assert_eq!(h.state(), Availability::UpdateRequired);
        h.failure();
        assert_eq!(h.state(), Availability::UpdateRequired);
        h.success(true);
        assert_eq!(h.state(), Availability::Online);
        h.incompatible();
        h.last = Some((Instant::now() - Duration::from_secs(26), false));
        assert_eq!(h.state(), Availability::Offline);
    }
    #[test]
    fn status_requires_verification_and_tolerates_transient_failures() {
        let mut h = Health::default();
        assert_eq!(h.state(), Availability::Checking);
        h.failure();
        h.failure();
        assert_eq!(h.state(), Availability::Checking);
        h.failure();
        assert_eq!(h.state(), Availability::Offline);
        h.success(true);
        h.failure();
        h.failure();
        assert_eq!(h.state(), Availability::Online);
        h.failure();
        assert_eq!(h.state(), Availability::Offline);
        h.success(false);
        assert_eq!(h.state(), Availability::ReceivingOff);
        h.last = Some((Instant::now() - Duration::from_secs(26), true));
        assert_eq!(h.state(), Availability::Offline);
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    fn app(root: &std::path::Path) -> Arc<Desktop> {
        let app = Desktop::new(root.into(), PathBuf::from("unused-helper")).unwrap();
        *app.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
        app
    }
    #[test]
    fn snapshot_keeps_authenticated_cable_route_without_android_usb_selection() {
        let files = tempfile::tempdir().unwrap();
        let caller = app(&files.path().join("caller"));
        let peer = Identity::generate().fingerprint();
        caller.store().unwrap().remember(&peer).unwrap();
        caller
            .save_device(
                &peer,
                Device {
                    name: "Cable Mac".into(),
                    address: "169.254.173.134:48177".into(),
                    edge: "left".into(),
                },
            )
            .unwrap();
        assert!(caller.selected_usb_route(&peer).is_none());
        assert_eq!(caller.snapshot().unwrap().peers[0].route, None);
        for receiving in [false, true] {
            caller
                .health
                .lock()
                .unwrap()
                .entry(peer.clone())
                .or_default()
                .usb_success(receiving);
            let snapshot = caller.snapshot().unwrap();
            assert_eq!(snapshot.peers[0].route, Some("usb"));
            assert_eq!(
                snapshot.peers[0].availability,
                if receiving {
                    Availability::Online
                } else {
                    Availability::ReceivingOff
                }
            );
        }
        caller
            .health
            .lock()
            .unwrap()
            .get_mut(&peer)
            .unwrap()
            .success(true);
        assert_eq!(caller.snapshot().unwrap().peers[0].route, None);
        {
            let mut statuses = caller.health.lock().unwrap();
            let health = statuses.get_mut(&peer).unwrap();
            health.usb_success(true);
            health.last = Some((Instant::now() - Duration::from_secs(26), true));
        }
        assert_eq!(caller.snapshot().unwrap().peers[0].route, None);
    }
    #[test]
    fn verified_candidate_repairs_address_but_wrong_identity_does_not() {
        let root = tempfile::tempdir().unwrap();
        let caller = app(&root.path().join("caller"));
        let server = Identity::generate();
        let peer = server.fingerprint();
        caller.store().unwrap().remember(&peer).unwrap();
        let stale = Device {
            name: "Custom name".into(),
            address: "127.0.0.2:9".into(),
            edge: "right".into(),
        };
        caller.save_device(&peer, stale.clone()).unwrap();
        for correct in [false, true] {
            let remote = Identity::generate();
            // Use the expected identity only in the second pass.
            let remote = if correct { &server } else { &remote };
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let store = TrustStore::open(root.path().join(if correct { "right" } else { "wrong" }))
                .unwrap();
            store
                .remember(&caller.identity().unwrap().fingerprint())
                .unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let _ =
                        serve_presence(listener.accept().unwrap().0, remote, &store, || Presence {
                            receiving: true,
                            port: 54321,
                            name: "Remote name".into(),
                        });
                });
                let result = caller
                    .resolve_presence(&peer, &stale, &caller.identity().unwrap(), &[address])
                    .unwrap();
                assert_eq!(result, correct);
            });
            let saved = caller.devices.lock().unwrap()[&peer].clone();
            assert_eq!(
                saved.address,
                if correct {
                    "127.0.0.1:54321"
                } else {
                    "127.0.0.2:9"
                }
            );
            assert_eq!(saved.name, "Custom name");
            assert_eq!(saved.edge, "right");
            assert_eq!(
                caller.device_info.lock().unwrap().get(&peer).cloned(),
                correct.then(extend_computer_agent::device_info::local_info)
            );
        }
        assert_eq!(
            caller.snapshot().unwrap().peers[0].availability,
            Availability::Online
        );
    }
}

#[cfg(test)]
mod route_race_tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn account_route_refresh_skips_stall_and_wrong_identity_without_manual_pairing() {
        let root = tempfile::tempdir().unwrap();
        let app = Desktop::new(root.path().join("caller"), PathBuf::from("unused-helper")).unwrap();
        *app.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
        let identity = app.identity().unwrap();
        let correct = Identity::generate();
        let wrong = Identity::generate();
        let peer = correct.fingerprint();
        // Account trust is enough: do not create a manual pairing.
        app.account_trust.replace(
            "test/account",
            std::slice::from_ref(&peer),
            Duration::from_secs(60),
        );
        let device = Device {
            name: "Custom".into(),
            address: "0.0.0.0:1".into(),
            edge: "left".into(),
        };
        app.save_device(&peer, device.clone()).unwrap();
        let stall = TcpListener::bind("127.0.0.1:0").unwrap();
        let bad = TcpListener::bind("127.0.0.1:0").unwrap();
        let good = TcpListener::bind("127.0.0.1:0").unwrap();
        let good_address = good.local_addr().unwrap();
        *app.discovered_presence.lock().unwrap() = vec![
            stall.local_addr().unwrap(),
            bad.local_addr().unwrap(),
            good_address,
        ];
        let remote_store = TrustStore::open(root.path().join("remote")).unwrap();
        remote_store.remember(&identity.fingerprint()).unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut socket = stall.accept().unwrap().0;
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = [0; 1024];
                while socket.read(&mut bytes).unwrap_or(0) != 0 {}
            });
            scope.spawn(|| {
                let _ = serve_presence(bad.accept().unwrap().0, &wrong, &remote_store, || {
                    Presence {
                        receiving: true,
                        port: 12345,
                        name: "Wrong".into(),
                    }
                });
            });
            scope.spawn(|| {
                let socket = good.accept().unwrap().0;
                std::thread::sleep(Duration::from_millis(150));
                serve_presence(socket, &correct, &remote_store, || Presence {
                    receiving: true,
                    port: 54321,
                    name: "Remote".into(),
                })
                .unwrap();
            });
            let start = Instant::now();
            assert!(app.refresh_local_route(&peer, &device, &identity).unwrap());
            assert!(start.elapsed() < Duration::from_secs(1));
        });
        let saved = app.devices.lock().unwrap()[&peer].clone();
        assert_eq!(saved.address, "127.0.0.1:54321");
        assert_eq!(saved.name, "Custom");
        assert_eq!(saved.edge, "left");
        assert!(app.store().unwrap().is_account_peer(&peer).unwrap());
        assert_eq!(app.presence_addresses.lock().unwrap()[&peer], good_address);
        assert_eq!(
            app.snapshot().unwrap().peers[0].device_info,
            Some(extend_computer_agent::device_info::local_info())
        );
    }
}
