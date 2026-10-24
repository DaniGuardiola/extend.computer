//! A code-entry pairing attempt whose local trust commit is serialized with cancel.
//! The remote may already remember an approved peer before its grant reaches us;
//! cancellation cannot roll back another device's explicit approval.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    identity::Identity,
    session::{Client, Decision},
    trust::TrustStore,
};
use std::{
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

struct State {
    phase: &'static str,
    active: bool,
    socket: Option<TcpStream>,
    peer: Option<String>,
    error: String,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    expires: Instant,
}
impl Shared {
    fn stop(&self, phase: &'static str) {
        let mut state = self.state.lock().unwrap();
        if state.active {
            state.active = false;
            state.phase = phase;
        }
        if let Some(socket) = &state.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.changed.notify_all();
    }
    fn commit(&self, trust: &TrustStore, peer: &str, can_commit: &dyn Fn() -> bool) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        ensure!(
            state.active && Instant::now() < self.expires,
            "Pairing canceled"
        );
        ensure!(can_commit(), "Stop the current connection before pairing");
        // Client used Once, so this is the only local durable pairing write.
        // A cancel that takes this lock first prevents it; a later cancel does
        // not erase a completed, explicitly authorized pairing.
        trust.remember(peer)?;
        state.peer = Some(peer.to_owned());
        state.phase = "paired";
        state.active = false;
        self.changed.notify_all();
        Ok(())
    }
}
pub(super) struct ManualPairing {
    pub id: String,
    address: SocketAddr,
    name: String,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    timer: Option<JoinHandle<()>>,
}
impl ManualPairing {
    pub fn start(
        id: String,
        identity: Arc<Identity>,
        trust: TrustStore,
        address: SocketAddr,
        code: Zeroizing<String>,
        name: String,
        names: Arc<crate::names::Names>,
        ttl: Duration,
        can_commit: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self> {
        ensure!(
            !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()),
            "Invalid pairing operation"
        );
        extend_computer_agent::session::validate_device_name(&name)?;
        ensure!(
            !ttl.is_zero() && ttl <= Duration::from_secs(120),
            "Invalid pairing lifetime"
        );
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phase: "connecting",
                active: true,
                socket: None,
                peer: None,
                error: String::new(),
            }),
            changed: Condvar::new(),
            expires: Instant::now() + ttl,
        });
        let active = shared.clone();
        let worker = thread::Builder::new()
            .name("extend-manual-pair".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let deadline = active.expires.min(Instant::now() + Duration::from_secs(5));
                    let socket = loop {
                        ensure!(active.state.lock().unwrap().active, "Pairing canceled");
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        ensure!(!remaining.is_zero(), "Could not reach this device");
                        match TcpStream::connect_timeout(
                            &address,
                            remaining.min(Duration::from_millis(200)),
                        ) {
                            Ok(socket) => break socket,
                            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
                            Err(error) => return Err(error).context("Could not reach this device"),
                        }
                    };
                    {
                        let mut state = active.state.lock().unwrap();
                        ensure!(
                            state.active && Instant::now() < active.expires,
                            "Pairing canceled"
                        );
                        state.socket = Some(socket.try_clone()?);
                    }
                    let client =
                        Client::connect(socket, &identity, &trust, Some(&code), None, |_, _| {
                            if !can_commit() {
                                return Decision::Deny;
                            }
                            let mut state = active.state.lock().unwrap();
                            if state.active && Instant::now() < active.expires {
                                state.phase = "waiting";
                                Decision::Once
                            } else {
                                Decision::Deny
                            }
                        })?;
                    let peer = client.peer().to_owned();
                    active.commit(&trust, &peer, &*can_commit)?;
                    // An authenticated route hint survives leaving the UI after the
                    // commit. Hint or Close failures cannot turn a committed pair
                    // into a reported pairing failure.
                    let _ = names.save_host(&peer, address.ip());
                    let _ = client.close();
                    Ok(())
                })();
                let mut state = active.state.lock().unwrap();
                state.socket = None;
                if state.active {
                    state.active = false;
                    state.phase = if Instant::now() >= active.expires {
                        "expired"
                    } else {
                        "failed"
                    };
                    if let Err(error) = result {
                        state.error = error.to_string();
                    }
                }
                active.changed.notify_all();
            })?;
        let clock = shared.clone();
        let timer = thread::Builder::new()
            .name("extend-manual-expiry".into())
            .spawn(move || {
                let mut state = clock.state.lock().unwrap();
                while state.active {
                    let remaining = clock.expires.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        state.active = false;
                        state.phase = "expired";
                        if let Some(socket) = &state.socket {
                            let _ = socket.shutdown(Shutdown::Both);
                        }
                        clock.changed.notify_all();
                        break;
                    }
                    state = clock.changed.wait_timeout(state, remaining).unwrap().0;
                }
            });
        let mut operation = Self {
            id,
            address,
            name,
            shared,
            worker: Some(worker),
            timer: None,
        };
        match timer {
            Ok(timer) => operation.timer = Some(timer),
            Err(error) => {
                operation.stop();
                return Err(error.into());
            }
        }
        Ok(operation)
    }
    pub fn active(&self) -> bool {
        self.shared.state.lock().unwrap().active
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.shared.state.lock().unwrap();
        serde_json::json!({"id":self.id,"state":state.phase,"peer":state.peer,
            "name":self.name,"address":self.address.to_string(),"error":state.error})
    }
    /// No thread join on the synchronous UI cancel path. The captured socket is
    /// interrupted immediately; a pending numeric connect checks every 200 ms.
    pub fn stop(&self) {
        self.shared.stop("closed");
    }
}
impl Drop for ManualPairing {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::{pairing::PairingWindow, session};
    use std::{io::Read, net::TcpListener, sync::mpsc};
    struct Sink;
    impl session::CursorSink for Sink {
        fn start_cursor(&mut self, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
            anyhow::bail!("No input in pairing fixture")
        }
    }
    fn wait(operation: &ManualPairing) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let state = operation.snapshot();
            if !operation.active() {
                return state;
            }
            assert!(Instant::now() < deadline, "Pairing did not finish: {state}");
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn start(
        files: &tempfile::TempDir,
        identity: Arc<Identity>,
        trust: TrustStore,
        address: SocketAddr,
        code: String,
        ttl: Duration,
        can_commit: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> ManualPairing {
        // Host network filters can delay a process's first connection past one
        // 200 ms connect slice; the retry would then miss the single-use fixture.
        static WARM: std::sync::Once = std::sync::Once::new();
        WARM.call_once(|| {
            let warm = TcpListener::bind("127.0.0.1:0").unwrap();
            let _ = TcpStream::connect(warm.local_addr().unwrap());
        });
        ManualPairing::start(
            "1".into(),
            identity,
            trust,
            address,
            crate::normalize_pair_code(&code).unwrap(),
            "Desktop".into(),
            Arc::new(crate::names::Names::open(&files.path().join("names")).unwrap()),
            ttl,
            can_commit,
        )
        .unwrap()
    }
    #[test]
    fn code_pair_commits_only_after_grant_and_late_cancel_preserves_both_pairs() {
        let files = tempfile::tempdir().unwrap();
        let left = Arc::new(Identity::generate());
        let right = Arc::new(Identity::generate());
        let trust = TrustStore::open(files.path().join("left")).unwrap();
        let remote = TrustStore::open(files.path().join("right")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let window = PairingWindow::new(Duration::from_secs(10));
        let code = window.code().unwrap().to_owned();
        let server_id = right.clone();
        let server_trust = remote.clone();
        let (pending, approval) = mpsc::channel();
        let (allow, allowed) = mpsc::channel();
        let server = thread::spawn(move || {
            session::serve_connection_with_cursor(
                listener.accept().unwrap().0,
                &server_id,
                &server_trust,
                &mut Some(window),
                |_, _| {
                    pending.send(()).unwrap();
                    allowed.recv_timeout(Duration::from_secs(3)).unwrap();
                    Decision::Remember
                },
                &mut Sink,
            )
        });
        let operation = start(
            &files,
            left.clone(),
            trust.clone(),
            address,
            code,
            Duration::from_secs(10),
            Arc::new(|| true),
        );
        approval.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(trust.peer(&right.fingerprint()).unwrap().is_none());
        allow.send(()).unwrap();
        assert_eq!(wait(&operation)["state"], "paired");
        server.join().unwrap().unwrap();
        operation.stop();
        drop(operation);
        assert!(TrustStore::open(files.path().join("left"))
            .unwrap()
            .peer(&right.fingerprint())
            .unwrap()
            .is_some());
        assert!(remote.peer(&left.fingerprint()).unwrap().is_some());
    }
    #[test]
    fn cancel_waiting_for_remote_approval_never_commits_local_trust_even_after_late_grant() {
        let files = tempfile::tempdir().unwrap();
        let left = Arc::new(Identity::generate());
        let right = Arc::new(Identity::generate());
        let trust = TrustStore::open(files.path().join("left")).unwrap();
        let remote = TrustStore::open(files.path().join("right")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let window = PairingWindow::new(Duration::from_secs(10));
        let code = window.code().unwrap().to_owned();
        let server_id = right.clone();
        let server_trust = remote.clone();
        let (pending, approval) = mpsc::channel();
        let (allow, allowed) = mpsc::channel();
        let server = thread::spawn(move || {
            session::serve_connection_with_cursor(
                listener.accept().unwrap().0,
                &server_id,
                &server_trust,
                &mut Some(window),
                |_, _| {
                    pending.send(()).unwrap();
                    allowed.recv_timeout(Duration::from_secs(3)).unwrap();
                    Decision::Remember
                },
                &mut Sink,
            )
        });
        let operation = start(
            &files,
            left.clone(),
            trust.clone(),
            address,
            code,
            Duration::from_secs(10),
            Arc::new(|| true),
        );
        approval.recv_timeout(Duration::from_secs(3)).unwrap();
        let canceled = Instant::now();
        operation.stop();
        drop(operation);
        assert!(canceled.elapsed() < Duration::from_secs(1));
        allow.send(()).unwrap();
        let _ = server.join().unwrap();
        assert!(trust.peer(&right.fingerprint()).unwrap().is_none());
        // Remote approval is a separate commit, not undone by our cancellation.
        assert!(remote.peer(&left.fingerprint()).unwrap().is_some());
    }
    #[test]
    fn stalled_handshake_is_interrupted_by_cancel_or_autonomous_expiry() {
        for cancel in [true, false] {
            let files = tempfile::tempdir().unwrap();
            let trust = TrustStore::open(files.path().join("trust")).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (accepted, ready) = mpsc::channel();
            let server = thread::spawn(move || {
                let mut socket = listener.accept().unwrap().0;
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut hello = [0; 9];
                socket.read_exact(&mut hello).unwrap();
                assert_eq!(&hello[..8], b"EXTEND05");
                assert_eq!(hello[8], b'P');
                accepted.send(()).unwrap();
                let mut data = Vec::new();
                socket.read_to_end(&mut data).unwrap();
            });
            let operation = start(
                &files,
                Arc::new(Identity::generate()),
                trust.clone(),
                address,
                "1234-5678-abcd-ef01".into(),
                if cancel {
                    Duration::from_secs(10)
                } else {
                    Duration::from_millis(200)
                },
                Arc::new(|| true),
            );
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            if cancel {
                operation.stop();
                assert_eq!(operation.snapshot()["state"], "closed");
            } else {
                assert_eq!(wait(&operation)["state"], "expired");
            }
            let stopped = Instant::now();
            drop(operation);
            assert!(stopped.elapsed() < Duration::from_secs(1));
            server.join().unwrap();
            assert!(trust.load().unwrap().peers.is_empty());
        }
    }
    #[test]
    fn commit_barrier_serializes_cancel_and_preserves_existing_manual_peers() {
        let files = tempfile::tempdir().unwrap();
        let trust = TrustStore::open(files.path()).unwrap();
        let retained = Identity::generate().fingerprint();
        let target = Identity::generate().fingerprint();
        trust.remember(&retained).unwrap();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phase: "waiting",
                active: true,
                socket: None,
                peer: None,
                error: String::new(),
            }),
            changed: Condvar::new(),
            expires: Instant::now() + Duration::from_secs(5),
        });
        shared.stop("closed");
        assert!(shared.commit(&trust, &target, &|| true).is_err());
        assert!(trust.peer(&target).unwrap().is_none());
        assert!(trust.peer(&retained).unwrap().is_some());
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phase: "waiting",
                active: true,
                socket: None,
                peer: None,
                error: String::new(),
            }),
            changed: Condvar::new(),
            expires: Instant::now() + Duration::from_secs(5),
        });
        let (entered, ready) = mpsc::channel();
        let (allow, allowed) = mpsc::channel();
        let committing = shared.clone();
        let committing_trust = trust.clone();
        let peer = target.clone();
        let commit = thread::spawn(move || {
            committing.commit(&committing_trust, &peer, &|| {
                entered.send(()).unwrap();
                allowed.recv_timeout(Duration::from_secs(2)).unwrap();
                true
            })
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let retiring = shared.clone();
        let (stopped, stop_done) = mpsc::channel();
        let stop = thread::spawn(move || {
            retiring.stop("closed");
            stopped.send(()).unwrap();
        });
        assert!(stop_done.recv_timeout(Duration::from_millis(20)).is_err());
        allow.send(()).unwrap();
        commit.join().unwrap().unwrap();
        stop.join().unwrap();
        assert_eq!(shared.state.lock().unwrap().phase, "paired");
        assert!(trust.peer(&target).unwrap().is_some());
    }
    #[test]
    fn local_commit_rechecks_session_policy() {
        let files = tempfile::tempdir().unwrap();
        let trust = TrustStore::open(files.path()).unwrap();
        let peer = Identity::generate().fingerprint();
        let shared = Shared {
            state: Mutex::new(State {
                phase: "waiting",
                active: true,
                socket: None,
                peer: None,
                error: String::new(),
            }),
            changed: Condvar::new(),
            expires: Instant::now() + Duration::from_secs(1),
        };
        assert!(shared.commit(&trust, &peer, &|| false).is_err());
        assert!(trust.peer(&peer).unwrap().is_none());
    }
}
