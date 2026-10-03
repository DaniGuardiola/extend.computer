use crate::runtime::Desktop;
use extend_computer_agent::discovery::PAIRING_SERVICE;
use serde::Serialize;
use std::sync::Arc;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
#[derive(Serialize)]
pub struct Candidate {
    name: String,
    addresses: Vec<String>,
}
#[tauri::command]
pub async fn discover(state: tauri::State<'_, Arc<Desktop>>) -> Result<Vec<Candidate>, String> {
    let local_id = state.discovery_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let local_ips: Vec<_> = if_addrs::get_if_addrs()
            .map_err(err)?
            .into_iter()
            .map(|i| i.ip())
            .collect();
        let daemon = extend_computer_agent::discovery::ipv4_daemon().map_err(err)?;
        let result = (|| {
            let receiver = daemon.browse(PAIRING_SERVICE).map_err(err)?;
            let deadline = Instant::now() + Duration::from_secs(4);
            let mut found = BTreeMap::new();
            while Instant::now() < deadline {
                let event = receiver.recv_timeout(Duration::from_millis(200));
                match event {
                    Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                        let addresses: Vec<_> = info
                            .get_addresses()
                            .iter()
                            .map(|ip| ip.to_ip_addr())
                            .collect();
                        if is_self(
                            info.get_property_val_str("discovery_id"),
                            &local_id,
                            &addresses,
                            &local_ips,
                        ) {
                            found.remove(info.get_fullname());
                            continue;
                        }
                        let addresses: Vec<_> = addresses
                            .into_iter()
                            .filter(|ip| ip.is_ipv4() && !ip.is_loopback() && !ip.is_unspecified())
                            .map(|ip| std::net::SocketAddr::new(ip, info.get_port()).to_string())
                            .collect();
                        if addresses.is_empty() {
                            continue;
                        }
                        found.insert(
                            info.get_fullname().to_owned(),
                            Candidate {
                                name: info
                                    .get_property_val_str("name")
                                    .filter(|name| {
                                        !name.is_empty()
                                            && name.len() <= 100
                                            && !name.chars().any(char::is_control)
                                    })
                                    .unwrap_or("extend.computer device")
                                    .to_owned(),
                                addresses,
                            },
                        );
                    }
                    Ok(mdns_sd::ServiceEvent::ServiceRemoved(_, name)) => {
                        found.remove(&name);
                    }
                    _ => {}
                }
            }
            let _ = daemon.stop_browse(PAIRING_SERVICE);
            Ok(found.into_values().collect())
        })();
        let _ = daemon.shutdown();
        result
    })
    .await
    .map_err(err)?
}

// Discovery identifiers are untrusted filtering hints, never authentication.
fn is_self(
    advertised_id: Option<&str>,
    local_id: &str,
    addresses: &[std::net::IpAddr],
    local_ips: &[std::net::IpAddr],
) -> bool {
    advertised_id == Some(local_id)
        || addresses
            .iter()
            .any(|ip| ip.is_loopback() || local_ips.contains(ip))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn self_filter_handles_loopback_missing_addresses_and_interface_changes() {
        let local = vec!["10.0.100.23".parse().unwrap()];
        assert!(is_self(Some("self"), "self", &[], &local));
        assert!(is_self(
            None,
            "self",
            &["127.0.0.1".parse().unwrap()],
            &local
        ));
        assert!(is_self(None, "self", &local, &local));
        assert!(!is_self(
            Some("remote"),
            "self",
            &["10.0.100.246".parse().unwrap()],
            &local
        ));
    }
}
