use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, path::Path};
#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub address: String,
    pub edge: String,
}
pub fn validate(device: &Device) -> Result<()> {
    ensure!(
        !device.name.trim().is_empty()
            && device.name.len() <= 100
            && !device.name.chars().any(char::is_control),
        "Enter a short device name."
    );
    device
        .address
        .parse::<std::net::SocketAddr>()
        .map_err(|_| {
            anyhow::anyhow!("Enter an IP address and port, for example 192.168.1.20:48177.")
        })?;
    ensure!(
        matches!(device.edge.as_str(), "left" | "right"),
        "Choose a screen edge."
    );
    Ok(())
}
pub fn load(root: &Path) -> Result<BTreeMap<String, Device>> {
    match std::fs::read(root.join("devices.json")) {
        Ok(bytes) => {
            ensure!(bytes.len() < 1024 * 1024, "Device settings are too large.");
            Ok(serde_json::from_slice(&bytes)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}
pub fn save(root: &Path, devices: &BTreeMap<String, Device>) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(&serde_json::to_vec_pretty(devices)?)?;
    file.as_file().sync_all()?;
    file.persist(root.join("devices.json"))?;
    Ok(())
}

pub fn local_name() -> String {
    std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| extend_computer_agent::session::validate_device_name(s).is_ok())
        .unwrap_or_else(|| "extend.computer device".into())
}
