use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Default, Serialize)]
pub struct Permissions {
    pub listen: bool,
    pub post: bool,
    pub available: bool,
    pub wifi: bool,
    pub wifi_pending: bool,
    pub wifi_installing: bool,
    pub wifi_setup_failed: bool,
}
impl Permissions {
    pub fn can_receive(&self) -> bool {
        self.available && self.post && self.wifi
    }
}
fn read_status(path: &Path) -> Result<String> {
    let mut child = Command::new(path)
        .arg("status")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Could not check macOS permissions.");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("macOS permission check timed out.");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut value = String::new();
    child
        .stdout
        .take()
        .context("Missing helper output")?
        .take(1024)
        .read_to_string(&mut value)?;
    Ok(value)
}
pub fn permissions(path: &Path) -> Result<Permissions> {
    if !path.is_file() {
        return Ok(Permissions::default());
    }
    let status = read_status(path)?;
    let wifi_status = crate::wifi_permission::status();
    #[cfg(not(test))]
    let wifi_ready = wifi_status == 1 && extend_computer_agent::low_jitter::ready();
    #[cfg(test)]
    let wifi_ready = status.contains("wifi=true");
    Ok(Permissions {
        listen: status.contains("listen=true"),
        post: status.contains("post=true"),
        available: true,
        wifi: wifi_ready,
        wifi_pending: wifi_status == 2,
        wifi_installing: wifi_status == 4,
        wifi_setup_failed: wifi_status == -1 || (wifi_status == 1 && !wifi_ready),
    })
}
