use crate::{
    approval::{Answer, Approvals, Request},
    native::{self, Permissions},
    peers::{self, Device},
};
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{identity::Identity, pairing::PairingWindow, trust::TrustStore};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    net::{Shutdown, SocketAddr, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
mod device_info;
mod device_names;
mod display;
mod errors;
mod incoming;
mod outgoing;
mod peer_health;
mod preferences;
mod presence;
mod routes;
#[cfg(test)]
mod tests;
mod unpair;
mod usb;
#[cfg(any(test, debug_assertions, feature = "display-benchmark"))]
mod usb_control_test;
mod wired;
pub(crate) use errors::friendly_error;
use errors::is_connection_error;
mod state;
pub use state::Session;
use state::{Phase, SessionKind};

struct Job {
    view: Session,
    cancelled: Arc<AtomicBool>,
    /// The local user pressed Disconnect, so the peer is told not to reconnect.
    ended_locally: bool,
    socket: Option<TcpStream>,
    display_updates: Option<std::sync::mpsc::Sender<display::SettingsUpdate>>,
}
struct PairCode {
    window: PairingWindow,
    expires: Instant,
}
impl PairCode {
    fn new() -> Self {
        let lifetime = Duration::from_secs(120);
        Self {
            window: PairingWindow::new(lifetime),
            expires: Instant::now() + lifetime,
        }
    }
}
struct Inner {
    next: u64,
    job: Option<Job>,
    listener: Option<Arc<AtomicBool>>,
    listener_port: u16,
    receiving_enabled: bool,
    pairing_open: bool,
    advertisement: Option<extend_computer_agent::discovery::Advertisement>,
    code: Option<PairCode>,
    error: Option<String>,
    /// The device a session error belongs to, paired with that exact message
    /// so a later unrelated error is never shown on its card.
    error_peer: Option<(String, String)>,
    permissions: Permissions,
    permission_request: Option<String>,
    notification: Option<Notification>,
    notification_sequence: u64,
}
impl Inner {
    fn receiving_paused(&mut self, message: String) {
        eprintln!("Receiving paused: {message}");
        self.receiving_enabled = false;
        self.error = Some(message.clone());
        self.notification_sequence += 1;
        self.notification = Some(Notification {
            id: self.notification_sequence,
            message,
        });
    }
}
#[derive(Serialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub address: String,
    pub edge: String,
    pub availability: presence::Availability,
    pub route: Option<&'static str>,
    pub trust_source: TrustSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_info: Option<extend_computer_agent::device_info::DeviceInfo>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustSource {
    Pairing,
    Account,
}
#[derive(Clone, Serialize)]
pub struct Notification {
    pub id: u64,
    pub message: String,
}
#[derive(Serialize)]
pub struct RemovedPeer {
    pub id: String,
    pub name: String,
    pub reason: String,
}
#[derive(Serialize)]
pub struct LocalDeviceInfo {
    pub name: String,
    pub identity: String,
    pub version: String,
}
#[derive(Serialize)]
pub struct Snapshot {
    pub development: bool,
    pub discoverable: bool,
    pub removed_peers: Vec<RemovedPeer>,
    pub notification: Option<Notification>,
    pub peers: Vec<Peer>,
    pub addresses: Vec<String>,
    pub session: Option<Session>,
    pub receiving: bool,
    pub code: Option<String>,
    pub code_seconds: u64,
    pub approval: Option<Request>,
    pub error: Option<String>,
    pub error_peer: Option<String>,
    pub permissions: Permissions,
    pub permission_request: Option<String>,
}
pub struct Desktop {
    pub root: PathBuf,
    pub discovery_id: String,
    pub helper: PathBuf,
    identity: Mutex<Option<Arc<Identity>>>,
    pub account_relay: Arc<crate::relay::Relay>,
    account_devices: Mutex<BTreeMap<String, String>>,
    presence_port: Mutex<u16>,
    account_trust: extend_computer_agent::account_trust::AccountTrust,
    receiving_preference: Mutex<bool>,
    local_name_override: Mutex<Option<String>>,
    devices: Mutex<BTreeMap<String, Device>>,
    device_info: Mutex<BTreeMap<String, extend_computer_agent::device_info::DeviceInfo>>,
    health: Mutex<BTreeMap<String, presence::Health>>,
    presence_addresses: Mutex<BTreeMap<String, SocketAddr>>,
    discovered_presence: Mutex<Vec<SocketAddr>>,
    inner: Mutex<Inner>,
    removals: Mutex<BTreeMap<String, String>>,
    #[cfg(target_os = "macos")]
    usb: Mutex<Option<usb::Connection>>,
    usb_connecting: AtomicBool,
    #[cfg(test)]
    test_usb_network: AtomicBool,
    usb_epoch: std::sync::atomic::AtomicU64,
    closing: AtomicBool,
    permission_checks: Mutex<u64>,
    update_pending: AtomicBool,
    pub approvals: Approvals,
}
impl Desktop {
    pub fn new(root: PathBuf, helper: PathBuf) -> Result<Arc<Self>> {
        TrustStore::open(&root)?;
        let devices = peers::load(&root)?;
        let mut removals: BTreeMap<String, String> = match std::fs::read(root.join("unpaired.json"))
        {
            Ok(data) => {
                ensure!(
                    data.len() < 1024 * 1024,
                    "Unpaired device history too large"
                );
                serde_json::from_slice(&data)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        // Older builds recorded local unpairs as placeholders too. Hide only
        // those exact legacy records; remote notices remain until dismissed.
        removals.retain(|_, reason| {
            reason != "Unpaired on this computer. Pair again to reconnect."
                && reason != "Unpaired on this computer."
        });
        for reason in removals.values_mut() {
            if reason == "Unpaired from the other computer. Pair again to reconnect." {
                *reason = unpair::REMOTE_UNPAIR_MESSAGE.into();
            }
        }
        Ok(Arc::new(Self {
            account_relay: Arc::new(Default::default()),
            account_devices: Default::default(),
            presence_port: Mutex::new(48178),
            account_trust: Default::default(),
            receiving_preference: Mutex::new(preferences::load(&root)?),
            local_name_override: Mutex::new(device_names::load_local_name(&root)?),
            device_info: Mutex::new(device_info::load(&root)),
            root,
            discovery_id: Identity::generate().fingerprint(),
            helper,
            identity: Mutex::new(None),
            devices: Mutex::new(devices),
            health: Mutex::new(BTreeMap::new()),
            presence_addresses: Mutex::new(BTreeMap::new()),
            discovered_presence: Mutex::new(Vec::new()),
            approvals: Approvals::default(),
            removals: Mutex::new(removals),
            #[cfg(target_os = "macos")]
            usb: Mutex::new(None),
            usb_connecting: AtomicBool::new(false),
            #[cfg(test)]
            test_usb_network: AtomicBool::new(false),
            usb_epoch: std::sync::atomic::AtomicU64::new(0),
            closing: AtomicBool::new(false),
            permission_checks: Mutex::new(0),
            update_pending: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                next: 0,
                job: None,
                listener: None,
                listener_port: 0,
                receiving_enabled: false,
                pairing_open: false,
                advertisement: None,
                code: None,
                error: None,
                error_peer: None,
                permissions: Permissions::default(),
                permission_request: None,
                notification: None,
                notification_sequence: 0,
            }),
        }))
    }
    fn identity(&self) -> Result<Arc<Identity>> {
        let mut identity = self.identity.lock().unwrap();
        if let Some(value) = identity.as_ref() {
            return Ok(value.clone());
        }
        #[cfg(all(feature = "dev-identity", debug_assertions, unix))]
        let value = Arc::new(Identity::load_development(&self.root)?);
        #[cfg(not(all(feature = "dev-identity", debug_assertions, unix)))]
        let value =
            Arc::new(Identity::load_persistent("default").context(errors::KeychainAccessError)?);
        *identity = Some(value.clone());
        Ok(value)
    }
    fn store(&self) -> Result<TrustStore> {
        Ok(TrustStore::open(&self.root)?.with_account_trust(self.account_trust.clone()))
    }
    pub fn clear_account_peers(&self) {
        self.account_relay.configure(None);
        self.account_trust.clear();
        self.account_devices.lock().unwrap().clear();
    }
    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }
    pub fn account_peer_allowed(&self, peer: &str) -> Result<bool> {
        Ok(self.store()?.peer(peer)?.is_some()
            && self.account_devices.lock().unwrap().contains_key(peer))
    }
    pub fn relay_listener_port(&self, kind: &str) -> Result<u16> {
        match kind {
            "presence" => Ok(*self.presence_port.lock().unwrap()),
            "control" => {
                let inner = self.inner.lock().unwrap();
                ensure!(
                    inner.receiving_enabled
                        && inner
                            .listener
                            .as_ref()
                            .is_some_and(|stop| !stop.load(Ordering::SeqCst)),
                    "Receiving is off"
                );
                Ok(inner.listener_port)
            }
            _ => anyhow::bail!("Invalid relay connection kind"),
        }
    }
    fn account_tunnel(&self, peer: &str, kind: &str) -> Result<TcpStream> {
        ensure!(
            self.store()?.peer(peer)?.is_some(),
            "Account device access expired"
        );
        let id = self
            .account_devices
            .lock()
            .unwrap()
            .get(peer)
            .cloned()
            .context("Device is not signed in to this account")?;
        self.account_relay.tunnel(&id, kind)
    }
    pub fn sync_account_peers(&self, scope: &str, directory: &[serde_json::Value]) -> Result<()> {
        let own = self.identity()?.fingerprint();
        let fingerprints: Vec<String> = directory
            .iter()
            .filter(|d| d["key_verified"] == true || d["key_verified"] == 1)
            .filter_map(|d| d["fingerprint"].as_str())
            .filter(|id| *id != own)
            .map(str::to_owned)
            .collect();
        self.account_trust
            .replace(scope, &fingerprints, Duration::from_secs(90));
        let mut devices = self.devices.lock().unwrap();
        let mut account_devices = self.account_devices.lock().unwrap();
        account_devices.clear();
        for item in directory {
            let Some(id) = item["fingerprint"].as_str() else {
                continue;
            };
            if !fingerprints.iter().any(|fp| fp == id) {
                continue;
            }
            if let Some(device_id) = item["id"].as_str() {
                account_devices.insert(id.into(), device_id.into());
            }
            devices.entry(id.into()).or_insert_with(|| Device {
                name: item["name"].as_str().unwrap_or("Account device").into(),
                address: "0.0.0.0:48177".into(),
                edge: "left".into(),
            });
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn receive_test_port(self: &Arc<Self>, port: u16) -> Result<()> {
        self.receive_at(false, port)
    }
    #[cfg(test)]
    pub fn set_test_identity(&self, identity: Identity) {
        *self.identity.lock().unwrap() = Some(Arc::new(identity));
    }
    pub fn account_public_key(&self) -> Result<String> {
        Ok(hex::encode(self.identity()?.public()))
    }
    pub fn account_key_proof(&self, server_key: &str, challenge: &str) -> Result<String> {
        self.identity()?.account_proof(server_key, challenge)
    }
    pub fn local_device_info(&self) -> Result<LocalDeviceInfo> {
        Ok(LocalDeviceInfo {
            name: self.local_name(),
            identity: self.identity()?.fingerprint(),
            version: env!("CARGO_PKG_VERSION").into(),
        })
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let store = self.store()?;
        let records = store.load()?;
        let devices = self.devices.lock().unwrap();
        let removals = self.removals.lock().unwrap();
        let removed_peers = devices
            .iter()
            .filter(|(id, _)| !records.peers.contains_key(*id))
            .filter_map(|(id, device)| {
                let reason = removals.get(id)?.clone();
                Some(RemovedPeer {
                    id: id.clone(),
                    name: device.name.clone(),
                    reason,
                })
            })
            .collect();
        drop(removals);
        let mut peers: Vec<Peer> = records
            .peers
            .into_iter()
            .filter(|(id, _)| !records.revoked.contains(id))
            .map(|(id, _)| -> Result<Peer> {
                let d = devices.get(&id);
                let trust_source = if store.is_account_peer(&id)? {
                    TrustSource::Account
                } else {
                    TrustSource::Pairing
                };
                let usb = self.selected_usb_route(&id);
                let health = self.health.lock().unwrap();
                let status = health.get(&id);
                let mut availability = status
                    .map(|h| h.state())
                    .unwrap_or(presence::Availability::Checking);
                let usb_verified = status.and_then(|h| h.route()) == Some("usb");
                drop(health);
                let route = if let Some(usb) = usb {
                    if !usb.current(self) {
                        availability = presence::Availability::Offline;
                        None
                    } else if !usb_verified {
                        availability = presence::Availability::Checking;
                        None
                    } else {
                        Some("usb")
                    }
                } else {
                    // macOS USB networking uses authenticated presence without
                    // selecting an Android bulk connection.
                    usb_verified.then_some("usb")
                };
                Ok(Peer {
                    name: d
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| format!("Device {}", &id[..8.min(id.len())])),
                    address: d.map(|d| d.address.clone()).unwrap_or_default(),
                    edge: d.map(|d| d.edge.clone()).unwrap_or_else(|| "left".into()),
                    availability,
                    route,
                    trust_source,
                    device_info: self.device_info.lock().unwrap().get(&id).cloned(),
                    id,
                })
            })
            .collect::<Result<_>>()?;
        drop(devices);
        let inner = self.inner.lock().unwrap();
        if let Some(job) = &inner.job {
            if job.view.phase == Phase::Connected {
                if let Some(peer) = peers
                    .iter_mut()
                    .find(|p| Some(&p.id) == job.view.peer.as_ref())
                {
                    peer.availability = presence::Availability::Online;
                }
            }
        }
        let seconds = inner
            .code
            .as_ref()
            .map(|c| {
                c.expires
                    .saturating_duration_since(Instant::now())
                    .as_secs()
            })
            .unwrap_or(0);
        Ok(Snapshot {
            development: cfg!(feature = "dev-identity"),
            discoverable: inner.pairing_open
                && inner.advertisement.is_some()
                && inner.job.is_none(),
            removed_peers,
            notification: inner.notification.clone(),
            peers,
            addresses: if_addrs::get_if_addrs()
                .unwrap_or_default()
                .into_iter()
                .filter(|i| i.ip().is_ipv4() && !i.is_loopback())
                .map(|i| SocketAddr::new(i.ip(), 48177).to_string())
                .collect(),
            session: inner
                .job
                .as_ref()
                .filter(|j| {
                    !(j.view.kind == SessionKind::Incoming && j.view.phase == Phase::Connecting)
                })
                .map(|j| j.view.clone()),
            receiving: inner.receiving_enabled
                && inner
                    .listener
                    .as_ref()
                    .is_some_and(|stop| !stop.load(Ordering::SeqCst)),
            code: inner
                .code
                .as_ref()
                .filter(|_| seconds > 0)
                .and_then(|c| c.window.code().map(String::from)),
            code_seconds: seconds,
            approval: self.approvals.current(),
            error: inner.error.clone(),
            error_peer: inner
                .error_peer
                .as_ref()
                .filter(|(_, message)| inner.error.as_ref() == Some(message))
                .map(|(peer, _)| peer.clone()),
            permissions: inner.permissions.clone(),
            permission_request: inner.permission_request.clone(),
        })
    }
    pub fn refresh_permissions(&self) -> Result<Permissions> {
        let ticket = self.begin_permission_check();
        self.apply_permissions(ticket, native::permissions(&self.helper))
    }
    fn refresh_permissions_for_route(&self, usb: bool) -> Result<Permissions> {
        if !usb {
            return self.refresh_permissions();
        }
        let ticket = self.begin_permission_check();
        self.apply_permissions(ticket, native::permissions_for_route(&self.helper, usb))
    }
    fn usb_session(&self, id: u64) -> bool {
        self.inner.lock().unwrap().job.as_ref().is_some_and(|job| {
            job.view.id == id
                && job.view.route == Some("usb")
                && !job.cancelled.load(Ordering::SeqCst)
        })
    }
    /// A cable session, or an idle active NCM interface, needs native permission
    /// checks without a Wi-Fi broker round trip. Network callers still probe
    /// Wi-Fi separately before accepting their own authenticated connection.
    fn refresh_background_permissions(&self) -> Result<Permissions> {
        let active = self.inner.lock().unwrap().job.as_ref().map(|job| {
            (
                job.view.id,
                job.view.route == Some("usb") && !job.cancelled.load(Ordering::SeqCst),
            )
        });
        let usb = active.map_or_else(|| self.usb_network_receiving_available(), |(_, usb)| usb);
        let ticket = self.begin_permission_check();
        let result = native::permissions_for_route(&self.helper, usb);
        self.apply_permissions_scoped(ticket, result, active.map(|(id, _)| id), active.is_none())
    }
    fn begin_permission_check(&self) -> u64 {
        let mut ticket = self.permission_checks.lock().unwrap();
        *ticket = ticket
            .checked_add(1)
            .expect("Permission check generation exhausted");
        *ticket
    }
    fn apply_permissions(&self, ticket: u64, result: Result<Permissions>) -> Result<Permissions> {
        self.apply_permissions_for_session(ticket, result, None)
    }
    fn apply_permissions_for_session(
        &self,
        ticket: u64,
        result: Result<Permissions>,
        expected_session: Option<u64>,
    ) -> Result<Permissions> {
        self.apply_permissions_scoped(ticket, result, expected_session, false)
    }
    fn apply_permissions_scoped(
        &self,
        ticket: u64,
        result: Result<Permissions>,
        expected_session: Option<u64>,
        expected_idle: bool,
    ) -> Result<Permissions> {
        let generation = self.permission_checks.lock().unwrap();
        if *generation != ticket {
            // Callers must not authorize a fresh receive from an obsolete success.
            return Ok(self.inner.lock().unwrap().permissions.clone());
        }
        let mut value = result.as_ref().cloned().unwrap_or_default();
        if value.wifi_skipped {
            // A cable-only check has no Wi-Fi answer. Keep the last real one so
            // the UI and local-network presence don't flip to "not ready".
            let previous = self.inner.lock().unwrap().permissions.clone();
            value.wifi = previous.wifi;
            value.wifi_pending = previous.wifi_pending;
            value.wifi_installing = previous.wifi_installing;
            value.wifi_setup_failed = previous.wifi_setup_failed;
        }
        let can_receive = self.receiving_permissions(&value);
        let cancel = {
            let mut inner = self.inner.lock().unwrap();
            if expected_idle && inner.job.is_some() {
                return Ok(inner.permissions.clone());
            }
            if expected_session.is_some_and(|id| {
                !inner
                    .job
                    .as_ref()
                    .is_some_and(|job| job.view.id == id && !job.cancelled.load(Ordering::SeqCst))
            }) {
                return Ok(inner.permissions.clone());
            }
            inner.permissions = value.clone();
            let active_usb = inner.job.as_ref().is_some_and(|job| {
                job.view.route == Some("usb") && !job.cancelled.load(Ordering::SeqCst)
            });
            let can_receive = can_receive || (active_usb && value.available && value.post);
            if inner.receiving_enabled && !can_receive {
                let reason = match &result {
                    Err(error) => {
                        let detail = format!("{error:#}");
                        format!("Could not verify macOS permissions: {}. Retry Allow connections; if this continues, reopen or reinstall extend.computer.", detail.trim_end_matches('.'))
                    }
                    Ok(_) if !value.available => "The input helper is unavailable. Reopen or reinstall extend.computer, then retry Allow connections.".into(),
                    Ok(_) => {
                        let mut missing = Vec::new();
                        if !value.post {
                            missing.push("Accessibility");
                        }
                        if !value.wifi {
                            missing.push(if value.wifi_pending {
                                "Wi-Fi optimization approval"
                            } else if value.wifi_installing {
                                "Wi-Fi optimization installation"
                            } else if value.wifi_setup_failed {
                                "Wi-Fi optimization setup or helper readiness"
                            } else {
                                "Wi-Fi optimization"
                            });
                        }
                        format!("{} is not ready. Check Permissions, then retry Allow connections.", missing.join(" and "))
                    }
                };
                inner.receiving_paused(format!("Incoming connections paused. {reason}"));
                if let Some(job) = inner.job.as_mut().filter(|job| {
                    job.view.kind == SessionKind::Incoming && job.view.phase == Phase::Connected
                }) {
                    // Revoke this exact job while its identity is still locked.
                    job.cancelled.store(true, Ordering::SeqCst);
                    job.view.advance(Phase::Disconnecting);
                    if let Some(socket) = &job.socket {
                        let _ = socket.shutdown(Shutdown::Both);
                    }
                    true
                } else {
                    false
                }
            } else if !value.can_receive() {
                if let Some(job) = inner.job.as_mut().filter(|job| {
                    job.view.kind == SessionKind::Incoming && job.view.route != Some("usb")
                }) {
                    job.cancelled.store(true, Ordering::SeqCst);
                    job.view.advance(Phase::Disconnecting);
                    if let Some(socket) = &job.socket {
                        let _ = socket.shutdown(Shutdown::Both);
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        drop(generation);
        if cancel {
            self.approvals.cancel();
        }
        result
    }
    pub fn clear_message(&self) {
        let mut i = self.inner.lock().unwrap();
        i.permission_request = None;
        i.error = None;
    }
    pub fn save_device(&self, id: &str, device: Device) -> Result<()> {
        peers::validate(&device)?;
        ensure!(
            self.store()?.peer(id)?.is_some(),
            "Pair with this device first."
        );
        let mut devices = self.devices.lock().unwrap();
        let mut next = devices.clone();
        next.insert(id.into(), device);
        peers::save(&self.root, &next)?;
        *devices = next;
        drop(devices);
        self.clear_removal(id)?;
        Ok(())
    }
    fn reserve(&self, kind: SessionKind, peer: Option<String>) -> Result<(u64, Arc<AtomicBool>)> {
        let mut i = self.inner.lock().unwrap();
        ensure!(
            !self.closing.load(Ordering::SeqCst) && !self.update_pending.load(Ordering::SeqCst),
            "extend.computer is preparing an update. Try again after it restarts."
        );
        ensure!(i.job.is_none(), "Disconnect the current session first.");
        i.advertisement = None;
        i.next += 1;
        let id = i.next;
        let cancelled = Arc::new(AtomicBool::new(false));
        i.error = None;
        i.job = Some(Job {
            view: Session {
                id,
                kind,
                phase: Phase::Connecting,
                peer,
                route: None,
                reconnecting: false,
                display: None,
                display_config: None,
            },
            cancelled: cancelled.clone(),
            ended_locally: false,
            socket: None,
            display_updates: None,
        });
        Ok((id, cancelled))
    }
    fn set_socket(&self, id: u64, socket: &TcpStream) -> Result<()> {
        self.set_socket_route(id, socket, None)
    }
    fn set_socket_route(
        &self,
        id: u64,
        socket: &TcpStream,
        route: Option<&'static str>,
    ) -> Result<()> {
        let cable_route = if route.is_none() {
            wired::socket_route(socket)
        } else {
            None
        };
        let mut i = self.inner.lock().unwrap();
        let j = i.job.as_mut().context("Session cancelled")?;
        ensure!(
            j.view.id == id && !j.cancelled.load(Ordering::SeqCst),
            "Session cancelled"
        );
        j.socket = Some(socket.try_clone()?);
        if matches!(j.view.kind, SessionKind::Incoming | SessionKind::Outgoing) {
            j.view.route = Some(match route.or(cable_route) {
                Some(route) => route,
                None if socket.peer_addr()?.ip().is_loopback() => "internet",
                None => "local",
            });
            eprintln!("Control transport: {}", j.view.route.unwrap());
        }
        Ok(())
    }
    fn stage(&self, id: u64, phase: Phase, peer: Option<&str>) {
        if let Some(j) = self
            .inner
            .lock()
            .unwrap()
            .job
            .as_mut()
            .filter(|j| j.view.id == id && !j.cancelled.load(Ordering::SeqCst))
        {
            if !j.view.advance(phase) {
                return;
            }
            if phase == Phase::Connected {
                j.view.reconnecting = false;
            }
            if let Some(peer) = peer {
                j.view.peer = Some(peer.into());
            }
        }
    }
    fn prepare_reconnect(&self, id: u64) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(job) = inner.job.as_mut().filter(|job| {
            job.view.id == id
                && job.view.kind == SessionKind::Outgoing
                && !job.cancelled.load(Ordering::SeqCst)
        }) else {
            return false;
        };
        job.view.reconnecting |= job.view.phase == Phase::Connected;
        if !job.view.advance(Phase::Connecting) {
            return false;
        }
        job.view.route = None;
        if let Some(socket) = job.socket.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        true
    }
    fn finish(&self, id: u64, result: Result<()>) {
        if let Err(error) = &result {
            eprintln!("Connection job {id} ended: {error:#}");
        }
        #[cfg(any(test, debug_assertions))]
        eprintln!(
            "job {id} finished: {}",
            result
                .as_ref()
                .map(|_| "ok".into())
                .unwrap_or_else(|e| format!("{e:#}"))
        );
        self.approvals.cancel();
        let mut i = self.inner.lock().unwrap();
        if i.job.as_ref().is_some_and(|j| j.view.id == id) {
            let job = i.job.take().unwrap();
            if !job.cancelled.load(Ordering::SeqCst) {
                match result {
                    Ok(()) => {}
                    Err(e) => {
                        if e.is::<extend_computer_agent::low_jitter::PermissionRequired>() {
                            i.permission_request = Some(
                                if job.view.kind == SessionKind::Incoming {
                                    "receive"
                                } else {
                                    "share"
                                }
                                .into(),
                            );
                        }
                        if !(job.view.kind == SessionKind::Incoming
                            && job.view.phase == Phase::Connected
                            && is_connection_error(&e))
                        {
                            let message = friendly_error(&e);
                            i.error_peer =
                                job.view.peer.clone().map(|peer| (peer, message.clone()));
                            i.error = Some(message);
                        }
                    }
                }
            }
        }
    }
    fn cancelled(&self, id: u64, cancelled: &AtomicBool) -> bool {
        if cancelled.load(Ordering::SeqCst) {
            return true;
        }
        let inner = self.inner.lock().unwrap();
        let Some(job) = inner.job.as_ref().filter(|j| j.view.id == id) else {
            return true;
        };
        #[cfg(unix)]
        if let Some(socket) = &job.socket {
            use std::os::fd::AsRawFd;
            let mut byte = 0u8;
            // Peek never consumes protocol bytes or changes the socket's blocking flags/timeouts.
            let count = unsafe {
                libc::recv(
                    socket.as_raw_fd(),
                    (&mut byte as *mut u8).cast(),
                    1,
                    libc::MSG_PEEK | libc::MSG_DONTWAIT,
                )
            };
            if count == 0 {
                return true;
            }
            if count < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock
            {
                return true;
            }
        }
        false
    }
    pub fn disconnect(&self) {
        {
            let mut i = self.inner.lock().unwrap();
            if let Some(j) = i.job.as_mut() {
                j.cancelled.store(true, Ordering::SeqCst);
                j.view.advance(Phase::Disconnecting);
                if let Some(s) = &j.socket {
                    let _ = s.shutdown(Shutdown::Both);
                }
            }
        }
        self.approvals.cancel();
    }
    /// The user's Disconnect. A receiving session ends at the sharing device's
    /// next message (at most 500 ms apart) and tells it not to reconnect;
    /// other stops (receiving turned off, quitting) stay recoverable.
    pub fn end_session(self: &Arc<Self>) {
        let id = {
            let mut i = self.inner.lock().unwrap();
            match i
                .job
                .as_mut()
                .filter(|j| j.view.kind == SessionKind::Incoming && j.socket.is_some())
            {
                Some(j) => {
                    j.ended_locally = true;
                    j.view.advance(Phase::Disconnecting);
                    j.view.id
                }
                None => {
                    drop(i);
                    return self.disconnect();
                }
            }
        };
        self.approvals.cancel();
        // A silent or stalled peer still ends here, without the notice.
        let app = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            if app.ended_locally(id) {
                app.disconnect();
            }
        });
    }
    fn ended_locally(&self, id: u64) -> bool {
        self.inner
            .lock()
            .unwrap()
            .job
            .as_ref()
            .is_some_and(|j| j.view.id == id && j.ended_locally)
    }
    pub fn stop_receiving(&self) -> Result<()> {
        let mut preference = self.receiving_preference.lock().unwrap();
        self.stop_listener();
        preferences::save(&self.root, false)?;
        *preference = false;
        Ok(())
    }
    fn stop_listener(&self) {
        {
            let mut i = self.inner.lock().unwrap();
            if let Some(stop) = &i.listener {
                stop.store(true, Ordering::SeqCst);
            }
            i.receiving_enabled = false;
            i.code = None;
            i.pairing_open = false;
            i.advertisement = None;
        }
        self.disconnect();
    }
    pub fn prepare_update(&self) -> bool {
        // Reserve and update installation share this lock, preventing a new
        // session from starting between the idle check and application exit.
        let inner = self.inner.lock().unwrap();
        if inner.job.is_some() {
            return false;
        }
        self.update_pending.store(true, Ordering::SeqCst);
        true
    }
    pub fn cancel_update(&self) {
        self.update_pending.store(false, Ordering::SeqCst);
    }
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.disconnect_usb();
        let _preference = self.receiving_preference.lock().unwrap();
        self.stop_listener();
    }
}
