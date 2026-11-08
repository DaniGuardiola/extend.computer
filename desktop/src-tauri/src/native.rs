#[cfg(any(test, target_os = "macos"))]
mod readiness;
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{helper::input::Status, link::Link};
use serde::Serialize;
use std::{
    io::Read,
    net::{IpAddr, TcpStream},
    path::Path,
    process::Stdio,
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
    /// An Ethernet link has a local network address, so devices can connect
    /// over it without Wi-Fi optimization.
    pub wired: bool,
    /// This device may capture its screen to extend or mirror it: macOS's
    /// Screen Recording. Always true where the OS asks for no grant.
    pub screen: bool,
    /// Cable checks skip the Wi-Fi broker, so their Wi-Fi fields carry no answer.
    #[serde(skip)]
    pub wifi_skipped: bool,
}
impl Permissions {
    /// Some local network link accepts connections: Wi-Fi once Wi-Fi
    /// optimization is ready, or Ethernet without it.
    pub fn can_receive(&self) -> bool {
        self.available && self.post && (self.wifi || self.wired)
    }
    /// Whether a connection that arrived over Wi-Fi (`wifi`) or another link
    /// can be accepted. Only Wi-Fi needs Wi-Fi optimization.
    pub fn can_receive_on(&self, wifi: bool) -> bool {
        self.available && self.post && (self.wifi || !wifi)
    }
}

/// Whether a connection on `link` needs Wi-Fi optimization, as the session
/// lease decides from the routed interface: Wi-Fi does, and so does a link
/// that can't be told apart from it; Ethernet and other links (Thunderbolt
/// Bridge, VPN tunnels) don't.
pub fn wifi_link(link: Option<Link>) -> bool {
    !matches!(link, Some(Link::Ethernet | Link::Other))
}

/// [`wifi_link`] for an accepted connection. Relay bridges arrive over
/// loopback and need no optimization.
pub fn wifi_connection(socket: &TcpStream) -> bool {
    match socket.local_addr() {
        Ok(local) if local.ip().is_loopback() => false,
        Ok(_) => wifi_link(extend_computer_agent::link::of(socket)),
        Err(_) => true,
    }
}

/// [`wifi_link`] for a session's route and its local link: cable and relay
/// sessions need no optimization.
pub fn wifi_session(route: Option<&str>, link: Option<Link>) -> bool {
    !matches!(route, Some("usb" | "internet")) && wifi_link(link)
}

/// Whether an Ethernet link has a routable local network address. Cable
/// interfaces between Macs only have link-local ones and count separately.
fn wired_link() -> bool {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .any(|interface| {
            let ip = interface.ip();
            interface.is_oper_up()
                && match ip {
                    IpAddr::V4(v4) => {
                        !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified()
                    }
                    IpAddr::V6(_) => false,
                }
                && extend_computer_agent::link::classify(ip) == Some(Link::Ethernet)
        })
}
fn read_status(path: &Path) -> Result<String> {
    let mut child = extend_computer_agent::cursor::helper_command(path)
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
#[cfg(all(not(test), target_os = "macos"))]
fn broker_ready(registration: i32, probe_allowed: bool) -> bool {
    static GATE: readiness::Gate = readiness::Gate::new();
    GATE.check(registration, probe_allowed, || {
        match extend_computer_agent::low_jitter::ready_result() {
            Ok(()) => true,
            Err(error) => {
                eprintln!("Wi-Fi broker readiness failed: registration={registration} failure={:?} elapsed_ms={}", error.failure, error.elapsed.as_millis());
                false
            }
        }
    })
}
/// Only macOS registers and probes a Wi-Fi broker.
#[cfg(all(not(test), not(target_os = "macos")))]
fn broker_ready(_: i32, _: bool) -> bool {
    extend_computer_agent::low_jitter::ready()
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
    let mut permissions = permissions_from_status(&status, wifi_status, usb, || {
        #[cfg(not(test))]
        return broker_ready(wifi_status, Status::parse(&status).post);
        #[cfg(test)]
        return status.contains("wifi=true");
    });
    // Test helpers describe the network and screen access instead of the
    // developer's Mac.
    (permissions.wired, permissions.screen) = if cfg!(test) {
        (
            status.contains("wired=true"),
            status.contains("capture=true"),
        )
    } else {
        (wired_link(), screen_access(path))
    };
    Ok(permissions)
}

/// The macOS pane that lists Screen Recording grants.
#[cfg(target_os = "macos")]
const SCREEN_RECORDING_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Whether the display helper beside `path` may capture the screen. It
/// checks as the process that captures does, so the answer follows whichever
/// app macOS holds responsible. A missing or failing helper counts as no.
fn screen_access(path: &Path) -> bool {
    if !crate::platform::SCREEN_PERMISSION {
        return true;
    }
    crate::platform::helper(path, "Display")
        .and_then(|display| read_status(&display))
        .is_ok_and(|status| capture_allowed(&status))
}

/// The display helper's `status` line: `capture=<bool> virtual=<bool>`.
fn capture_allowed(status: &str) -> bool {
    status
        .split_whitespace()
        .any(|field| field == "capture=true")
}

/// Asks for Screen Recording through the display helper beside `path`, from
/// the process that captures: the first time, macOS shows its prompt and
/// lists the app under Screen Recording. Then opens that list in System
/// Settings, where the user turns it on.
pub fn request_screen_access(path: &Path) -> Result<()> {
    let display = crate::platform::helper(path, "Display")?;
    ensure!(
        display.is_file(),
        "Display helper is missing. Rebuild extend.computer."
    );
    let mut child = extend_computer_agent::cursor::helper_command(&display)
        .arg("request-permission")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(4);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    #[cfg(target_os = "macos")]
    open::that(SCREEN_RECORDING_SETTINGS).context("Could not open System Settings.")?;
    Ok(())
}

fn permissions_from_status(
    status: &str,
    wifi_status: i32,
    usb: bool,
    wifi_probe: impl FnOnce() -> bool,
) -> Permissions {
    let wifi_ready = !usb && wifi_probe();
    let status = Status::parse(status);
    Permissions {
        listen: status.listen,
        post: status.post,
        available: true,
        wifi: wifi_ready,
        wifi_pending: wifi_status == 2,
        wifi_installing: wifi_status == 4,
        wifi_setup_failed: wifi_status == -1 || (!usb && wifi_status == 1 && !wifi_ready),
        wired: false,
        screen: false,
        wifi_skipped: usb,
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
            assert!(permissions.wifi_skipped);
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

    #[test]
    fn ethernet_receives_without_wifi_optimization_but_wifi_still_needs_it() {
        let mut ethernet_only =
            permissions_from_status("listen=true post=true", 1, false, || false);
        assert!(!ethernet_only.can_receive());
        ethernet_only.wired = true;
        assert!(ethernet_only.can_receive());
        // Each connection is judged by its own link.
        assert!(ethernet_only.can_receive_on(wifi_link(Some(Link::Ethernet))));
        assert!(ethernet_only.can_receive_on(wifi_link(Some(Link::Other))));
        assert!(!ethernet_only.can_receive_on(wifi_link(Some(Link::Wifi))));
        assert!(!ethernet_only.can_receive_on(wifi_link(None)));
        // Native permissions still gate every link.
        let denied = Permissions {
            post: false,
            ..ethernet_only.clone()
        };
        assert!(!denied.can_receive() && !denied.can_receive_on(false));
        // With Wi-Fi optimization every link receives, as before.
        let optimized = permissions_from_status("listen=true post=true", 1, false, || true);
        assert!(optimized.can_receive_on(true) && optimized.can_receive_on(false));
    }

    #[test]
    fn screen_recording_follows_the_display_helper_status() {
        assert!(capture_allowed("capture=true virtual=true\n"));
        assert!(!capture_allowed("capture=false virtual=true\n"));
        assert!(!capture_allowed(""));
        assert!(!capture_allowed("capture=truthy"));
        // A helper that isn't there can't capture.
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            screen_access(&temp.path().join("missing")),
            !crate::platform::SCREEN_PERMISSION
        );
    }

    #[test]
    fn sessions_need_wifi_optimization_only_over_wifi() {
        assert!(wifi_session(Some("local"), Some(Link::Wifi)));
        assert!(wifi_session(Some("local"), None));
        assert!(wifi_session(None, None));
        assert!(!wifi_session(Some("local"), Some(Link::Ethernet)));
        assert!(!wifi_session(Some("usb"), None));
        assert!(!wifi_session(Some("internet"), None));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        assert!(!wifi_connection(&client), "relay bridges use loopback");
    }
}
