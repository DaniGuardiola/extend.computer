//! Authenticated display metadata. Names never grant identity or trust.
use anyhow::{ensure, Result};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
};
pub struct Names {
    path: PathBuf,
    values: Mutex<BTreeMap<String, String>>,
    hosts: Mutex<BTreeMap<String, std::net::IpAddr>>,
    removed: Mutex<BTreeMap<String, String>>,
    local: Mutex<Option<String>>,
}
impl Names {
    pub fn open(root: &std::path::Path) -> Result<Self> {
        let path = root.join("peer-names.json");
        let values: BTreeMap<String, String> = match fs::File::open(&path) {
            Ok(file) => {
                let mut data = Vec::new();
                file.take(65537).read_to_end(&mut data)?;
                ensure!(data.len() <= 65536, "Device metadata too large");
                serde_json::from_slice(&data)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        ensure!(values.len() <= 256, "Too many device names");
        for (peer, name) in &values {
            Self::validate(peer, name)?;
        }
        let hosts_path = root.join("peer-hosts.json");
        let hosts: BTreeMap<String, std::net::IpAddr> = match fs::File::open(&hosts_path) {
            Ok(file) => {
                let mut data = Vec::new();
                file.take(65537).read_to_end(&mut data)?;
                ensure!(data.len() <= 65536, "Device hosts too large");
                serde_json::from_slice(&data)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        ensure!(hosts.len() <= 256, "Too many device hosts");
        for (peer, host) in &hosts {
            Self::validate(peer, "Host")?;
            ensure!(
                !host.is_unspecified() && !host.is_multicast(),
                "Invalid device host"
            );
        }
        let removed_path = path.with_file_name("peer-removals.json");
        let removed: BTreeMap<String, String> = match fs::File::open(&removed_path) {
            Ok(file) => {
                let mut data = Vec::new();
                file.take(65537).read_to_end(&mut data)?;
                ensure!(data.len() <= 65536, "Removal metadata too large");
                serde_json::from_slice(&data)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        ensure!(removed.len() <= 256, "Too many removal notices");
        for (peer, name) in &removed {
            Self::validate(peer, name)?;
        }
        Ok(Self {
            removed: Mutex::new(removed),
            local: Mutex::new(None),
            hosts: Mutex::new(hosts),
            path,
            values: Mutex::new(values),
        })
    }
    fn validate(peer: &str, name: &str) -> Result<()> {
        ensure!(
            peer.len() == 64 && peer.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid peer identity"
        );
        extend_computer_agent::session::validate_device_name(name)
    }
    pub fn set_local(&self, name: &str) -> Result<()> {
        extend_computer_agent::session::validate_device_name(name)?;
        *self.local.lock().unwrap() = Some(name.to_owned());
        Ok(())
    }
    pub fn local(&self, fallback: &str) -> String {
        self.local
            .lock()
            .unwrap()
            .as_deref()
            .unwrap_or(fallback)
            .to_owned()
    }
    pub fn snapshot(&self) -> Result<String> {
        Ok(serde_json::to_string(&*self.values.lock().unwrap())?)
    }
    pub fn save(&self, peer: &str, name: &str) -> Result<()> {
        Self::validate(peer, name)?;
        let mut values = self.values.lock().unwrap();
        ensure!(
            values.contains_key(peer) || values.len() < 256,
            "Too many device names"
        );
        let mut next = values.clone();
        next.insert(peer.into(), name.into());
        self.write(&next)?;
        *values = next;
        drop(values);
        self.dismiss_removal(peer)?;
        Ok(())
    }
    /// TCP source IP is only a candidate. Its port is never a receiving endpoint.
    pub fn save_host(&self, peer: &str, host: std::net::IpAddr) -> Result<()> {
        Self::validate(peer, "Host")?;
        ensure!(
            !host.is_unspecified() && !host.is_multicast(),
            "Invalid device host"
        );
        let mut hosts = self.hosts.lock().unwrap();
        ensure!(
            hosts.contains_key(peer) || hosts.len() < 256,
            "Too many device hosts"
        );
        let mut next = hosts.clone();
        next.insert(peer.into(), host);
        self.write_hosts(&next)?;
        *hosts = next;
        Ok(())
    }
    pub fn hosts(&self) -> Result<String> {
        Ok(serde_json::to_string(&*self.hosts.lock().unwrap())?)
    }
    fn write_hosts(&self, hosts: &BTreeMap<String, std::net::IpAddr>) -> Result<()> {
        let path = self.path.with_file_name("peer-hosts.json");
        Self::write_bytes(&path, &serde_json::to_vec(hosts)?)
    }
    pub fn forget(&self, peer: &str) -> Result<()> {
        let mut hosts = self.hosts.lock().unwrap();
        let mut next_hosts = hosts.clone();
        next_hosts.remove(peer);
        self.write_hosts(&next_hosts)?;
        *hosts = next_hosts;
        let mut values = self.values.lock().unwrap();
        let mut next = values.clone();
        next.remove(peer);
        self.write(&next)?;
        *values = next;
        Ok(())
    }
    pub fn removals(&self) -> Result<BTreeMap<String, String>> {
        Ok(self.removed.lock().unwrap().clone())
    }
    pub fn record_removal(&self, peer: &str) -> Result<()> {
        // Unknown authenticated peers may idempotently unpair themselves, but
        // must not create unsolicited UI notices.
        let name = self.values.lock().unwrap().get(peer).cloned();
        let Some(name) = name else { return Ok(()) };
        Self::validate(peer, &name)?;
        let mut removed = self.removed.lock().unwrap();
        let mut next = removed.clone();
        if next.len() >= 256 && !next.contains_key(peer) {
            if let Some(oldest) = next.keys().next().cloned() {
                next.remove(&oldest);
            }
        }
        next.insert(peer.to_owned(), name);
        Self::write_bytes(
            &self.path.with_file_name("peer-removals.json"),
            &serde_json::to_vec(&next)?,
        )?;
        *removed = next;
        drop(removed);
        self.forget(peer)
    }
    pub fn dismiss_removal(&self, peer: &str) -> Result<()> {
        let mut removed = self.removed.lock().unwrap();
        if !removed.contains_key(peer) {
            return Ok(());
        }
        let mut next = removed.clone();
        next.remove(peer);
        Self::write_bytes(
            &self.path.with_file_name("peer-removals.json"),
            &serde_json::to_vec(&next)?,
        )?;
        *removed = next;
        Ok(())
    }
    fn write(&self, values: &BTreeMap<String, String>) -> Result<()> {
        Self::write_bytes(&self.path, &serde_json::to_vec(values)?)
    }
    fn write_bytes(path: &std::path::Path, data: &[u8]) -> Result<()> {
        let temp = path.with_extension("tmp");
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(temp, path)?;
        Ok(())
    }
}
