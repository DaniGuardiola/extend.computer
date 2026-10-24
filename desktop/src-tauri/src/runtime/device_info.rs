//! Cached authenticated hardware metadata is presentation-only and keyed by peer identity.
use super::*;
use extend_computer_agent::device_info::DeviceInfo;
use std::{io::Write, path::Path};

fn valid_peer(peer: &str) -> bool {
    peer.len() == 64
        && peer
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn load(root: &Path) -> BTreeMap<String, DeviceInfo> {
    let Ok(bytes) = std::fs::read(root.join("peer-device-info.json")) else {
        return BTreeMap::new();
    };
    if bytes.len() > 512 * 1024 {
        return BTreeMap::new();
    }
    let Ok(mut infos) = serde_json::from_slice::<BTreeMap<String, DeviceInfo>>(&bytes) else {
        return BTreeMap::new();
    };
    if infos.len() > 4096 {
        return BTreeMap::new();
    }
    infos.retain(|peer, info| valid_peer(peer) && info.validate().is_ok());
    infos
}

fn save(root: &Path, infos: &BTreeMap<String, DeviceInfo>) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(infos)?;
    ensure!(
        infos.len() <= 4096 && bytes.len() <= 512 * 1024,
        "Device metadata cache is too large"
    );
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(root.join("peer-device-info.json"))?;
    Ok(())
}

impl Desktop {
    pub(super) fn update_peer_device_info(&self, peer: &str, info: DeviceInfo) -> Result<()> {
        info.validate()?;
        ensure!(valid_peer(peer), "Invalid peer identity");
        if self.store()?.peer(peer)?.is_none() {
            return Ok(());
        }
        let mut infos = self.device_info.lock().unwrap();
        if infos.get(peer) == Some(&info) {
            return Ok(());
        }
        let mut next = infos.clone();
        next.insert(peer.into(), info);
        save(&self.root, &next)?;
        *infos = next;
        Ok(())
    }
    pub(super) fn clear_peer_device_info(&self, peer: &str) -> Result<()> {
        let mut infos = self.device_info.lock().unwrap();
        if infos.contains_key(peer) {
            let mut next = infos.clone();
            next.remove(peer);
            save(&self.root, &next)?;
            *infos = next;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::device_info::DeviceKind;
    #[test]
    fn trusted_hardware_metadata_survives_restart_without_changing_preferences() {
        let files = tempfile::tempdir().unwrap();
        let root = files.path().join("state");
        let app = Desktop::new(root.clone(), files.path().join("helper")).unwrap();
        let peer = Identity::generate().fingerprint();
        app.store().unwrap().remember(&peer).unwrap();
        let device = Device {
            name: "My chosen name".into(),
            address: "127.0.0.1:48177".into(),
            edge: "right".into(),
        };
        app.devices
            .lock()
            .unwrap()
            .insert(peer.clone(), device.clone());
        peers::save(&root, &app.devices.lock().unwrap()).unwrap();
        let info = DeviceInfo {
            platform: "macos".into(),
            kind: DeviceKind::Desktop,
        };
        app.update_peer_device_info(&peer, info.clone()).unwrap();
        let restarted = Desktop::new(root, files.path().join("helper")).unwrap();
        let snapshot = restarted.snapshot().unwrap();
        assert_eq!(snapshot.peers[0].device_info, Some(info));
        assert_eq!(snapshot.peers[0].name, device.name);
        assert_eq!(snapshot.peers[0].address, device.address);
        assert_eq!(snapshot.peers[0].edge, device.edge);
        assert_eq!(
            snapshot.peers[0].availability,
            presence::Availability::Checking
        );
    }
    #[test]
    fn untrusted_and_retired_peers_cannot_write_hardware_cache() {
        let files = tempfile::tempdir().unwrap();
        let app = Desktop::new(files.path().join("state"), files.path().join("helper")).unwrap();
        let peer = Identity::generate().fingerprint();
        let info = DeviceInfo {
            platform: "macos".into(),
            kind: DeviceKind::Laptop,
        };
        app.update_peer_device_info(&peer, info.clone()).unwrap();
        assert!(app.device_info.lock().unwrap().is_empty());
        assert!(!app.root.join("peer-device-info.json").exists());
        app.store().unwrap().remember(&peer).unwrap();
        app.update_peer_device_info(&peer, info.clone()).unwrap();
        app.store().unwrap().complete_unpair(&peer).unwrap();
        app.clear_peer_device_info(&peer).unwrap();
        app.update_peer_device_info(&peer, info).unwrap();
        assert!(load(&app.root).is_empty());
    }
    #[test]
    fn corrupt_or_unbounded_optional_cache_falls_back_to_unknown() {
        let files = tempfile::tempdir().unwrap();
        let path = files.path().join("peer-device-info.json");
        for bytes in [b"{bad".to_vec(), vec![b' '; 512 * 1024 + 1]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load(files.path()).is_empty());
        }
        let peer = Identity::generate().fingerprint();
        let value = serde_json::json!({ (peer.clone()): { "platform":"macos", "kind":"future_kind" }, "bad-identity": {"platform":"macos","kind":"laptop"} });
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            load(files.path()).get(&peer).unwrap().kind,
            DeviceKind::Unknown
        );
        assert_eq!(load(files.path()).len(), 1);
    }
}
