//! Infrequent checks; silence never means unpaired.
use super::*;
impl Desktop {
    pub fn start_peer_checks(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            use extend_computer_agent::discovery::PRESENCE_SERVICE;
            let daemon = extend_computer_agent::discovery::ipv4_daemon().ok();
            // Notice a new cable's interface within a second.
            if let Some(daemon) = &daemon {
                let _ = daemon.set_ip_check_interval(1);
            }
            let mut receiver = daemon
                .as_ref()
                .and_then(|d| d.browse(PRESENCE_SERVICE).ok());
            let mut candidates = BTreeMap::<String, Vec<SocketAddr>>::new();
            let mut cable_candidates = Vec::<SocketAddr>::new();
            let mut tried_identity = false;
            let mut next_check = Instant::now();
            let mut links = wired::local_links();
            let mut known = Vec::<SocketAddr>::new();
            let mut rebrowse_at = None::<Instant>;
            loop {
                if let Some(receiver) = &receiver {
                    if let Ok(event) = receiver.recv_timeout(Duration::from_millis(200)) {
                        match event {
                            mdns_sd::ServiceEvent::ServiceResolved(info) => {
                                let local: Vec<_> = if_addrs::get_if_addrs()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|i| i.ip())
                                    .collect();
                                let addresses: Vec<_> = info
                                    .get_addresses()
                                    .iter()
                                    .map(|ip| ip.to_ip_addr())
                                    .filter(|ip| {
                                        ip.is_ipv4()
                                            && !ip.is_loopback()
                                            && !ip.is_unspecified()
                                            && !local.contains(ip)
                                    })
                                    .map(|ip| SocketAddr::new(ip, info.get_port()))
                                    .collect();
                                if candidates.len() < 64
                                    || candidates.contains_key(info.get_fullname())
                                {
                                    merge_candidates(
                                        candidates
                                            .entry(info.get_fullname().to_owned())
                                            .or_default(),
                                        addresses,
                                    );
                                }
                            }
                            mdns_sd::ServiceEvent::ServiceRemoved(_, name) => {
                                candidates.remove(&name);
                            }
                            _ => {}
                        }
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(200));
                }
                let Some(app) = weak.upgrade() else { break };
                if app.closing.load(Ordering::SeqCst) {
                    break;
                }
                let discovered: Vec<_> = cable_candidates
                    .iter()
                    .chain(candidates.values().flatten())
                    .copied()
                    .take(32)
                    .collect();
                // A cable or network that comes or goes changes which routes
                // work, so check right away instead of at the next interval.
                // New addresses usually arrive a few seconds after a cable.
                let current = wired::local_links();
                let mut discovered_now = discovered.clone();
                discovered_now.sort();
                discovered_now.dedup();
                if current != links || discovered_now != known {
                    if current != links {
                        rebrowse_at = Some(Instant::now() + Duration::from_millis(1500));
                    }
                    links = current;
                    known = discovered_now;
                    next_check = Instant::now();
                }
                *app.discovered_presence.lock().unwrap() = discovered;
                // Ask again on every interface once mDNS has picked up the
                // change: otherwise a new cable is only queried at its next retry.
                if rebrowse_at.is_some_and(|at| Instant::now() >= at) {
                    rebrowse_at = None;
                    if let Some(daemon) = &daemon {
                        let _ = daemon.stop_browse(PRESENCE_SERVICE);
                        receiver = daemon.browse(PRESENCE_SERVICE).ok();
                    }
                }
                if Instant::now() < next_check {
                    continue;
                }
                next_check = Instant::now() + Duration::from_secs(5);
                let Ok(records) = app.store().and_then(|s| s.load()) else {
                    continue;
                };
                if records.peers.is_empty() {
                    continue;
                }
                cable_candidates = wired::neighbor_candidates(48178);
                *app.discovered_presence.lock().unwrap() = cable_candidates
                    .iter()
                    .chain(candidates.values().flatten())
                    .copied()
                    .take(32)
                    .collect();
                let cached = app.identity.lock().unwrap().clone();
                let identity = if let Some(identity) = cached {
                    identity
                } else {
                    if tried_identity {
                        continue;
                    }
                    tried_identity = true;
                    let Ok(identity) = app.identity() else {
                        continue;
                    };
                    identity
                };
                let mut devices = app.devices.lock().unwrap().clone();
                for peer in records.peers.keys() {
                    if app.selected_usb_route(peer).is_some() {
                        devices.entry(peer.clone()).or_insert_with(|| Device {
                            name: format!("Device {}", &peer[..8]),
                            address: String::new(),
                            edge: "left".into(),
                        });
                    }
                }
                let addresses: Vec<_> = cable_candidates
                    .iter()
                    .chain(candidates.values().flatten())
                    .copied()
                    .take(32)
                    .collect();
                for (peer, device) in devices {
                    if app.closing.load(Ordering::SeqCst) {
                        break;
                    }
                    if records.peers.contains_key(&peer) && !records.revoked.contains(&peer) {
                        let _ = app.resolve_presence(&peer, &device, &identity, &addresses);
                    }
                }
            }
            if let Some(daemon) = daemon {
                let _ = daemon.shutdown();
            }
        });
    }
    #[cfg(test)]
    pub(super) fn check_peer(
        &self,
        peer: &str,
        device: &Device,
        identity: &Identity,
    ) -> Result<()> {
        let epoch = {
            let inner = self.inner.lock().unwrap();
            if inner.job.is_some() {
                return Ok(());
            }
            inner.next
        };
        let socket = extend_computer_agent::local_network::connect_timeout(
            &device.address.parse()?,
            Duration::from_secs(2),
        )?;
        if !extend_computer_agent::session::pairing_status(socket, identity, peer)? {
            self.apply_peer_unpaired(peer, epoch)?;
        } else {
            let _ = self.sync_peer_name(peer, device, identity, None);
        }
        Ok(())
    }
    pub(super) fn apply_peer_unpaired(&self, peer: &str, epoch: u64) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        if inner.next != epoch || self.closing.load(Ordering::SeqCst) {
            return Ok(());
        }
        if self.store()?.peer(peer)?.is_none() {
            return Ok(());
        }
        if self.store()?.is_account_peer(peer)? {
            return Ok(());
        }
        self.store()?.complete_unpair(peer)?;
        let name = self.record_removal(peer, unpair::REMOTE_UNPAIR_MESSAGE)?;
        drop(inner);
        if let Some(name) = name {
            self.notify(format!("{name} is no longer paired."));
        }
        Ok(())
    }
}

/// mDNS resolves a service separately on each interface, each time with only
/// that interface's addresses. Keep the union so a cable result cannot hide
/// the network address the device is still reachable on.
fn merge_candidates(known: &mut Vec<SocketAddr>, resolved: Vec<SocketAddr>) {
    for address in resolved {
        known.retain(|a| *a != address);
        known.push(address);
    }
    let excess = known.len().saturating_sub(8);
    known.drain(..excess);
}

#[cfg(test)]
mod merge_tests {
    use super::merge_candidates;
    #[test]
    fn per_interface_results_accumulate_with_newest_kept() {
        let wifi = "10.0.100.23:48178".parse().unwrap();
        let cable = "169.254.30.100:48178".parse().unwrap();
        let mut known = Vec::new();
        merge_candidates(&mut known, vec![wifi]);
        merge_candidates(&mut known, vec![cable]);
        assert_eq!(known, vec![wifi, cable]);
        merge_candidates(&mut known, vec![wifi]);
        assert_eq!(known, vec![cable, wifi]);
        let many: Vec<_> = (1..=10)
            .map(|i| format!("10.0.0.{i}:48178").parse().unwrap())
            .collect();
        merge_candidates(&mut known, many.clone());
        assert_eq!(known, many[2..]);
    }
}
