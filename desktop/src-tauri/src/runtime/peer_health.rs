//! Infrequent checks; silence never means unpaired.
use super::*;
impl Desktop {
    pub fn start_peer_checks(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            let daemon = mdns_sd::ServiceDaemon::new().ok();
            let receiver = daemon.as_ref().and_then(|d| {
                d.browse(extend_computer_agent::discovery::PRESENCE_SERVICE)
                    .ok()
            });
            let mut candidates = BTreeMap::<String, Vec<SocketAddr>>::new();
            let mut tried_identity = false;
            let mut next_check = Instant::now();
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
                                    .take(8)
                                    .collect();
                                if candidates.len() < 64
                                    || candidates.contains_key(info.get_fullname())
                                {
                                    candidates.insert(info.get_fullname().to_owned(), addresses);
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
                let devices = app.devices.lock().unwrap().clone();
                let addresses: Vec<_> = candidates.values().flatten().copied().collect();
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
        let socket = TcpStream::connect_timeout(&device.address.parse()?, Duration::from_secs(2))?;
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
        self.store()?.complete_unpair(peer)?;
        let name = self.record_removal(peer, unpair::REMOTE_UNPAIR_MESSAGE)?;
        drop(inner);
        if let Some(name) = name {
            self.notify(format!("{name} is no longer paired."));
        }
        Ok(())
    }
}
