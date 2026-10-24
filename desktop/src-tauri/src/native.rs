mod readiness;
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
#[cfg(not(test))]
fn broker_ready(registration: i32, probe_allowed: bool) -> bool {
    static GATE: readiness::Gate = readiness::Gate::new();
    GATE.check(registration, probe_allowed, || {
        #[cfg(target_os = "macos")]
        return match extend_computer_agent::low_jitter::ready_result() {
            Ok(()) => true,
            Err(error) => {
                eprintln!("Wi-Fi broker readiness failed: registration={registration} failure={:?} elapsed_ms={}", error.failure, error.elapsed.as_millis());
                false
            }
        };
        #[cfg(not(target_os = "macos"))]
        extend_computer_agent::low_jitter::ready()
    })
}

pub fn permissions(path: &Path) -> Result<Permissions> {
    permissions_for_route(path, false)
}

/// Verified cable sessions still check the native helper and Accessibility.
/// Wi-Fi broker probes cannot gate or stall their input/resize barriers.
pub fn permissions_for_route(path: &Path, usb: bool) -> Result<Permissions> {
    if !path.is_file() {
        return Ok(Permissions::default());
    }
    let status = read_status(path)?;
    let wifi_status = crate::wifi_permission::status();
    Ok(permissions_from_status(&status, wifi_status, usb, || {
        #[cfg(not(test))]
        return broker_ready(wifi_status, status.contains("post=true"));
        #[cfg(test)]
        return status.contains("wifi=true");
    }))
}

fn permissions_from_status(
    status: &str,
    wifi_status: i32,
    usb: bool,
    wifi_probe: impl FnOnce() -> bool,
) -> Permissions {
    let wifi_ready = !usb && wifi_probe();
    Permissions {
        listen: status.contains("listen=true"),
        post: status.contains("post=true"),
        available: true,
        wifi: wifi_ready,
        wifi_pending: wifi_status == 2,
        wifi_installing: wifi_status == 4,
        wifi_setup_failed: wifi_status == -1 || (!usb && wifi_status == 1 && !wifi_ready),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_usb_skips_wifi_probe_but_keeps_native_permission_denials() {
        for post in [false, true] {
            let status = format!("listen=true post={post}");
            let permissions = permissions_from_status(&status, 1, true, || {
                panic!("Cable permission check must never probe Wi-Fi")
            });
            assert!(permissions.available && permissions.listen);
            assert_eq!(permissions.post, post);
            assert!(!permissions.wifi && !permissions.wifi_setup_failed);
            assert!(!permissions.can_receive());
        }
    }

    #[test]
    fn network_permission_checks_still_probe_and_require_wifi_readiness() {
        let mut probes = 0;
        let denied = permissions_from_status("listen=true post=true", 1, false, || {
            probes += 1;
            false
        });
        assert_eq!(probes, 1);
        assert!(!denied.can_receive() && denied.wifi_setup_failed);
        let allowed = permissions_from_status("listen=true post=true", 1, false, || {
            probes += 1;
            true
        });
        assert_eq!(probes, 2);
        assert!(allowed.can_receive());
    }
}
