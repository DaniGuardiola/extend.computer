//! Display names never authenticate a device or replace a user-chosen name.
use super::*;
impl Desktop {
    pub(super) fn update_peer_name(&self, peer: &str, name: &str) -> Result<()> {
        extend_computer_agent::session::validate_device_name(name)?;
        if self.store()?.peer(peer)?.is_none() {
            return Ok(());
        }
        let mut devices = self.devices.lock().unwrap();
        let Some(device) = devices.get(peer) else {
            return Ok(());
        };
        let ip = device.address.parse::<SocketAddr>()?.ip();
        if device.name != format!("Device · {ip}") && device.name != format!("Computer · {ip}") {
            return Ok(());
        }
        let mut next = devices.clone();
        next.get_mut(peer).unwrap().name = name.into();
        peers::save(&self.root, &next)?;
        *devices = next;
        Ok(())
    }
    pub(super) fn sync_peer_name(
        &self,
        peer: &str,
        device: &Device,
        identity: &Identity,
        job: Option<u64>,
    ) -> Result<()> {
        if self.store()?.peer(peer)?.is_none() {
            return Ok(());
        }
        let socket = TcpStream::connect_timeout(&device.address.parse()?, Duration::from_secs(2))?;
        if let Some(job) = job {
            self.set_socket(job, &socket)?;
        }
        let name = extend_computer_agent::session::exchange_device_name(
            socket,
            identity,
            peer,
            &self.local_name(),
        )?;
        self.update_peer_name(peer, &name)
    }
}

pub(super) fn load_local_name(root: &std::path::Path) -> Result<Option<String>> {
    match std::fs::read(root.join("device-name.json")) {
        Ok(bytes) => {
            ensure!(bytes.len() <= 1024, "Device name is too long.");
            let name: Option<String> = serde_json::from_slice(&bytes)?;
            if let Some(name) = &name { extend_computer_agent::session::validate_device_name(name)?; }
            Ok(name)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
impl Desktop {
    pub(super) fn local_name(&self) -> String {
        self.local_name_override.lock().unwrap().clone().unwrap_or_else(peers::local_name)
    }
    pub fn set_local_device_name(&self, name: String) -> Result<()> {
        use std::io::Write;
        let name = name.trim();
        let next = if name.is_empty() { None } else {
            extend_computer_agent::session::validate_device_name(name)?;
            Some(name.to_owned())
        };
        {
            let mut current = self.local_name_override.lock().unwrap();
            let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
            file.write_all(&serde_json::to_vec(&next)?)?;
            file.as_file().sync_all()?;
            file.persist(self.root.join("device-name.json"))?;
            *current = next;
        }
        self.inner.lock().unwrap().advertisement = None;
        Ok(())
    }
}
