//! Cancellable foreground visual pairing. The shared engine commits trust only
//! after both users approve the same transcript-bound ordered symbols.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{identity::Identity, session, trust::TrustStore};
use std::{
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
static IDS: AtomicU64 = AtomicU64::new(1);
struct Request {
    id: String,
    peer: String,
    symbols: [u8; 8],
    decision: Option<bool>,
}
struct State {
    phase: &'static str,
    request: Option<Request>,
    socket: Option<TcpStream>,
    peer: Option<String>,
    authenticated_name: Option<String>,
    metadata_deadline: Option<Instant>,
    error: String,
    active: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    expires: Instant,
}
pub struct VisualPairing {
    pub id: String,
    address: SocketAddr,
    name: String,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    timer: Option<JoinHandle<()>>,
}
impl VisualPairing {
    pub fn start(
        identity: Arc<Identity>,
        trust: TrustStore,
        address: SocketAddr,
        name: String,
        local_name: String,
        names: Arc<crate::names::Names>,
        ttl: Duration,
    ) -> Result<Self> {
        session::validate_device_name(&name)?;
        session::validate_device_name(&local_name)?;
        ensure!(
            !ttl.is_zero() && ttl <= Duration::from_secs(120),
            "Invalid pairing lifetime"
        );
        let id = IDS.fetch_add(1, Ordering::Relaxed).to_string();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phase: "connecting",
                request: None,
                socket: None,
                peer: None,
                authenticated_name: None,
                metadata_deadline: None,
                error: String::new(),
                active: true,
            }),
            changed: Condvar::new(),
            expires: Instant::now() + ttl,
        });
        let active = shared.clone();
        let worker = thread::Builder::new().name("extend-visual-pair".into()).spawn(move || {
            let result = (|| -> Result<String> {
                // Numeric addresses only: no detached DNS work survives cancellation.
                // Short bounded attempts let stop interrupt the connection phase.
                let connect_deadline = active.expires.min(Instant::now() + Duration::from_secs(5));
                let socket = loop {
                    ensure!(active.state.lock().unwrap().active, "Pairing cancelled");
                    let remaining = connect_deadline.saturating_duration_since(Instant::now());
                    ensure!(!remaining.is_zero(), "Could not reach this device");
                    match TcpStream::connect_timeout(&address, remaining.min(Duration::from_millis(200))) {
                        Ok(socket) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
                        Err(error) => return Err(error).context("Could not reach this device"),
                    }
                };
                {
                    let mut state = active.state.lock().unwrap();
                    ensure!(state.active && Instant::now() < active.expires, "Pairing cancelled");
                    state.socket = Some(socket.try_clone()?);
                }
                let peer = session::pair_visually(socket, &identity, &trust, |peer, symbols| {
                    if !crate::visual_pairing_idle() { return false; }
                    let mut state = active.state.lock().unwrap();
                    if !state.active || Instant::now() >= active.expires { return false; }
                    state.phase = "verify";
                    state.request = Some(Request { id: IDS.fetch_add(1, Ordering::Relaxed).to_string(),
                        peer: peer.to_owned(), symbols: *symbols, decision: None });
                    loop {
                        if !state.active || Instant::now() >= active.expires { return false; }
                        match state.request.as_ref().and_then(|r| r.decision) {
                            Some(allow) => { drop(state); return allow && crate::visual_pairing_idle(); },
                            None => state = active.changed.wait_timeout(state,
                                active.expires.saturating_duration_since(Instant::now())).unwrap().0,
                        }
                    }
                })?;
                // Route hints are recorded only after authenticated mutual consent.
                // A metadata failure must not hide an already committed pairing.
                let _ = names.save_host(&peer, address.ip());
                {
                    let mut state = active.state.lock().unwrap();
                    // From here cancellation/metadata errors cannot undo mutual trust.
                    state.peer = Some(peer.clone());
                    if !state.active { state.phase = "paired"; }
                    state.request = None;
                    state.authenticated_name = names.snapshot().ok()
                        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                        .and_then(|json| json[&peer].as_str().map(str::to_owned));
                }
                let metadata = (|| -> Result<()> {
                    let deadline = active.expires.min(Instant::now() + Duration::from_secs(3));
                    ensure!(active.state.lock().unwrap().active, "Pairing cancelled");
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    ensure!(!remaining.is_zero(), "Metadata exchange expired");
                    let socket = TcpStream::connect_timeout(&address, remaining.min(Duration::from_millis(200)))?;
                    {
                        let mut state = active.state.lock().unwrap();
                        ensure!(state.active && Instant::now() < deadline, "Pairing cancelled");
                        state.socket = Some(socket.try_clone()?);
                        state.metadata_deadline = Some(deadline);
                        active.changed.notify_all();
                    }
                    // The fresh channel pins the identity approved above. Its timer
                    // shuts down the captured socket even if handshake I/O is stalled.
                    let remote_name = session::exchange_device_name(socket, &identity, &peer, &local_name)?;
                    ensure!(trust.peer(&peer)?.is_some(), "Device was forgotten");
                    names.save(&peer, &remote_name)?;
                    active.state.lock().unwrap().authenticated_name = Some(remote_name);
                    Ok(())
                })();
                let _ = metadata; // Optional metadata failure never changes durable trust.
                Ok(peer)
            })();
            let mut state = active.state.lock().unwrap();
            state.socket = None;
            state.metadata_deadline = None;
            state.request = None;
            if state.active {
                match result {
                    Ok(peer) => { state.peer = Some(peer); state.phase = "paired"; },
                    Err(error) => {
                        state.phase = if Instant::now() >= active.expires { "expired" }
                        else if matches!(error.downcast_ref::<extend_computer_agent::error::EngineError>(),
                            Some(extend_computer_agent::error::EngineError::LocalConsentDenied | extend_computer_agent::error::EngineError::RemoteConsentDenied)) {
                            "rejected"
                        } else { "failed" };
                        state.error = error.to_string();
                    },
                }
                state.active = false;
            }
            active.changed.notify_all();
        })?;
        let clock = shared.clone();
        let timer = thread::Builder::new()
            .name("extend-visual-expiry".into())
            .spawn(move || {
                let mut state = clock.state.lock().unwrap();
                while state.active {
                    let now = Instant::now();
                    if state
                        .metadata_deadline
                        .is_some_and(|deadline| now >= deadline)
                    {
                        state.metadata_deadline = None;
                        if let Some(socket) = &state.socket {
                            let _ = socket.shutdown(Shutdown::Both);
                        }
                    }
                    let remaining = state
                        .metadata_deadline
                        .unwrap_or(clock.expires)
                        .min(clock.expires)
                        .saturating_duration_since(now);
                    if remaining.is_zero() {
                        state.active = false;
                        state.phase = if state.peer.is_some() {
                            "paired"
                        } else {
                            "expired"
                        };
                        state.request = None;
                        if let Some(socket) = &state.socket {
                            let _ = socket.shutdown(Shutdown::Both);
                        }
                        clock.changed.notify_all();
                        break;
                    }
                    state = clock.changed.wait_timeout(state, remaining).unwrap().0;
                }
            });
        let mut result = Self {
            id,
            address,
            name,
            shared,
            worker: Some(worker),
            timer: None,
        };
        match timer {
            Ok(timer) => result.timer = Some(timer),
            Err(error) => {
                result.stop();
                return Err(error.into());
            }
        }
        Ok(result)
    }
    pub fn active(&self) -> bool {
        self.shared.state.lock().unwrap().active
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.shared.state.lock().unwrap();
        serde_json::json!({"id":self.id,"state":state.phase,"peer":state.peer,
            "name":state.authenticated_name.clone().unwrap_or_else(|| state.peer.as_ref()
                .map(|peer| format!("Device {}", &peer[..8])).unwrap_or_else(|| "Device".into())),
            "suggestedName":self.name,"address":self.address.to_string(),"error":state.error,
            "secondsRemaining":self.shared.expires.saturating_duration_since(Instant::now()).as_secs(),
            "request":state.request.as_ref().map(|r| serde_json::json!({"id":r.id,"peer":r.peer,
                "kind":"verify","symbols":r.symbols}))})
    }
    pub fn respond(&self, request: &str, allow: bool) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        ensure!(
            state.active && Instant::now() < self.shared.expires,
            "Pairing closed"
        );
        let pending = state.request.as_mut().context("Pairing request expired")?;
        ensure!(
            pending.id == request && pending.decision.is_none(),
            "Pairing request changed"
        );
        pending.decision = Some(allow);
        self.shared.changed.notify_all();
        Ok(())
    }
    pub fn stop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.active {
                state.phase = if state.peer.is_some() {
                    "paired"
                } else {
                    "closed"
                };
            }
            state.active = false;
            state.request = None;
            if let Some(socket) = &state.socket {
                let _ = socket.shutdown(Shutdown::Both);
            }
            self.shared.changed.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}
impl Drop for VisualPairing {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::incoming::Incoming;
    fn wait(mut f: impl FnMut() -> Option<serde_json::Value>) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = f() {
                return value;
            }
            assert!(Instant::now() < deadline, "Pairing state did not arrive");
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn both_directions_show_same_symbols_and_require_both_approvals() {
        for (a, b) in [(true, true), (false, true), (true, false)] {
            let files = tempfile::tempdir().unwrap();
            let left = Arc::new(Identity::generate());
            let right = Arc::new(Identity::generate());
            let left_trust = TrustStore::open(files.path().join("left")).unwrap();
            let right_trust = TrustStore::open(files.path().join("right")).unwrap();
            let right_names =
                Arc::new(crate::names::Names::open(&files.path().join("right")).unwrap());
            let mut incoming = Incoming::named(
                right.clone(),
                right_trust.clone(),
                Some(Duration::from_secs(10)),
                "Verified Desktop".into(),
                right_names.clone(),
            )
            .unwrap();
            let port = incoming.snapshot()["port"].as_u64().unwrap() as u16;
            let mut outgoing = VisualPairing::start(
                left.clone(),
                left_trust.clone(),
                SocketAddr::from(([127, 0, 0, 1], port)),
                "Desktop".into(),
                "Tablet".into(),
                Arc::new(crate::names::Names::open(files.path()).unwrap()),
                Duration::from_secs(10),
            )
            .unwrap();
            let remote = wait(|| {
                incoming.snapshot()["request"]
                    .as_object()
                    .map(|_| incoming.snapshot()["request"].clone())
            });
            let local = wait(|| {
                outgoing.snapshot()["request"]
                    .as_object()
                    .map(|_| outgoing.snapshot()["request"].clone())
            });
            assert_eq!(remote["kind"], "verify");
            assert_eq!(local["symbols"], remote["symbols"]);
            assert_eq!(local["symbols"].as_array().unwrap().len(), 8);
            assert!(left_trust.peer(&right.fingerprint()).unwrap().is_none());
            assert!(right_trust.peer(&left.fingerprint()).unwrap().is_none());
            assert!(outgoing.respond("stale", true).is_err());
            assert!(incoming.respond("stale", true).is_err());
            outgoing.respond(local["id"].as_str().unwrap(), a).unwrap();
            assert!(outgoing.respond(local["id"].as_str().unwrap(), a).is_err());
            incoming.respond(remote["id"].as_str().unwrap(), b).unwrap();
            let final_state = wait(|| (!outgoing.active()).then(|| outgoing.snapshot()));
            if a && b {
                assert_eq!(final_state["name"], "Verified Desktop");
                let names: serde_json::Value =
                    serde_json::from_str(&right_names.snapshot().unwrap()).unwrap();
                assert_eq!(names[&left.fingerprint()], "Tablet");
                assert_eq!(final_state["state"], "paired");
            } else {
                assert!(matches!(
                    final_state["state"].as_str(),
                    Some("rejected" | "failed")
                ));
            }
            wait(|| (incoming.snapshot()["request"].is_null()).then(|| incoming.snapshot()));
            assert_eq!(
                left_trust.peer(&right.fingerprint()).unwrap().is_some(),
                a && b
            );
            assert_eq!(
                right_trust.peer(&left.fingerprint()).unwrap().is_some(),
                a && b
            );
            outgoing.stop();
            incoming.stop();
        }
    }
    #[test]
    fn metadata_timeout_and_cancel_preserve_pairing_without_promoting_hint() {
        use extend_computer_agent::pairing::PairingWindow;
        use std::net::TcpListener;
        use std::sync::mpsc;
        struct Sink;
        impl session::CursorSink for Sink {
            fn start_cursor(&mut self, _: &str) -> Result<bool> {
                Ok(false)
            }
            fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
                anyhow::bail!("No input in pairing fixture")
            }
        }
        for cancel in [false, true] {
            let files = tempfile::tempdir().unwrap();
            let left = Arc::new(Identity::generate());
            let right = Arc::new(Identity::generate());
            let trust = TrustStore::open(files.path().join("left")).unwrap();
            let remote_trust = TrustStore::open(files.path().join("right")).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (connected, ready) = mpsc::channel();
            let (release, done) = mpsc::channel();
            let server_identity = right.clone();
            let server_trust = remote_trust.clone();
            let server = thread::spawn(move || {
                session::serve_connection_with_verification(
                    listener.accept().unwrap().0,
                    &server_identity,
                    &server_trust,
                    &mut Some(PairingWindow::new(Duration::from_secs(10))),
                    |_, _| session::Decision::Deny,
                    &mut Sink,
                    |_, _| true,
                )
                .unwrap();
                let _socket = listener.accept().unwrap().0;
                connected.send(()).unwrap();
                done.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            let mut operation = VisualPairing::start(
                left.clone(),
                trust.clone(),
                address,
                "Unverified discovery label".into(),
                "Tablet".into(),
                Arc::new(crate::names::Names::open(files.path()).unwrap()),
                Duration::from_secs(10),
            )
            .unwrap();
            let request = wait(|| {
                operation.snapshot()["request"]
                    .as_object()
                    .map(|_| operation.snapshot()["request"].clone())
            });
            operation
                .respond(request["id"].as_str().unwrap(), true)
                .unwrap();
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let before = Instant::now();
            if cancel {
                operation.stop();
            } else {
                wait(|| (!operation.active()).then(|| operation.snapshot()));
            }
            assert!(
                before.elapsed()
                    < if cancel {
                        Duration::from_secs(1)
                    } else {
                        Duration::from_secs(4)
                    }
            );
            let state = operation.snapshot();
            assert_eq!(state["state"], "paired");
            assert_eq!(
                state["name"],
                format!("Device {}", &right.fingerprint()[..8])
            );
            assert!(trust.peer(&right.fingerprint()).unwrap().is_some());
            assert!(remote_trust.peer(&left.fingerprint()).unwrap().is_some());
            release.send(()).unwrap();
            server.join().unwrap();
            operation.stop();
        }
    }
    #[test]
    fn cancellation_interrupts_handshake_before_symbols() {
        use std::net::TcpListener;
        use std::sync::mpsc;
        let files = tempfile::tempdir().unwrap();
        let trust = TrustStore::open(files.path().join("trust")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted, ready) = mpsc::channel();
        let (release, done) = mpsc::channel();
        let server = thread::spawn(move || {
            let _socket = listener.accept().unwrap().0;
            accepted.send(()).unwrap();
            done.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let mut operation = VisualPairing::start(
            Arc::new(Identity::generate()),
            trust.clone(),
            address,
            "Desktop".into(),
            "Tablet".into(),
            Arc::new(crate::names::Names::open(files.path()).unwrap()),
            Duration::from_secs(10),
        )
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = Instant::now();
        operation.stop();
        assert!(before.elapsed() < Duration::from_secs(1));
        assert_eq!(operation.snapshot()["state"], "closed");
        assert!(operation.snapshot()["request"].is_null());
        assert!(trust.load().unwrap().peers.is_empty());
        release.send(()).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn cancel_and_expiry_close_pending_handshake_without_trust() {
        for expire in [false, true] {
            let files = tempfile::tempdir().unwrap();
            let left = Arc::new(Identity::generate());
            let right = Arc::new(Identity::generate());
            let left_trust = TrustStore::open(files.path().join("left")).unwrap();
            let right_trust = TrustStore::open(files.path().join("right")).unwrap();
            let mut incoming =
                Incoming::start(right.clone(), right_trust.clone(), Duration::from_secs(10))
                    .unwrap();
            let port = incoming.snapshot()["port"].as_u64().unwrap() as u16;
            let mut outgoing = VisualPairing::start(
                left.clone(),
                left_trust.clone(),
                SocketAddr::from(([127, 0, 0, 1], port)),
                "Desktop".into(),
                "Tablet".into(),
                Arc::new(crate::names::Names::open(files.path()).unwrap()),
                if expire {
                    Duration::from_millis(300)
                } else {
                    Duration::from_secs(10)
                },
            )
            .unwrap();
            let request = wait(|| {
                outgoing.snapshot()["request"]
                    .as_object()
                    .map(|_| outgoing.snapshot()["request"].clone())
            });
            if expire {
                wait(|| (!outgoing.active()).then(|| outgoing.snapshot()));
            } else {
                outgoing.stop();
            }
            assert!(outgoing
                .respond(request["id"].as_str().unwrap(), true)
                .is_err());
            assert_eq!(
                outgoing.snapshot()["state"],
                if expire { "expired" } else { "closed" }
            );
            incoming.stop();
            outgoing.stop();
            assert!(left_trust.peer(&right.fingerprint()).unwrap().is_none());
            assert!(right_trust.peer(&left.fingerprint()).unwrap().is_none());
        }
    }
}
