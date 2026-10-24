//! Foreground pairing listener. Unknown peers always wait for explicit local consent.
use anyhow::{ensure, Result};
use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{self, Decision},
    trust::TrustStore,
};
use std::{
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

static REQUESTS: AtomicU64 = AtomicU64::new(1);
struct Request {
    id: u64,
    peer: String,
    kind: &'static str,
    symbols: Option<[u8; 8]>,
    decision: Option<bool>,
}
struct State {
    running: bool,
    code: Option<Zeroizing<String>>,
    request: Option<Request>,
    socket: Option<TcpStream>,
    outcome: &'static str,
    input_lease: Option<(String, u64)>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    expires: Option<Instant>,
    port: u16,
    input: Option<Arc<crate::receiver::Receiver>>,
}
pub struct Incoming {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    timer: Option<JoinHandle<()>>,
}

impl Incoming {
    #[cfg(test)]
    pub fn start(identity: Arc<Identity>, trust: TrustStore, ttl: Duration) -> Result<Self> {
        Self::open(
            identity,
            trust,
            Some(ttl),
            "extend.computer".into(),
            None,
            None,
        )
    }
    pub fn named(
        identity: Arc<Identity>,
        trust: TrustStore,
        ttl: Option<Duration>,
        name: String,
        names: Arc<crate::names::Names>,
    ) -> Result<Self> {
        session::validate_device_name(&name)?;
        Self::open(identity, trust, ttl, name, Some(names), None)
    }
    pub fn receiving(
        identity: Arc<Identity>,
        trust: TrustStore,
        name: String,
        names: Arc<crate::names::Names>,
        input: Arc<crate::receiver::Receiver>,
    ) -> Result<Self> {
        session::validate_device_name(&name)?;
        Self::open(identity, trust, None, name, Some(names), Some(input))
    }
    fn open(
        identity: Arc<Identity>,
        trust: TrustStore,
        ttl: Option<Duration>,
        name: String,
        names: Option<Arc<crate::names::Names>>,
        input: Option<Arc<crate::receiver::Receiver>>,
    ) -> Result<Self> {
        // A user-enabled receiver is an admission gate, not the health of its
        // optional LAN listener. Private USB streams remain available if bind fails.
        let listener = match TcpListener::bind("0.0.0.0:0") {
            Ok(listener) => Some(listener),
            Err(_) if ttl.is_none() && input.is_some() => None,
            Err(error) => return Err(error.into()),
        };
        let mut window = ttl.map(PairingWindow::new);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                running: true,
                code: window
                    .as_ref()
                    .and_then(|w| w.code())
                    .map(|s| Zeroizing::new(s.to_owned())),
                request: None,
                socket: None,
                outcome: "waiting",
                input_lease: None,
            }),
            changed: Condvar::new(),
            expires: ttl.map(|ttl| Instant::now() + ttl),
            port: listener
                .as_ref()
                .map(|listener| listener.local_addr().map(|a| a.port()))
                .transpose()?
                .unwrap_or(0),
            input: input.clone(),
        });
        let Some(listener) = listener else {
            return Ok(Self {
                shared,
                worker: None,
                timer: None,
            });
        };
        let active = shared.clone();
        let worker = thread::Builder::new()
            .name("extend-pairing".into())
            .spawn(move || {
                let mut sink = NameSink {
                    name,
                    names,
                    host: None,
                    input,
                    input_id: None,
                    shared: active.clone(),
                    listener_gate: None,
                };
                loop {
                    {
                        let mut state = active.state.lock().unwrap();
                        if !state.running {
                            break;
                        }
                        if active
                            .expires
                            .is_some_and(|expires| Instant::now() >= expires)
                        {
                            state.running = false;
                            state.code = None;
                            state.outcome = "expired";
                            break;
                        }
                    }
                    let socket = match listener.accept() {
                        Ok((socket, address)) => {
                            // A private relay/USB bridge is not a LAN address hint.
                            sink.host = (!address.ip().is_loopback() || sink.input.is_none())
                                .then_some(address.ip());
                            socket
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => {
                            let mut state = active.state.lock().unwrap();
                            if active.input.is_none() {
                                state.running = false;
                                state.outcome = "failed";
                            }
                            break;
                        }
                    };
                    if socket.set_nonblocking(false).is_err() {
                        continue;
                    }
                    {
                        let mut state = active.state.lock().unwrap();
                        if !state.running {
                            break;
                        }
                        if state.socket.is_some() {
                            continue;
                        }
                        match socket.try_clone() {
                            Ok(clone) => state.socket = Some(clone),
                            Err(_) => continue,
                        }
                    }
                    let requested_peer = std::cell::RefCell::new(None);
                    let previously_trusted = std::cell::Cell::new(false);
                    let had_code = window.as_ref().and_then(|w| w.code()).is_some();
                    let result = session::serve_connection_with_verification(
                        socket,
                        &identity,
                        &trust,
                        &mut window,
                        |peer, _| {
                            *requested_peer.borrow_mut() = Some(peer.to_owned());
                            previously_trusted.set(trust.peer(peer).ok().flatten().is_some());
                            if crate::visual_pairing_idle()
                                && await_approval(&active, peer, None)
                                && crate::visual_pairing_idle()
                            {
                                Decision::Remember
                            } else {
                                Decision::Deny
                            }
                        },
                        &mut sink,
                        |peer, symbols| {
                            *requested_peer.borrow_mut() = Some(peer.to_owned());
                            previously_trusted.set(trust.peer(peer).ok().flatten().is_some());
                            crate::visual_pairing_idle()
                                && await_approval(&active, peer, Some(*symbols))
                                && crate::visual_pairing_idle()
                        },
                    );
                    let requested_peer = requested_peer.into_inner();
                    let mut state = active.state.lock().unwrap();
                    state.socket = None;
                    state.request = None;
                    if state.running && had_code && window.as_ref().and_then(|w| w.code()).is_none()
                    {
                        state.code = None;
                        // A peer can close after trust is committed. Report the durable result.
                        state.outcome = if result.is_ok()
                            || (!previously_trusted.get()
                                && requested_peer
                                    .as_ref()
                                    .is_some_and(|peer| trust.peer(peer).ok().flatten().is_some()))
                        {
                            "paired"
                        } else {
                            "rejected"
                        };
                    } else if state.running && had_code && result.is_err() {
                        state.outcome = "waiting";
                    }
                }
                let mut state = active.state.lock().unwrap();
                state.code = None;
                state.request = None;
                if active.input.is_none() {
                    state.socket = None;
                }
            })?;
        let Some(expires) = shared.expires else {
            return Ok(Self {
                shared,
                worker: Some(worker),
                timer: None,
            });
        };
        let clock = shared.clone();
        let timer = thread::Builder::new()
            .name("extend-pairing-expiry".into())
            .spawn(move || {
                let mut state = clock.state.lock().unwrap();
                while state.running {
                    let remaining = expires.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        let wake = state.socket.is_none();
                        state.running = false;
                        state.code = None;
                        state.request = None;
                        state.outcome = "expired";
                        if let Some(socket) = &state.socket {
                            let _ = socket.shutdown(Shutdown::Both);
                        }
                        clock.changed.notify_all();
                        drop(state);
                        if wake {
                            wake_accept(clock.port);
                        }
                        return;
                    }
                    state = clock.changed.wait_timeout(state, remaining).unwrap().0;
                }
            });
        match timer {
            Ok(timer) => Ok(Self {
                shared,
                worker: Some(worker),
                timer: Some(timer),
            }),
            Err(error) => {
                let mut stopped = Self {
                    shared,
                    worker: Some(worker),
                    timer: None,
                };
                stopped.stop();
                Err(error.into())
            }
        }
    }
    /// Reserve the same admission slot used by LAN. Private carriers cannot
    /// replace a live socket or establish trust/name/route side effects before
    /// proving the exact expected Noise identity.
    pub fn accept_pinned(
        &self,
        socket: TcpStream,
        identity: Arc<Identity>,
        trust: TrustStore,
        names: Arc<crate::names::Names>,
        expected: &str,
    ) -> Result<()> {
        let input = self
            .shared
            .input
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Input unavailable"))?;
        {
            let mut state = self.shared.state.lock().unwrap();
            ensure!(state.running, "Receiving is off");
            ensure!(state.socket.is_none(), "Receiving is busy");
            state.socket = Some(socket.try_clone()?);
        }
        let active = self.shared.clone();
        let mut sink = NameSink {
            input: Some(input),
            input_id: None,
            shared: active.clone(),
            listener_gate: None,
            host: None,
            name: names.local("Android device"),
            names: Some(names),
        };
        let expected = expected.to_owned();
        if let Err(error) = thread::Builder::new()
            .name("extend-pinned-input".into())
            .spawn(move || {
                let _ = session::serve_connection_with_cursor_pinned(
                    socket, &identity, &trust, &expected, &mut sink,
                );
                let mut state = active.state.lock().unwrap();
                state.socket = None;
                active.changed.notify_all();
            })
        {
            self.shared.state.lock().unwrap().socket = None;
            return Err(error.into());
        }
        Ok(())
    }
    /// Account relay reuses the same pinned admission as native USB.
    pub fn relay_stream(
        &self,
        identity: Arc<Identity>,
        trust: TrustStore,
        names: Arc<crate::names::Names>,
        expected: &str,
    ) -> Result<TcpStream> {
        let (local, remote) = extend_computer_agent::relay::socket_pair()?;
        self.accept_pinned(remote, identity, trust, names, expected)?;
        Ok(local)
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.shared.state.lock().unwrap();
        serde_json::json!({"state":if state.running { state.outcome } else if state.outcome == "expired" { "expired" } else { "closed" },
            "port":self.shared.port,"code":state.code.as_ref().map(|s| s.as_str()),
            "secondsRemaining":self.shared.expires.map(|expires|expires.saturating_duration_since(Instant::now()).as_secs()).unwrap_or(0),
            "request":state.request.as_ref().map(|r| serde_json::json!({"id":r.id.to_string(),"peer":r.peer,"kind":r.kind,"symbols":r.symbols}))})
    }
    pub fn respond(&self, id: &str, allow: bool) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        ensure!(
            state.running
                && !self
                    .shared
                    .expires
                    .is_some_and(|expires| Instant::now() >= expires),
            "Pairing closed"
        );
        let request = state
            .request
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Pairing request expired"))?;
        ensure!(
            request.id.to_string() == id && request.decision.is_none(),
            "Pairing request changed"
        );
        request.decision = Some(allow);
        self.shared.changed.notify_all();
        Ok(())
    }
    /// Cancel only this authenticated peer's active input lease. The caller
    /// must first remove unavailable account trust, retaining manual trust.
    /// Listener ownership and permission configuration remain unchanged.
    pub fn cancel_peer(&self, peer: &str) -> bool {
        let mut state = self.shared.state.lock().unwrap();
        if !state
            .input_lease
            .as_ref()
            .is_some_and(|(current, _)| current == peer)
        {
            return false;
        }
        let (_, id) = state.input_lease.take().unwrap();
        if let Some(input) = &self.shared.input {
            // Clear queued input and enqueue native release before waking the
            // protocol worker; no further events can use this generation.
            input.finish(id);
        }
        if let Some(socket) = &state.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.shared.changed.notify_all();
        true
    }
    pub fn stop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.running = false;
            state.code = None;
            if let Some((_, id)) = state.input_lease.take() {
                if let Some(input) = &self.shared.input {
                    input.finish(id);
                }
            }
            if let Some(socket) = &state.socket {
                let _ = socket.shutdown(Shutdown::Both);
            }
            self.shared.changed.notify_all();
        }
        // A private stream may occupy the admission slot while the LAN worker
        // is still in accept(). Wake it regardless of the current socket owner.
        if self.shared.port != 0 {
            wake_accept(self.shared.port);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}
// Only an expiring foreground window can ask to trust an unknown visual peer.
fn await_approval(active: &Shared, peer: &str, symbols: Option<[u8; 8]>) -> bool {
    let mut state = active.state.lock().unwrap();
    state.code = None;
    let Some(expires) = active.expires else {
        return false;
    };
    if !state.running || Instant::now() >= expires {
        return false;
    }
    state.request = Some(Request {
        id: REQUESTS.fetch_add(1, Ordering::Relaxed),
        peer: peer.to_owned(),
        kind: if symbols.is_some() { "verify" } else { "pair" },
        symbols,
        decision: None,
    });
    let deadline = if symbols.is_some() {
        expires
    } else {
        expires.min(Instant::now() + Duration::from_secs(30))
    };
    loop {
        if !state.running || Instant::now() >= deadline {
            return false;
        }
        match state.request.as_ref().and_then(|r| r.decision) {
            Some(allow) => return allow,
            None => {
                state = active
                    .changed
                    .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                    .unwrap()
                    .0
            }
        }
    }
}
fn wake_accept(port: u16) {
    let _ = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(100),
    );
}
impl Drop for Incoming {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::session::Client;
    fn pending(listener: &Incoming) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(id) = listener.snapshot()["request"]["id"].as_str() {
                return id.to_owned();
            }
            assert!(
                Instant::now() < deadline,
                "Expected authenticated consent request"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn incoming_requires_consent_reconnects_and_cancels() {
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let trust = TrustStore::open(files.path().join("tablet")).unwrap();
        let remote = Arc::new(Identity::generate());
        let remote_trust = TrustStore::open(files.path().join("remote")).unwrap();
        let peer = remote.fingerprint();
        let mut listener =
            Incoming::start(identity.clone(), trust.clone(), Duration::from_secs(10)).unwrap();
        let snapshot = listener.snapshot();
        let address = format!("127.0.0.1:{}", snapshot["port"]);
        let code = snapshot["code"].as_str().unwrap().to_owned();
        let remote_copy = remote.clone();
        let store_copy = remote_trust.clone();
        let target = address.clone();
        let client = thread::spawn(move || {
            let mut client = Client::connect(
                TcpStream::connect(target).unwrap(),
                &remote_copy,
                &store_copy,
                Some(&code),
                None,
                |_, _| Decision::Remember,
            )?;
            client.probe()?;
            client.close()
        });
        let id = pending(&listener);
        assert!(trust.load().unwrap().peers.is_empty());
        assert!(listener.respond("stale-request", true).is_err());
        listener.respond(&id, true).unwrap();
        assert!(listener.respond(&id, true).is_err());
        client.join().unwrap().unwrap();
        assert!(trust.peer(&peer).unwrap().is_some());
        let mut client = Client::connect(
            TcpStream::connect(&address).unwrap(),
            &remote,
            &remote_trust,
            None,
            Some(&identity.fingerprint()),
            |_, _| Decision::Deny,
        )
        .unwrap();
        client.probe().unwrap();
        client.close().unwrap();
        listener.stop();
        assert!(TcpStream::connect(&address).is_err());

        for approve in [Some(false), None] {
            let stranger = Arc::new(Identity::generate());
            let stranger_peer = stranger.fingerprint();
            let store = TrustStore::open(files.path().join(&stranger_peer)).unwrap();
            let mut listener =
                Incoming::start(identity.clone(), trust.clone(), Duration::from_secs(10)).unwrap();
            let snapshot = listener.snapshot();
            let code = snapshot["code"].as_str().unwrap().to_owned();
            let port = snapshot["port"].as_u64().unwrap();
            let client = thread::spawn(move || {
                Client::connect(
                    TcpStream::connect(("127.0.0.1", port as u16)).unwrap(),
                    &stranger,
                    &store,
                    Some(&code),
                    None,
                    |_, _| Decision::Remember,
                )
                .is_err()
            });
            let id = pending(&listener);
            if let Some(allow) = approve {
                listener.respond(&id, allow).unwrap();
            } else {
                listener.stop();
            }
            assert!(client.join().unwrap());
            assert!(trust.peer(&stranger_peer).unwrap().is_none());
            listener.stop();
        }
    }
    #[test]
    #[ignore = "Owns process-wide receiver; run alone with --ignored --test-threads=1"]
    fn manual_pairing_denies_busy_arrival_and_busy_after_consent() {
        // This isolated test exercises real PAKE and the production native busy
        // guard, without exposing other parallel fixtures to global input state.
        let input = crate::receiver::shared();
        struct Reset(Arc<crate::receiver::Receiver>);
        impl Drop for Reset {
            fn drop(&mut self) {
                let _ = self.0.configure(false, 100.0, 100.0);
            }
        }
        let _reset = Reset(input.clone());
        for busy_before in [true, false] {
            let files = tempfile::tempdir().unwrap();
            let identity = Arc::new(Identity::generate());
            let trust = TrustStore::open(files.path().join("tablet")).unwrap();
            let remote = Arc::new(Identity::generate());
            let peer = remote.fingerprint();
            let remote_trust = TrustStore::open(files.path().join("remote")).unwrap();
            let mut listener =
                Incoming::start(identity, trust.clone(), Duration::from_secs(10)).unwrap();
            let state = listener.snapshot();
            let port = state["port"].as_u64().unwrap() as u16;
            let code = state["code"].as_str().unwrap().to_owned();
            input.configure(true, 100.0, 100.0).unwrap();
            input.drain();
            let mut held = None;
            if busy_before {
                held = input.begin(&peer).unwrap();
                assert!(held.is_some());
            }
            let client = thread::spawn(move || {
                Client::connect(
                    TcpStream::connect(("127.0.0.1", port)).unwrap(),
                    &remote,
                    &remote_trust,
                    Some(&code),
                    None,
                    |_, _| Decision::Remember,
                )
                .is_err()
            });
            if !busy_before {
                let request = pending(&listener);
                input.drain();
                held = input.begin(&peer).unwrap();
                assert!(held.is_some());
                listener.respond(&request, true).unwrap();
            }
            assert!(client.join().unwrap());
            assert!(trust.peer(&peer).unwrap().is_none());
            assert!(
                input.active(),
                "Pairing must not terminate the existing input session"
            );
            assert!(listener.snapshot()["request"].is_null());
            input.finish(held.unwrap());
            listener.stop();
        }
    }
    #[test]
    fn expiry_interrupts_idle_handshake_and_releases_port() {
        let files = tempfile::tempdir().unwrap();
        let mut listener = Incoming::start(
            Arc::new(Identity::generate()),
            TrustStore::open(files.path()).unwrap(),
            Duration::from_millis(100),
        )
        .unwrap();
        let port = listener.snapshot()["port"].as_u64().unwrap() as u16;
        let _idle = TcpStream::connect(("127.0.0.1", port)).unwrap();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(listener.snapshot()["state"], "expired");
        listener.stop();
        assert!(TcpListener::bind(("0.0.0.0", port)).is_ok());
    }
}

struct NameSink {
    input: Option<Arc<crate::receiver::Receiver>>,
    input_id: Option<u64>,
    shared: Arc<Shared>,
    listener_gate: Option<Arc<Shared>>,
    host: Option<std::net::IpAddr>,
    name: String,
    names: Option<Arc<crate::names::Names>>,
}
impl session::CursorSink for NameSink {
    fn shortcut_policy(&self) -> Option<extend_computer_agent::input::ShortcutPolicy> {
        self.input.as_ref()?.shortcut_policy(self.input_id?)
    }
    fn supports_text_input(&self) -> bool {
        self.input
            .as_ref()
            .is_some_and(|input| input.supports_text_input())
    }
    fn unpaired(&mut self, peer: &str) -> Result<()> {
        // The shared engine committed authenticated revocation before this callback.
        if let Some(runtime) = crate::RUNTIME.get() {
            crate::accounts::cancel_retired(
                runtime,
                std::collections::BTreeMap::from([(peer.to_owned(), String::new())]),
            )?;
            let accessory = crate::usb::snapshot();
            if accessory["peer"].as_str() == Some(peer) {
                if let Some(id) = accessory["id"].as_str() {
                    crate::usb::stop(id);
                }
            }
        }
        if let Some(names) = &self.names {
            names.record_removal(peer)?;
        }
        Ok(())
    }
    fn start_cursor(&mut self, peer: &str) -> Result<bool> {
        self.start_control(peer)
    }
    fn start_control(&mut self, peer: &str) -> Result<bool> {
        if let Some(listener) = &self.listener_gate {
            ensure!(listener.state.lock().unwrap().running, "Receiving is off");
        }
        let mut state = self.shared.state.lock().unwrap();
        ensure!(state.running, "Incoming listener closed");
        self.input_id = match &self.input {
            Some(input) => input.begin(peer)?,
            None => None,
        };
        state.input_lease = self.input_id.map(|id| (peer.to_owned(), id));
        Ok(self.input_id.is_some())
    }
    fn start_input(&mut self, peer: &str) -> Result<bool> {
        self.start_control(peer)
    }
    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.input
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Input unavailable"))?
            .enqueue(
                self.input_id
                    .ok_or_else(|| anyhow::anyhow!("Input not granted"))?,
                serde_json::json!({"kind":"move","x":x,"y":y}),
            )
    }
    fn input(&mut self, event: &extend_computer_agent::input::InputEvent) -> Result<()> {
        self.input
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Input unavailable"))?
            .input(
                self.input_id
                    .ok_or_else(|| anyhow::anyhow!("Input not granted"))?,
                event,
            )
    }
    fn heartbeat(&mut self) -> Result<()> {
        if let (Some(input), Some(id)) = (&self.input, self.input_id) {
            input.alive(id)?;
        }
        Ok(())
    }
    fn display_size(&self) -> Result<session::DisplaySize> {
        self.input
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Input unavailable"))?
            .display()
    }
    fn finish(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        if let (Some(input), Some(id)) = (&self.input, self.input_id.take()) {
            input.finish(id);
            if state
                .input_lease
                .as_ref()
                .is_some_and(|(_, current)| *current == id)
            {
                state.input_lease = None;
            }
        }
    }

    fn device_name(&self) -> Option<String> {
        Some(
            self.names
                .as_ref()
                .map(|names| names.local(&self.name))
                .unwrap_or_else(|| self.name.clone()),
        )
    }
    fn peer_name(&mut self, peer: &str, name: &str) -> Result<()> {
        if let Some(names) = &self.names {
            names.save(peer, name)?;
            if let Some(host) = self.host {
                names.save_host(peer, host)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod receiving_tests {
    use super::*;
    #[test]
    fn account_peer_cancel_releases_idle_input_and_preserves_listener_and_manual_peer() {
        use extend_computer_agent::{account_trust::AccountTrust, input::InputEvent};
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let account_peer = Identity::generate();
        let manual_peer = Identity::generate();
        let account = AccountTrust::default();
        account.replace(
            "test-account",
            &[account_peer.fingerprint()],
            Duration::from_secs(60),
        );
        let trust = TrustStore::open(files.path().join("tablet"))
            .unwrap()
            .with_account_trust(account.clone());
        trust.remember(&manual_peer.fingerprint()).unwrap();
        let remote_trust = TrustStore::open(files.path().join("remote")).unwrap();
        remote_trust.remember(&identity.fingerprint()).unwrap();
        let input = Arc::new(crate::receiver::Receiver::default());
        input.configure(true, 1600., 2560.).unwrap();
        input.drain();
        let mut listener = Incoming::receiving(
            identity.clone(),
            trust.clone(),
            "Tablet".into(),
            Arc::new(crate::names::Names::open(files.path()).unwrap()),
            input.clone(),
        )
        .unwrap();
        let address = format!("127.0.0.1:{}", listener.snapshot()["port"]);
        let connect = |peer: &Identity| {
            session::Client::connect(
                TcpStream::connect(&address).unwrap(),
                peer,
                &remote_trust,
                None,
                Some(&identity.fingerprint()),
                |_, _| Decision::Deny,
            )
            .unwrap()
        };
        let mut client = connect(&account_peer);
        client.request_control().unwrap();
        client
            .queue_input(InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            })
            .unwrap();
        client.probe().unwrap();
        let events = input.drain();
        let lease = events[0]["id"].as_u64().unwrap();
        assert_eq!(events[1]["kind"], "button");
        assert!(!listener.cancel_peer(&manual_peer.fingerprint()));
        assert!(input.alive(lease).is_ok());
        account.clear();
        assert!(trust.peer(&account_peer.fingerprint()).unwrap().is_none());
        assert!(trust.peer(&manual_peer.fingerprint()).unwrap().is_some());
        // The remote is idle: release cannot depend on another protocol message.
        assert!(listener.cancel_peer(&account_peer.fingerprint()));
        assert!(input.alive(lease).is_err());
        let released = input.drain();
        assert_eq!(released.as_array().unwrap().len(), 1);
        assert_eq!(released[0]["kind"], "release");
        assert_eq!(released[0]["end"], true);
        assert_eq!(released[0]["id"], lease);
        assert!(client.probe().is_err());
        let deadline = Instant::now() + Duration::from_secs(2);
        while listener.shared.state.lock().unwrap().socket.is_some() {
            assert!(Instant::now() < deadline, "Revoked socket did not close");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(listener.snapshot()["state"], "waiting");
        let mut manual = connect(&manual_peer);
        manual.request_control().unwrap();
        assert!(!listener.cancel_peer(&account_peer.fingerprint()));
        manual.probe().unwrap();
        assert!(input.active());
        assert_eq!(input.drain()[0]["peer"], manual_peer.fingerprint());
        listener.stop();
        assert!(!input.active());
    }

    #[test]
    fn receiving_rejects_unknown_and_persists_authenticated_names() {
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let trust = TrustStore::open(files.path().join("trust")).unwrap();
        let names = Arc::new(crate::names::Names::open(files.path()).unwrap());
        let mut listener = Incoming::named(
            identity.clone(),
            trust.clone(),
            None,
            "Test Tablet".into(),
            names.clone(),
        )
        .unwrap();
        let address = format!("127.0.0.1:{}", listener.snapshot()["port"]);
        assert!(listener.snapshot()["code"].is_null());
        let remote = Identity::generate();
        let remote_store = TrustStore::open(files.path().join("remote")).unwrap();
        assert!(session::Client::connect(
            TcpStream::connect(&address).unwrap(),
            &remote,
            &remote_store,
            None,
            Some(&identity.fingerprint()),
            |_, _| Decision::Remember
        )
        .is_err());
        assert!(trust.load().unwrap().peers.is_empty());
        trust.remember(&remote.fingerprint()).unwrap();
        remote_store.remember(&identity.fingerprint()).unwrap();
        let name = session::exchange_device_name(
            TcpStream::connect(&address).unwrap(),
            &remote,
            &identity.fingerprint(),
            "Test Mac",
        )
        .unwrap();
        assert_eq!(name, "Test Tablet");
        let restored = crate::names::Names::open(files.path()).unwrap();
        let records: serde_json::Value =
            serde_json::from_str(&restored.snapshot().unwrap()).unwrap();
        assert_eq!(records[remote.fingerprint()], "Test Mac");
        let hosts: serde_json::Value = serde_json::from_str(&restored.hosts().unwrap()).unwrap();
        assert_eq!(hosts[remote.fingerprint()], "127.0.0.1");
        assert_eq!(hosts.as_object().unwrap().len(), 1);
        assert!(session::exchange_device_name(
            TcpStream::connect(&address).unwrap(),
            &Identity::generate(),
            &identity.fingerprint(),
            "Spoofed Mac"
        )
        .is_err());
        let records: serde_json::Value = serde_json::from_str(&names.snapshot().unwrap()).unwrap();
        assert_eq!(records[remote.fingerprint()], "Test Mac");
        listener.stop();
        assert!(TcpStream::connect(&address).is_err());
    }
}

#[cfg(test)]
mod relay_tests {
    use super::*;
    use extend_computer_agent::session::Client;
    #[test]
    fn relay_offer_pin_rejects_other_trusted_identity_and_eof_releases_exact_input_lease() {
        let files = tempfile::tempdir().unwrap();
        let local = Arc::new(Identity::generate());
        let expected = Identity::generate();
        let substituted = Identity::generate();
        let trust = TrustStore::open(files.path().join("local")).unwrap();
        trust.remember(&expected.fingerprint()).unwrap();
        trust.remember(&substituted.fingerprint()).unwrap();
        let sender_trust = TrustStore::open(files.path().join("sender")).unwrap();
        sender_trust.remember(&local.fingerprint()).unwrap();
        let names = Arc::new(crate::names::Names::open(files.path()).unwrap());
        let receiver = Arc::new(crate::receiver::Receiver::default());
        receiver.configure(true, 1000., 800.).unwrap();
        receiver.drain();
        let mut incoming = Incoming::receiving(
            local.clone(),
            trust.clone(),
            "Tablet".into(),
            names.clone(),
            receiver.clone(),
        )
        .unwrap();
        let socket = incoming
            .relay_stream(
                local.clone(),
                trust.clone(),
                names.clone(),
                &expected.fingerprint(),
            )
            .unwrap();
        assert!(Client::connect(
            socket,
            &substituted,
            &sender_trust,
            None,
            Some(&local.fingerprint()),
            |_, _| Decision::Deny
        )
        .is_err());
        assert!(!receiver.active());
        assert_eq!(names.hosts().unwrap(), "{}");
        receiver.drain();
        let socket = incoming
            .relay_stream(
                local.clone(),
                trust.clone(),
                names.clone(),
                &expected.fingerprint(),
            )
            .unwrap();
        let mut client = Client::connect(
            socket,
            &expected,
            &sender_trust,
            None,
            Some(&local.fingerprint()),
            |_, _| Decision::Deny,
        )
        .unwrap();
        client.request_control().unwrap();
        client
            .queue_input(extend_computer_agent::input::InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            })
            .unwrap();
        client.probe().unwrap();
        assert!(receiver.active());
        drop(client);
        let until = Instant::now() + Duration::from_secs(2);
        while receiver.active() {
            assert!(Instant::now() < until, "EOF kept relay input held");
            thread::sleep(Duration::from_millis(5));
        }
        let events = receiver.drain();
        assert!(events
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "release" && e["end"] == true));
        incoming.stop();
        assert!(incoming
            .relay_stream(local, trust, names, &expected.fingerprint())
            .is_err());
    }
}
