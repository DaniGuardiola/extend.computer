//! A short, plain-text summary of this app's state for a problem report,
//! written to be read by the person who shares it. It names no device, and
//! carries no device keys, emails, tokens, addresses or error text, which can
//! hold any of those.
use super::*;
use crate::support::System;
use extend_computer_agent::{device_info::DeviceKind, link::Link, session::display::Mode};
use std::fmt::Write as _;

/// The account details a report may show: where, not who.
pub struct AccountSummary<'a> {
    pub server: &'a str,
    pub signed_in: bool,
}

impl Desktop {
    pub fn diagnostics(&self, system: &System, account: &AccountSummary) -> Result<String> {
        Ok(report(system, &self.snapshot()?, account))
    }
}

fn word(value: bool, when_true: &'static str, when_false: &'static str) -> &'static str {
    if value {
        when_true
    } else {
        when_false
    }
}

/// The server's host (and port, when it isn't the default), never a path.
fn server_host(server: &str) -> String {
    url::Url::parse(server)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_owned();
            Some(match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_else(|| "unknown".into())
}

fn platform_name(platform: &str) -> &'static str {
    match platform {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        "android" => "Android",
        "ios" => "iOS",
        "ipados" => "iPadOS",
        _ => "Unknown",
    }
}

fn link_name(link: Link) -> &'static str {
    match link {
        Link::Wifi => "Wi-Fi",
        Link::Ethernet => "Ethernet",
        Link::Other => "other link",
    }
}

fn route_name(route: Option<&str>, link: Option<Link>) -> Option<String> {
    Some(match route? {
        "usb" => "USB".into(),
        "internet" => "internet relay".into(),
        "local" => match link {
            Some(link) => format!("local network ({})", link_name(link)),
            None => "local network".into(),
        },
        other => other.into(),
    })
}

fn device(peer: &Peer) -> String {
    let (platform, kind) = peer
        .device_info
        .as_ref()
        .map_or(("Unknown", "device"), |info| {
            (
                platform_name(&info.platform),
                match info.kind {
                    DeviceKind::Desktop => "desktop",
                    DeviceKind::Laptop => "laptop",
                    DeviceKind::Phone => "phone",
                    DeviceKind::Tablet => "tablet",
                    DeviceKind::Unknown => "device",
                },
            )
        });
    let trust = match peer.trust_source {
        TrustSource::Pairing => "paired",
        TrustSource::Account => "account",
    };
    let availability = match peer.availability {
        presence::Availability::Checking => "checking",
        presence::Availability::Online => "online",
        presence::Availability::ReceivingOff => "not accepting connections",
        presence::Availability::Offline => "offline",
        presence::Availability::UpdateRequired => "update required",
    };
    let mut line = format!("{platform} {kind}, {trust}: {availability}");
    if let Some(route) = route_name(peer.route, None) {
        let _ = write!(line, ", {route}");
    }
    if let Some(link) = peer.link {
        let _ = write!(line, ", on {}", link_name(link));
    }
    line
}

fn session(session: &Session) -> String {
    let what = match session.kind {
        SessionKind::Pair => "pairing",
        SessionKind::Unpair => "unpairing",
        _ => match session.display {
            Some(Mode::Extend) => "extended display",
            Some(Mode::Mirror) => "mirrored display",
            None => "keyboard and mouse",
        },
    };
    let direction = match session.kind {
        SessionKind::Incoming => "incoming",
        SessionKind::Outgoing => "outgoing",
        SessionKind::Pair | SessionKind::Unpair => "",
    };
    let phase = match session.phase {
        Phase::Connecting if session.reconnecting => "reconnecting",
        Phase::Connecting => "connecting",
        Phase::Approval => "waiting for approval",
        Phase::Connected => "connected",
        Phase::Disconnecting => "disconnecting",
    };
    let mut line = what.to_owned();
    for part in [direction, phase] {
        if !part.is_empty() {
            let _ = write!(line, ", {part}");
        }
    }
    if let Some(route) = route_name(session.route, session.link) {
        let _ = write!(line, ", {route}");
    }
    if let Some(quality) = &session.quality {
        let level = match quality.level {
            quality::Level::Good => "good",
            quality::Level::Fair => "fair",
            quality::Level::Poor => "poor",
        };
        let _ = write!(line, ", {level} connection");
    }
    line
}

pub(crate) fn report(system: &System, snapshot: &Snapshot, account: &AccountSummary) -> String {
    let mut text = String::new();
    let mut line = |label: &str, value: &str| {
        let _ = writeln!(text, "{label}: {value}");
    };
    line("extend.computer", &system.app_version());
    line("OS", &system.os);
    line("Architecture", system.arch);
    line(
        "Account server",
        &format!(
            "{}, {}",
            server_host(account.server),
            word(account.signed_in, "signed in", "signed out")
        ),
    );
    let permissions = &snapshot.permissions;
    let capabilities = &snapshot.capabilities;
    line("Allow connections", word(snapshot.receiving, "on", "off"));
    line(
        "Input helper",
        word(permissions.available, "found", "missing"),
    );
    if capabilities.input_permissions {
        line(
            "Accessibility",
            word(permissions.post, "allowed", "not allowed"),
        );
        line(
            "Keyboard and mouse capture",
            word(permissions.listen, "ready", "not ready"),
        );
    }
    if capabilities.wifi_optimization {
        line(
            "Wi-Fi optimization",
            // The same order as the receiving error's explanation.
            if permissions.wifi {
                "allowed"
            } else if permissions.wifi_pending {
                "waiting for approval"
            } else if permissions.wifi_installing {
                "setting up"
            } else if permissions.wifi_setup_failed {
                "setup failed"
            } else {
                "not allowed"
            },
        );
    }
    let starts: Vec<_> = [
        (capabilities.extend_display, "extend display"),
        (capabilities.mirror_display, "mirror display"),
        (capabilities.share_input, "share keyboard and mouse"),
    ]
    .into_iter()
    .filter_map(|(able, name)| able.then_some(name))
    .collect();
    line(
        "Can start",
        &if starts.is_empty() {
            "nothing".into()
        } else {
            starts.join(", ")
        },
    );
    line(
        "Session",
        &snapshot
            .session
            .as_ref()
            .map_or_else(|| "none".into(), session),
    );
    line("Devices", &snapshot.peers.len().to_string());
    for (number, peer) in snapshot.peers.iter().enumerate() {
        let _ = writeln!(text, "  {}. {}", number + 1, device(peer));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::device_info::DeviceInfo;

    const EMAIL: &str = "person@example.com";
    const NAME: &str = "Dani’s MacBook";
    const ADDRESS: &str = "192.168.1.21:48177";

    fn peer(id: &str, platform: &str, kind: DeviceKind) -> Peer {
        Peer {
            id: id.into(),
            name: NAME.into(),
            address: ADDRESS.into(),
            edge: "left".into(),
            availability: presence::Availability::Online,
            route: None,
            trust_source: TrustSource::Pairing,
            device_info: Some(DeviceInfo {
                platform: platform.into(),
                kind,
            }),
            link: Some(Link::Wifi),
            offers: None,
        }
    }

    fn snapshot() -> Snapshot {
        let mac = "a".repeat(64);
        let tablet = "b".repeat(64);
        Snapshot {
            development: false,
            discoverable: false,
            removed_peers: Vec::new(),
            notification: None,
            peers: vec![
                peer(&mac, "macos", DeviceKind::Laptop),
                Peer {
                    availability: presence::Availability::Offline,
                    trust_source: TrustSource::Account,
                    route: Some("usb"),
                    link: None,
                    ..peer(&tablet, "android", DeviceKind::Tablet)
                },
            ],
            addresses: vec![ADDRESS.into()],
            session: Some(Session {
                id: 7,
                kind: SessionKind::Outgoing,
                phase: Phase::Connected,
                peer: Some(mac),
                route: Some("local"),
                link: Some(Link::Ethernet),
                quality: None,
                reconnecting: false,
                display: Some(Mode::Extend),
                display_config: None,
            }),
            receiving: true,
            code: Some("a1b2-c3d4-e5f6-7890".into()),
            code_seconds: 90,
            approval: None,
            error: Some(format!("Could not reach {NAME} at {ADDRESS}.")),
            error_peer: None,
            permissions: native::Permissions {
                listen: true,
                post: true,
                available: true,
                wifi: false,
                wifi_setup_failed: true,
                ..Default::default()
            },
            permission_request: None,
            capabilities: crate::platform::Capabilities {
                extend_display: true,
                mirror_display: true,
                share_input: true,
                remote_desktop: true,
                input_permissions: true,
                wifi_optimization: true,
                screen_permission: true,
            },
        }
    }

    fn system() -> System {
        System {
            version: "0.4.0",
            development: false,
            os: "macOS 15.5".into(),
            arch: "arm64",
        }
    }

    #[test]
    fn report_reads_as_plain_lines() {
        let report = report(
            &system(),
            &snapshot(),
            &AccountSummary {
                server: "https://accounts.example.test:8443",
                signed_in: true,
            },
        );
        assert_eq!(
            report,
            "extend.computer: 0.4.0
OS: macOS 15.5
Architecture: arm64
Account server: accounts.example.test:8443, signed in
Allow connections: on
Input helper: found
Accessibility: allowed
Keyboard and mouse capture: ready
Wi-Fi optimization: setup failed
Can start: extend display, mirror display, share keyboard and mouse
Session: extended display, outgoing, connected, local network (Ethernet)
Devices: 2
  1. macOS laptop, paired: online, on Wi-Fi
  2. Android tablet, account: offline, USB
"
        );
    }

    #[test]
    fn report_leaves_out_names_keys_addresses_emails_and_errors() {
        let snapshot = snapshot();
        let report = report(
            &system(),
            &snapshot,
            &AccountSummary {
                server: "https://extend.computer",
                signed_in: true,
            },
        );
        assert!(report.contains("Account server: extend.computer, signed in"));
        for secret in [
            NAME,
            "192.168.",
            ":48177",
            EMAIL,
            "a1b2-c3d4",
            "Could not reach",
            "https://",
            &"a".repeat(16),
            &"b".repeat(16),
        ] {
            assert!(!report.contains(secret), "{secret} leaked:\n{report}");
        }
    }

    #[test]
    fn wifi_optimization_reads_like_the_receiving_explanation() {
        let account = AccountSummary {
            server: "https://extend.computer",
            signed_in: false,
        };
        for (permissions, expected) in [
            (
                native::Permissions {
                    wifi: true,
                    wifi_pending: true,
                    ..Default::default()
                },
                "allowed",
            ),
            (
                native::Permissions {
                    wifi_pending: true,
                    wifi_setup_failed: true,
                    ..Default::default()
                },
                "waiting for approval",
            ),
            (
                native::Permissions {
                    wifi_installing: true,
                    ..Default::default()
                },
                "setting up",
            ),
            (
                native::Permissions {
                    wifi_setup_failed: true,
                    ..Default::default()
                },
                "setup failed",
            ),
            (native::Permissions::default(), "not allowed"),
        ] {
            let snapshot = Snapshot {
                permissions,
                ..snapshot()
            };
            let report = report(&system(), &snapshot, &account);
            assert!(
                report.contains(&format!("\nWi-Fi optimization: {expected}\n")),
                "{expected}:\n{report}"
            );
        }
    }

    #[test]
    fn report_follows_platform_grants_and_idle_state() {
        let mut snapshot = snapshot();
        snapshot.capabilities.input_permissions = false;
        snapshot.capabilities.wifi_optimization = false;
        snapshot.capabilities.extend_display = false;
        snapshot.session = None;
        snapshot.peers.clear();
        snapshot.receiving = false;
        let report = report(
            &System {
                development: true,
                os: "Windows 10.0.26100.4349".into(),
                arch: "x86_64",
                ..system()
            },
            &snapshot,
            &AccountSummary {
                server: "https://extend.computer",
                signed_in: false,
            },
        );
        assert!(report.starts_with("extend.computer: 0.4.0 (development)\nOS: Windows"));
        assert!(report.contains("Account server: extend.computer, signed out\n"));
        assert!(report.contains("Allow connections: off\n"));
        assert!(!report.contains("Accessibility") && !report.contains("Wi-Fi optimization"));
        assert!(report.contains("Can start: mirror display, share keyboard and mouse\n"));
        assert!(report.contains("Session: none\nDevices: 0\n"));
    }

    #[test]
    fn sessions_describe_pairing_and_reconnecting() {
        let pairing = Session {
            id: 1,
            kind: SessionKind::Pair,
            phase: Phase::Approval,
            peer: None,
            route: None,
            link: None,
            quality: None,
            reconnecting: false,
            display: None,
            display_config: None,
        };
        assert_eq!(session(&pairing), "pairing, waiting for approval");
        let reconnecting = Session {
            kind: SessionKind::Outgoing,
            phase: Phase::Connecting,
            reconnecting: true,
            route: Some("usb"),
            ..pairing
        };
        assert_eq!(
            session(&reconnecting),
            "keyboard and mouse, outgoing, reconnecting, USB"
        );
    }
}
