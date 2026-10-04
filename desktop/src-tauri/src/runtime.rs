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
mod device_names;
mod errors;
mod incoming;
mod outgoing;
mod peer_health;
mod preferences;
mod presence;
#[cfg(test)]
mod tests;
mod unpair;
pub(crate) use errors::friendly_error;
use errors::is_connection_error;
mod state;
pub use state::Session;
use state::{Phase, SessionKind};

struct Job {
    view: Session,
    cancelled: Arc<AtomicBool>,
    socket: Option<TcpStream>,
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
    permissions: Permissions,
    permission_request: Option<String>,
    notification: Option<Notification>,
    notification_sequence: u64,
}
#[derive(Serialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub address: String,
    pub edge: String,
    pub availability: presence::Availability,
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
    health: Mutex<BTreeMap<String, presence::Health>>,
    presence_addresses: Mutex<BTreeMap<String, SocketAddr>>,
    inner: Mutex<Inner>,
    removals: Mutex<BTreeMap<String, String>>,
    closing: AtomicBool,
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
            root,
            discovery_id: Identity::generate().fingerprint(),
            helper,
            identity: Mutex::new(None),
            devices: Mutex::new(devices),
            health: Mutex::new(BTreeMap::new()),
            presence_addresses: Mutex::new(BTreeMap::new()),
            approvals: Approvals::default(),
            removals: Mutex::new(removals),
            closing: AtomicBool::new(false),
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
        let records = self.store()?.load()?;
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
            .map(|(id, _)| {
                let d = devices.get(&id);
                Peer {
                    name: d
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| format!("Device {}", &id[..8.min(id.len())])),
                    address: d.map(|d| d.address.clone()).unwrap_or_default(),
                    edge: d.map(|d| d.edge.clone()).unwrap_or_else(|| "left".into()),
                    availability: self
                        .health
                        .lock()
                        .unwrap()
                        .get(&id)
                        .map(|h| h.state())
                        .unwrap_or(presence::Availability::Checking),
                    id,
                }
            })
            .collect();
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
            permissions: inner.permissions.clone(),
            permission_request: inner.permission_request.clone(),
        })
    }
    pub fn refresh_permissions(&self) -> Result<Permissions> {
        let result = native::permissions(&self.helper);
        let value = result.as_ref().cloned().unwrap_or_default();
        let cancel = {
            let mut inner = self.inner.lock().unwrap();
            inner.permissions = value.clone();
            if inner.receiving_enabled && !value.can_receive() {
                inner.receiving_enabled = false;
                inner.notification_sequence += 1;
                inner.notification = Some(Notification {
                    id: inner.notification_sequence,
                    message: "Incoming connections paused. Allow Accessibility and Wi-Fi optimization to enable them again.".into(),
                });
                inner.job.as_ref().is_some_and(|job| {
                    job.view.kind == SessionKind::Incoming && job.view.phase == Phase::Connected
                })
            } else {
                false
            }
        };
        if cancel {
            self.disconnect();
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
        ensure!(!self.closing.load(Ordering::SeqCst) && !self.update_pending.load(Ordering::SeqCst), "extend.computer is preparing an update. Try again after it restarts.");
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
            },
            cancelled: cancelled.clone(),
            socket: None,
        });
        Ok((id, cancelled))
    }
    fn set_socket(&self, id: u64, socket: &TcpStream) -> Result<()> {
        let mut i = self.inner.lock().unwrap();
        let j = i.job.as_mut().context("Session cancelled")?;
        ensure!(
            j.view.id == id && !j.cancelled.load(Ordering::SeqCst),
            "Session cancelled"
        );
        j.socket = Some(socket.try_clone()?);
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
        if !job.view.advance(Phase::Connecting) {
            return false;
        }
        if let Some(socket) = job.socket.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        true
    }
    fn finish(&self, id: u64, result: Result<()>) {
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
                            i.error = Some(friendly_error(&e));
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
        if inner.job.is_some() { return false; }
        self.update_pending.store(true, Ordering::SeqCst);
        true
    }
    pub fn cancel_update(&self) {
        self.update_pending.store(false, Ordering::SeqCst);
    }
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let _preference = self.receiving_preference.lock().unwrap();
        self.stop_listener();
    }
}
