//! Session-scoped macOS AWDL leases. GUI control sessions require optimization
//! on Wi-Fi; legacy diagnostic entry points may explicitly use best-effort mode.
use crate::session::CursorSink;
use anyhow::{ensure, Context, Result};
use std::{
    io::{BufRead, BufReader, Read},
    net::IpAddr,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{mpsc, OnceLock},
    time::{Duration, Instant},
};

const HELPER: &str =
    "/Library/PrivilegedHelperTools/computer.extend.lowjitter-development/ExtendComputerLowJitter";

static APP_HELPER: OnceLock<std::path::PathBuf> = OnceLock::new();
/// Desktop bundles select their own broker; never fall back to another profile's service.
pub fn configure_app_helper(path: std::path::PathBuf) -> Result<()> {
    APP_HELPER
        .set(path)
        .map_err(|_| anyhow::anyhow!("Wi-Fi helper already configured"))
}
fn helper_path() -> &'static Path {
    APP_HELPER
        .get()
        .map(|p| p.as_path())
        .unwrap_or_else(|| Path::new(HELPER))
}

#[derive(Debug)]
pub struct PermissionRequired;
impl std::fmt::Display for PermissionRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Allow Wi-Fi optimization in Permissions, then reconnect. It is required for sharing controls over Wi-Fi.")
    }
}
impl std::error::Error for PermissionRequired {}

/// The broker renews off the input thread until its stdin closes.
pub struct Lease(Child);
impl Lease {
    pub fn maintain(&mut self) -> Result<()> {
        ensure!(
            self.0.try_wait()?.is_none(),
            "Wi-Fi optimization stopped. Allow Wi-Fi optimization in Permissions, then reconnect."
        );
        Ok(())
    }
}
pub fn maintain_required(lease: &mut Option<Lease>) -> Result<()> {
    if let Some(active) = lease {
        active.maintain().context(PermissionRequired)?;
    }
    Ok(())
}
pub fn maintain(lease: &mut Option<Lease>) {
    if let Err(error) = maintain_required(lease) {
        eprintln!("{error}");
        lease.take();
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // EOF also reaches the broker if extend.computer is killed without running Drop.
        drop(self.0.stdin.take());
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Lease {
    fn launch(path: &Path) -> Result<Self> {
        let mut lease = Self(
            Command::new(path)
                .arg("session")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .context("launch low-jitter broker")?,
        );
        let output = lease.0.stdout.take().context("broker stdout")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(output)
                .take(256)
                .read_line(&mut line)
                .map(|_| line);
            let _ = tx.send(result);
        });
        let ready = rx
            .recv_timeout(Duration::from_secs(3))
            .context("broker startup timed out")??;
        ensure!(
            ready == "READY\n",
            "broker did not grant a session lease (helper upgrade may be required)"
        );
        Ok(lease)
    }
}

fn output(program: &str, args: &[&str]) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().context("command stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut value = String::new();
        let result = stdout.take(16384).read_to_string(&mut value).map(|_| value);
        let _ = tx.send(result);
    });
    let result = rx.recv_timeout(Duration::from_secs(2));
    // Even after stdout closes, do not wait without a bound for process exit.
    let until = Instant::now() + Duration::from_millis(100);
    while Instant::now() < until && matches!(child.try_wait(), Ok(None)) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let status = match child.try_wait()? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            child.wait()?
        }
    };
    ensure!(status.success(), "route inspection failed");
    Ok(result.context("route inspection timed out")??)
}

fn wifi_route(route: &str, hardware: &str) -> bool {
    let Some(interface) = route
        .lines()
        .find_map(|line| line.trim().strip_prefix("interface: "))
    else {
        return false;
    };
    // Identify the actual routed interface, never assume en0 means Wi-Fi.
    hardware.split("\n\n").any(|block| {
        let mut wifi = false;
        let mut device = None;
        for line in block.lines().map(str::trim) {
            if let Some(port) = line.strip_prefix("Hardware Port: ") {
                wifi = port == "Wi-Fi" || port == "AirPort";
            }
            if let Some(name) = line.strip_prefix("Device: ") {
                device = Some(name);
            }
        }
        wifi && device == Some(interface.trim())
    })
}

pub fn required_for_peer(peer: IpAddr) -> Result<bool> {
    if !cfg!(target_os = "macos") || peer.is_loopback() || peer.is_unspecified() {
        return Ok(false);
    }
    let address = peer.to_string();
    let family = if peer.is_ipv4() { "-inet" } else { "-inet6" };
    let route = output("/sbin/route", &["-n", "get", family, &address])?;
    let hardware = output("/usr/sbin/networksetup", &["-listallhardwareports"])?;
    Ok(wifi_route(&route, &hardware))
}

/// Read-only, authenticated service probe. File existence is not readiness.
pub fn ready() -> bool {
    cfg!(target_os = "macos")
        && output(helper_path().to_str().unwrap_or(""), &["status"])
            .is_ok_and(|status| status.starts_with("protocol=2 "))
}

pub fn require_ready_for_peer(peer: IpAddr) -> Result<()> {
    if required_for_peer(peer)? {
        ensure!(ready(), PermissionRequired);
    }
    Ok(())
}

pub fn start_required(peer: IpAddr) -> Result<Option<Lease>> {
    if !required_for_peer(peer)
        .context("Could not check the connection's Wi-Fi requirements. Try again.")?
    {
        return Ok(None);
    }
    ensure!(ready(), PermissionRequired);
    Lease::launch(helper_path())
        .map(Some)
        .context(PermissionRequired)
}

pub fn start_for_peer(peer: IpAddr, enabled: bool) -> Option<Lease> {
    if !enabled {
        return None;
    }
    match start_required(peer) {
        Ok(lease) => lease,
        Err(error) => {
            eprintln!("Low-jitter mode unavailable: {error:#}; using ordinary networking.");
            None
        }
    }
}

/// Wrap only an explicitly configured cursor sink. A denied request never
/// inspects routes or starts a privileged lease.
pub struct ManagedCursor<S> {
    inner: S,
    peer: IpAddr,
    enabled: bool,
    lease: Option<Lease>,
    required: bool,
}
impl<S> ManagedCursor<S> {
    pub fn required(inner: S, peer: IpAddr) -> Self {
        Self {
            inner,
            peer,
            enabled: true,
            lease: None,
            required: true,
        }
    }
    fn start(&mut self) -> Result<()> {
        self.lease = if self.required {
            start_required(self.peer)?
        } else {
            start_for_peer(self.peer, self.enabled)
        };
        Ok(())
    }
    pub fn new(inner: S, peer: IpAddr, enabled: bool) -> Self {
        Self {
            inner,
            peer,
            enabled,
            lease: None,
            required: false,
        }
    }
}
impl<S: CursorSink> CursorSink for ManagedCursor<S> {
    fn device_name(&self) -> Option<String> {
        self.inner.device_name()
    }
    fn peer_name(&mut self, peer: &str, name: &str) -> Result<()> {
        self.inner.peer_name(peer, name)
    }
    fn unpaired(&mut self, peer: &str) -> Result<()> {
        self.inner.unpaired(peer)
    }
    fn approve_control(
        &mut self,
        peer: &str,
        remembered: bool,
    ) -> Result<crate::session::ControlApproval> {
        let result = self.inner.approve_control(peer, remembered)?;
        if result != crate::session::ControlApproval::Deny {
            self.start()?;
        }
        Ok(result)
    }

    fn approve(&mut self, peer: &str) -> Result<bool> {
        if !self.inner.approve(peer)? {
            return Ok(false);
        }
        self.start()?;
        Ok(true)
    }
    fn approve_input(&mut self, peer: &str) -> Result<bool> {
        if !self.inner.approve_input(peer)? {
            return Ok(false);
        }
        self.start()?;
        Ok(true)
    }
    fn input(&mut self, event: &crate::input::InputEvent) -> Result<()> {
        self.inner.input(event)
    }
    fn heartbeat(&mut self) -> Result<()> {
        self.inner.heartbeat()?;
        if self.required {
            maintain_required(&mut self.lease)?;
        } else {
            maintain(&mut self.lease);
        }
        Ok(())
    }
    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.inner.move_to(x, y)
    }
    fn display_size(&self) -> Result<crate::session::DisplaySize> {
        self.inner.display_size()
    }
    fn finish(&mut self) {
        // Release held input before waiting for privileged lease cleanup.
        self.inner.finish();
        self.lease.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permission_failures_do_not_become_retriable_transport_errors() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "broker closed",
        ))
        .context(PermissionRequired);
        assert!(!crate::error::is_connection_failure(&error));
    }

    #[test]
    fn local_connections_do_not_need_a_privileged_helper() {
        assert!(!required_for_peer("127.0.0.1".parse().unwrap()).unwrap());
        assert!(!required_for_peer("::1".parse().unwrap()).unwrap());
        assert!(start_required("127.0.0.1".parse().unwrap())
            .unwrap()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn broker_death_is_a_permission_failure_not_a_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("broker");
        std::fs::write(&script, "#!/bin/sh\nprintf 'READY\\n'\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut lease = Some(Lease::launch(&script).unwrap());
        lease.as_mut().unwrap().0.wait().unwrap();
        assert!(maintain_required(&mut lease)
            .unwrap_err()
            .is::<PermissionRequired>());
    }

    #[test]
    fn only_actual_wifi_route_is_eligible() {
        let hardware =
            "Hardware Port: Ethernet\nDevice: en0\n\nHardware Port: Wi-Fi\nDevice: en7\n";
        assert!(wifi_route(" interface: en7\n", hardware));
        for route in [
            "interface: en0",
            "interface: utun4",
            "interface: bridge0",
            "",
            "interface:",
        ] {
            assert!(!wifi_route(route, hardware));
        }
        assert!(!wifi_route("interface: en7", "unrecognized output"));
    }
    #[cfg(unix)]
    #[test]
    fn broker_closes_on_scope_exit_and_startup_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("broker");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'READY\\n'\ncat >/dev/null\nprintf restored > \"$0.done\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        drop(Lease::launch(&script).unwrap());
        assert_eq!(
            std::fs::read(script.with_extension("done")).unwrap(),
            b"restored"
        );
        std::fs::write(&script, "#!/bin/sh\nprintf 'DENIED\\n'\n").unwrap();
        assert!(Lease::launch(&script).is_err());
    }
}
