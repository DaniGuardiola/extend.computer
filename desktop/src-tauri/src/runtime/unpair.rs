//! Local unpair completes immediately; notifying the peer is best effort.
use super::*;
use std::io::Write;
pub(super) const REMOTE_UNPAIR_MESSAGE: &str =
    "Pairing was removed on the other device. Pair again to reconnect.";
impl Desktop {
    fn save_removals(&self, next: &BTreeMap<String, String>) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        file.write_all(&serde_json::to_vec_pretty(next)?)?;
        file.as_file().sync_all()?;
        file.persist(self.root.join("unpaired.json"))?;
        Ok(())
    }
    pub(super) fn clear_removal(&self, peer: &str) -> Result<()> {
        let mut removals = self.removals.lock().unwrap();
        if removals.contains_key(peer) {
            let mut next = removals.clone();
            next.remove(peer);
            self.save_removals(&next)?;
            *removals = next;
        }
        Ok(())
    }
    pub(super) fn record_removal(&self, peer: &str, reason: &str) -> Result<Option<String>> {
        let name = self
            .devices
            .lock()
            .unwrap()
            .get(peer)
            .map(|d| d.name.clone());
        let Some(name) = name else { return Ok(None) };
        let mut removals = self.removals.lock().unwrap();
        if removals.contains_key(peer) {
            return Ok(None);
        }
        let mut next = removals.clone();
        next.insert(peer.into(), reason.into());
        self.save_removals(&next)?;
        *removals = next;
        Ok(Some(name))
    }
    pub fn notify(&self, message: String) {
        let mut inner = self.inner.lock().unwrap();
        inner.notification_sequence += 1;
        inner.notification = Some(Notification {
            id: inner.notification_sequence,
            message,
        });
    }
    pub fn dismiss_notification(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.notification.as_ref().is_some_and(|n| n.id == id) {
            inner.notification = None;
        }
    }
    pub fn dismiss_removed(&self, peer: &str) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        ensure!(
            inner.job.is_none(),
            "Wait for the current connection to finish."
        );
        ensure!(
            self.store()?.peer(peer)?.is_none(),
            "Device is paired again."
        );
        self.remove_device_metadata(peer)
    }
    fn remove_device_metadata(&self, peer: &str) -> Result<()> {
        let mut devices = self.devices.lock().unwrap();
        let mut next = devices.clone();
        next.remove(peer);
        peers::save(&self.root, &next)?;
        *devices = next;
        drop(devices);
        self.clear_removal(peer)
    }
    pub fn received_unpair(&self, peer: &str) -> Result<()> {
        if let Some(name) = self.record_removal(peer, REMOTE_UNPAIR_MESSAGE)? {
            self.notify(format!("{name} is no longer paired."));
        }
        Ok(())
    }
    pub fn unpair(self: &Arc<Self>, peer: &str) -> Result<()> {
        let (id, _) = self.reserve(SessionKind::Unpair, Some(peer.into()))?;
        let device = self.devices.lock().unwrap().get(peer).cloned();
        let result = (|| -> Result<()> {
            self.store()?.complete_unpair(peer)?;
            self.remove_device_metadata(peer)?;
            if let Some(device) = &device {
                self.notify(format!("Unpaired from {}.", device.name));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let text = format!("{error:#}");
            self.finish(id, Err(error));
            anyhow::bail!(text);
        }
        self.finish(id, Ok(()));
        // Do not prompt for Keychain access just to deliver a nonessential hint.
        let identity = self.identity.lock().unwrap().clone();
        if let (Some(device), Some(identity)) = (device, identity) {
            let app = Arc::downgrade(self);
            let peer = peer.to_owned();
            std::thread::spawn(move || {
                let _ = (|| -> Result<()> {
                    let socket = TcpStream::connect_timeout(
                        &device.address.parse()?,
                        Duration::from_secs(2),
                    )?;
                    extend_computer_agent::session::notify_unpair_if(
                        socket,
                        &identity,
                        &peer,
                        || {
                            let Some(app) = app.upgrade() else {
                                return false;
                            };
                            let inner = app.inner.lock().unwrap();
                            !app.closing.load(Ordering::SeqCst)
                                && inner.next == id
                                && inner.job.is_none()
                        },
                    )
                })();
            });
        }
        Ok(())
    }
}
