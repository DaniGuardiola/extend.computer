//! Anonymous discovery suggests endpoints; pinned encrypted replies establish presence.
use super::*;
use extend_computer_agent::{
    discovery::Advertisement,
    error::EngineError,
    session::{query_presence, serve_presence, Presence},
};
use std::{collections::BTreeSet, net::TcpListener, sync::atomic::AtomicUsize};

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
}
impl Health {
    fn success(&mut self, receiving: bool) {
        self.last = Some((Instant::now(), receiving));
        self.failures = 0;
        self.incompatible = false;
    }
    fn incompatible(&mut self) {
        self.last = Some((Instant::now(), false));
        self.failures = 0;
        self.incompatible = true;
    }
    fn failure(&mut self) {
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
        let listener =
            TcpListener::bind(("0.0.0.0", 48178)).or_else(|_| TcpListener::bind(("0.0.0.0", 0)))?;
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
                        let _ = app.refresh_permissions();
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
                                if let Ok(store) = app.store() {
                                    let _ = serve_presence(socket, &identity, &store, || {
                                        let inner = app.inner.lock().unwrap();
                                        Presence {
                                            receiving: inner.receiving_enabled
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
        let mut endpoints = Vec::new();
        if let Some(address) = self.presence_addresses.lock().unwrap().get(peer).copied() {
            endpoints.push(address);
        }
        if let Ok(address) = device.address.parse::<SocketAddr>() {
            endpoints.push(SocketAddr::new(address.ip(), 48178));
        }
        endpoints.extend_from_slice(candidates);
        let mut seen = BTreeSet::new();
        for address in endpoints
            .into_iter()
            .filter(|a| !a.ip().is_unspecified() && seen.insert(*a))
            .take(32)
        {
            if self.closing.load(Ordering::SeqCst) {
                break;
            }
            let epoch = self.inner.lock().unwrap().next;
            let result = TcpStream::connect_timeout(&address, Duration::from_millis(400))
                .map_err(anyhow::Error::from)
                .and_then(|s| query_presence(s, identity, peer));
            match result {
                Ok(Some(status)) => {
                    // Re-check trust and update just the endpoint, preserving simultaneous UI edits.
                    if self.store()?.peer(peer)?.is_none() {
                        return Ok(false);
                    }
                    let _ = self.update_peer_name(peer, &status.name);
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
                    self.health
                        .lock()
                        .unwrap()
                        .entry(peer.into())
                        .or_default()
                        .success(status.receiving);
                    return Ok(true);
                }
                Ok(None) => {
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
        }
        if self.account_relay.online() && self.account_devices.lock().unwrap().contains_key(peer) {
            let result = self
                .account_tunnel(peer, "presence")
                .and_then(|socket| query_presence(socket, identity, peer));
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
            if let Ok(Some(status)) = result {
                if self.store()?.peer(peer)?.is_some() {
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
    fn verified_candidate_repairs_address_but_wrong_identity_does_not() {
        let root = tempfile::tempdir().unwrap();
        let caller = app(&root.path().join("caller"));
        let server = Identity::generate();
        let peer = server.fingerprint();
        caller.store().unwrap().remember(&peer, false).unwrap();
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
                .remember(&caller.identity().unwrap().fingerprint(), false)
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
        }
        assert_eq!(
            caller.snapshot().unwrap().peers[0].availability,
            Availability::Online
        );
    }
}
