//! Native ordered input sender. UI threads enqueue; only this worker touches TCP.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    identity::Identity,
    input::{InputEvent, ShortcutPolicy},
    session::{Client, Decision},
    trust::TrustStore,
};
use std::{
    collections::VecDeque,
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
static IDS: AtomicU64 = AtomicU64::new(1);
#[derive(Debug)]
enum Command {
    Move(f64, f64),
    Input(InputEvent),
}
struct State {
    active: bool,
    phase: &'static str,
    error: String,
    socket: Option<TcpStream>,
    queue: VecDeque<Command>,
    width: f64,
    height: f64,
    sent: u64,
    text: bool,
    shortcut_policy: Option<ShortcutPolicy>,
    route: &'static str,
    cancellation: Arc<extend_computer_agent::relay::Cancellation>,
    stop: Arc<AtomicBool>,
}
pub struct Sender {
    id: String,
    peer: String,
    state: Arc<(Mutex<State>, Condvar)>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Sender {
    pub fn start(
        identity: Arc<Identity>,
        trust: TrustStore,
        peer: String,
        address: SocketAddr,
    ) -> Result<Self> {
        Self::start_with_connector(identity, trust, peer, move |_| {
            Ok((
                TcpStream::connect_timeout(&address, Duration::from_secs(3))?,
                "lan",
            ))
        })
    }
    pub fn start_routed(
        identity: Arc<Identity>,
        trust: TrustStore,
        peer: String,
        address: String,
        relay: Arc<extend_computer_agent::relay::Relay>,
    ) -> Result<Self> {
        let target = peer.clone();
        // Select the carrier before parsing a possibly stale or absent LAN hint.
        let usb = crate::usb::selected(&peer, &address)?;
        let address = if usb.is_some() {
            None
        } else {
            crate::relay::endpoint_optional(&address)?
        };
        Self::start_with_route_connector(identity, trust, peer, move |cancel, stop| {
            if let Some(usb) = usb {
                return Ok((
                    usb.open(
                        extend_computer_agent::display::usb_stream::UsbService::Control,
                        &stop,
                        Instant::now() + Duration::from_secs(3),
                    )?,
                    "usb",
                ));
            }
            crate::relay::outgoing(&target, address, &relay, cancel)
        })
    }
    pub fn start_with_connector(
        identity: Arc<Identity>,
        trust: TrustStore,
        peer: String,
        connect: impl FnOnce(
                Arc<extend_computer_agent::relay::Cancellation>,
            ) -> Result<(TcpStream, &'static str)>
            + Send
            + 'static,
    ) -> Result<Self> {
        Self::start_with_route_connector(identity, trust, peer, move |cancel, _| connect(cancel))
    }
    fn start_with_route_connector(
        identity: Arc<Identity>,
        trust: TrustStore,
        peer: String,
        connect: impl FnOnce(
                Arc<extend_computer_agent::relay::Cancellation>,
                Arc<AtomicBool>,
            ) -> Result<(TcpStream, &'static str)>
            + Send
            + 'static,
    ) -> Result<Self> {
        ensure!(trust.peer(&peer)?.is_some(), "Device is not paired");
        let id = IDS.fetch_add(1, Ordering::Relaxed).to_string();
        let target_peer = peer.clone();
        let state = Arc::new((
            Mutex::new(State {
                active: true,
                phase: "connecting",
                error: String::new(),
                socket: None,
                queue: VecDeque::new(),
                width: 0.0,
                height: 0.0,
                sent: 0,
                text: false,
                shortcut_policy: None,
                route: "connecting",
                cancellation: Arc::new(Default::default()),
                stop: Arc::new(AtomicBool::new(false)),
            }),
            Condvar::new(),
        ));
        let shared = state.clone();
        let worker = thread::Builder::new()
            .name("extend-input-sender".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let (cancellation, stop) = {
                        let state = shared.0.lock().unwrap();
                        (state.cancellation.clone(), state.stop.clone())
                    };
                    let (socket, route) = connect(cancellation, stop)?;
                    {
                        let mut state = shared.0.lock().unwrap();
                        ensure!(state.active, "Control canceled");
                        state.socket = Some(socket.try_clone()?);
                        state.route = route;
                    }
                    let mut client =
                        Client::connect(socket, &identity, &trust, None, Some(&peer), |_, _| {
                            Decision::Deny
                        })?;
                    let display = client.request_control()?;
                    {
                        let mut state = shared.0.lock().unwrap();
                        ensure!(state.active, "Control canceled");
                        state.width = display.width;
                        state.height = display.height;
                        state.phase = "connected";
                        state.text = client.supports_text_input();
                        state.shortcut_policy = client.shortcut_policy();
                    }
                    let mut pending = 0;
                    let mut ack = Instant::now();
                    loop {
                        let command = {
                            let mut state = shared.0.lock().unwrap();
                            if !state.active {
                                break;
                            }
                            if state.queue.is_empty() && pending == 0 {
                                state = shared
                                    .1
                                    .wait_timeout(state, Duration::from_millis(50))
                                    .unwrap()
                                    .0;
                            }
                            if !state.active {
                                break;
                            }
                            state.queue.pop_front()
                        };
                        ensure!(trust.peer(&peer)?.is_some(), "Device was forgotten");
                        if let Some(command) = command {
                            match command {
                                Command::Move(x, y) => client.queue_cursor(x, y)?,
                                Command::Input(event) => client.queue_input(event)?,
                            }
                            shared.0.lock().unwrap().sent += 1;
                            pending += 1;
                        }
                        if pending == 4
                            || (pending > 0 && command_is_idle(&shared))
                            || ack.elapsed() >= Duration::from_millis(300)
                        {
                            client.probe()?;
                            pending = 0;
                            ack = Instant::now();
                        }
                    }
                    // Remote CursorSink::finish also releases every held input on EOF.
                    let _ = client.close();
                    Ok(())
                })();
                let mut state = shared.0.lock().unwrap();
                if state.active {
                    state.phase = if result.is_ok() { "closed" } else { "failed" };
                    state.error = result.err().map(|e| format!("{e:#}")).unwrap_or_default();
                }
                state.active = false;
                state.text = false;
                state.shortcut_policy = None;
                state.socket = None;
                state.queue.clear();
                shared.1.notify_all();
            })?;
        Ok(Self {
            id,
            peer: target_peer,
            state,
            worker: Mutex::new(Some(worker)),
        })
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.state.0.lock().unwrap();
        serde_json::json!({"id":self.id,"peer":self.peer,"state":state.phase,"error":state.error,"width":state.width,"height":state.height,"sent":state.sent,"text_supported":state.text,"shortcut_policy":state.shortcut_policy,"route":state.route})
    }
    pub fn enqueue(&self, id: &str, json: &str) -> Result<()> {
        ensure!(id == self.id, "Control session replaced");
        ensure!(json.len() <= 1024, "Input event too large");
        let value: serde_json::Value = serde_json::from_str(json)?;
        let command = if value["kind"] == "move" {
            ensure!(
                value.as_object().is_some_and(|v| v.len() == 3),
                "Invalid cursor event"
            );
            let x = value["x"].as_f64().context("Missing cursor x")?;
            let y = value["y"].as_f64().context("Missing cursor y")?;
            ensure!(
                x.is_finite()
                    && y.is_finite()
                    && (0.0..=1.0).contains(&x)
                    && (0.0..=1.0).contains(&y),
                "Invalid cursor position"
            );
            Command::Move(x, y)
        } else {
            let event: InputEvent = serde_json::from_value(value)?;
            event.validate()?;
            Command::Input(event)
        };
        let mut state = self.state.0.lock().unwrap();
        ensure!(
            state.active && state.phase == "connected",
            "Control is not connected"
        );
        if matches!(&command, Command::Input(InputEvent::Text { .. })) {
            ensure!(state.text, "This peer does not support committed text");
        }
        if matches!(command, Command::Input(InputEvent::Release)) {
            state.queue.clear();
        }
        if matches!(command, Command::Move(..))
            && matches!(state.queue.back(), Some(Command::Move(..)))
        {
            state.queue.pop_back();
        }
        if state.queue.len() >= 256 {
            state.active = false;
            state.phase = "failed";
            state.error = "Input queue overflow".into();
            if let Some(socket) = &state.socket {
                let _ = socket.shutdown(Shutdown::Both);
            }
            self.state.1.notify_all();
            anyhow::bail!("Input queue overflow; control stopped");
        }
        state.queue.push_back(command);
        self.state.1.notify_all();
        Ok(())
    }
    pub fn cancel(&self, id: &str) {
        if id != self.id {
            return;
        }
        let mut state = self.state.0.lock().unwrap();
        state.active = false;
        state.text = false;
        state.shortcut_policy = None;
        state.stop.store(true, Ordering::Release);
        state.cancellation.cancel();
        state.phase = "closed";
        state.queue.clear();
        if let Some(socket) = &state.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.state.1.notify_all();
    }
}
fn command_is_idle(shared: &Arc<(Mutex<State>, Condvar)>) -> bool {
    shared.0.lock().unwrap().queue.is_empty()
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.cancel(&self.id);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            // Cancellation interrupts TCP; never block a UI-owned final Arc drop.
            // The worker owns its state until the bounded connect/read exits.
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::session::{self, CursorSink};
    use std::net::TcpListener;
    struct Sink(Arc<Mutex<Vec<String>>>);
    impl CursorSink for Sink {
        fn supports_text_input(&self) -> bool {
            true
        }
        fn start_control(&mut self, _: &str) -> Result<bool> {
            self.0.lock().unwrap().push("start".into());
            Ok(true)
        }
        fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
            self.0.lock().unwrap().push(format!("move {x} {y}"));
            Ok(())
        }
        fn input(&mut self, event: &InputEvent) -> Result<()> {
            self.0.lock().unwrap().push(serde_json::to_string(event)?);
            Ok(())
        }
        fn finish(&mut self) {
            self.0.lock().unwrap().push("released".into());
        }
    }
    fn wait_for(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(Instant::now() < deadline, "Control state did not settle");
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn committed_text_keeps_authenticated_control_and_stale_session_guards() {
        let files = tempfile::tempdir().unwrap();
        let local = Arc::new(Identity::generate());
        let remote = Arc::new(Identity::generate());
        let trust = TrustStore::open(files.path().join("local")).unwrap();
        trust.remember(&remote.fingerprint()).unwrap();
        let remote_trust = TrustStore::open(files.path().join("remote")).unwrap();
        remote_trust.remember(&local.fingerprint()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let log = events.clone();
        let peer = remote.fingerprint();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let _ = session::serve_connection_with_cursor(
                socket,
                &remote,
                &remote_trust,
                &mut None,
                |_, _| Decision::Once,
                &mut Sink(log),
            );
        });
        let sender = Sender::start(local, trust, peer, address).unwrap();
        wait_for(|| sender.snapshot()["state"] == "connected");
        assert_eq!(sender.snapshot()["text_supported"], true);
        let value = r#"{"kind":"text","value":"é🙂中文"}"#;
        assert!(sender.enqueue("wrong", value).is_err());
        {
            sender.state.0.lock().unwrap().text = false;
            assert!(sender.enqueue(&sender.id, value).is_err());
            assert!(sender.state.0.lock().unwrap().queue.is_empty());
            sender.state.0.lock().unwrap().text = true;
        }
        sender.enqueue(&sender.id, value).unwrap();
        let expected = serde_json::to_string(&InputEvent::Text {
            value: "é🙂中文".into(),
        })
        .unwrap();
        wait_for(|| {
            events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event == &expected)
        });
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.starts_with('{'))
                .cloned()
                .collect::<Vec<_>>(),
            vec![expected]
        );
        sender.cancel(&sender.id);
        assert!(sender.enqueue(&sender.id, value).is_err());
        drop(sender);
        server.join().unwrap();
    }
    struct ControlSlot;
    impl Drop for ControlSlot {
        fn drop(&mut self) {
            crate::CONTROL.lock().unwrap().take();
        }
    }
    fn account_runtime(root: &std::path::Path, peer: &str, manual: bool) -> crate::Runtime {
        let account = crate::accounts::Membership::default();
        account
            .trust
            .replace("test-account", &[peer.to_owned()], Duration::from_secs(60));
        let trust = TrustStore::open(root.join("trust"))
            .unwrap()
            .with_account_trust(account.trust.clone());
        if manual {
            trust.remember(peer).unwrap();
        }
        crate::Runtime {
            identity: Arc::new(Identity::generate()),
            trust,
            root: root.to_owned(),
            names: Arc::new(crate::names::Names::open(root).unwrap()),
            account,
            relay: Arc::new(Default::default()),
        }
    }
    #[test]
    #[ignore = "Owns process-wide CONTROL; run alone with --ignored --test-threads=1"]
    fn account_revocation_closes_only_matching_account_control_and_releases_remote_input() {
        use std::collections::BTreeMap;
        let _cleanup = ControlSlot;
        assert!(crate::CONTROL.lock().unwrap().is_none());
        for manual in [false, true] {
            let files = tempfile::tempdir().unwrap();
            let remote = Arc::new(Identity::generate());
            let runtime = account_runtime(files.path(), &remote.fingerprint(), manual);
            let remote_trust = TrustStore::open(files.path().join("remote")).unwrap();
            remote_trust
                .remember(&runtime.identity.fingerprint())
                .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let events = Arc::new(Mutex::new(Vec::new()));
            let log = events.clone();
            let identity = remote.clone();
            let server = thread::spawn(move || {
                let (socket, _) = listener.accept().unwrap();
                let _ = session::serve_connection_with_cursor(
                    socket,
                    &identity,
                    &remote_trust,
                    &mut None,
                    |_, _| Decision::Once,
                    &mut Sink(log),
                );
            });
            let sender = Arc::new(
                Sender::start(
                    runtime.identity.clone(),
                    runtime.trust.clone(),
                    remote.fingerprint(),
                    address,
                )
                .unwrap(),
            );
            *crate::CONTROL.lock().unwrap() = Some(sender.clone());
            wait_for(|| sender.snapshot()["state"] == "connected");
            assert_eq!(sender.snapshot()["peer"], remote.fingerprint());
            sender
                .enqueue(
                    &sender.id,
                    r#"{"kind":"button","button":0,"down":true,"clicks":1}"#,
                )
                .unwrap();
            wait_for(|| {
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|event| event.contains("button"))
            });
            let other_peer = Identity::generate().fingerprint();
            crate::accounts::cancel_retired(
                &runtime,
                BTreeMap::from([(other_peer, "Other".into())]),
            )
            .unwrap();
            assert_eq!(sender.snapshot()["state"], "connected");
            runtime.account.trust.clear();
            crate::accounts::cancel_retired(
                &runtime,
                BTreeMap::from([(remote.fingerprint(), "Remote".into())]),
            )
            .unwrap();
            if manual {
                assert!(runtime.trust.peer(&remote.fingerprint()).unwrap().is_some());
                assert_eq!(sender.snapshot()["state"], "connected");
                sender
                    .enqueue(
                        &sender.id,
                        r#"{"kind":"button","button":0,"down":false,"clicks":1}"#,
                    )
                    .unwrap();
                wait_for(|| {
                    events
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|event| event.contains("button"))
                        .count()
                        == 2
                });
                sender.cancel(&sender.id);
            } else {
                assert!(runtime.trust.peer(&remote.fingerprint()).unwrap().is_none());
                assert_eq!(sender.snapshot()["state"], "closed");
                assert!(sender.enqueue(&sender.id, r#"{"kind":"release"}"#).is_err());
            }
            wait_for(|| {
                events
                    .lock()
                    .unwrap()
                    .last()
                    .is_some_and(|event| event == "released")
            });
            server.join().unwrap();
            crate::CONTROL.lock().unwrap().take();
        }
    }
    #[test]
    #[ignore = "Owns process-wide CONTROL; run alone with --ignored --test-threads=1"]
    fn account_revocation_interrupts_control_startup_without_waiting_for_handshake_timeout() {
        use std::{collections::BTreeMap, io::Read, sync::mpsc};
        let _cleanup = ControlSlot;
        assert!(crate::CONTROL.lock().unwrap().is_none());
        let files = tempfile::tempdir().unwrap();
        let peer = Identity::generate().fingerprint();
        let runtime = account_runtime(files.path(), &peer, false);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (hello_tx, hello_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut hello = [0u8; 9];
            socket.read_exact(&mut hello).unwrap();
            hello_tx.send(()).unwrap();
            // Deliberately never respond to Noise: cancellation must wake the
            // blocked client rather than wait its ten-second handshake budget.
            let mut bytes = Vec::new();
            let result = socket.read_to_end(&mut bytes);
            closed_tx.send(result).unwrap();
        });
        let sender = Arc::new(
            Sender::start(
                runtime.identity.clone(),
                runtime.trust.clone(),
                peer.clone(),
                address,
            )
            .unwrap(),
        );
        *crate::CONTROL.lock().unwrap() = Some(sender.clone());
        hello_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(sender.snapshot()["state"], "connecting");
        assert_eq!(sender.snapshot()["peer"], peer);
        runtime.account.trust.clear();
        let started = Instant::now();
        crate::accounts::cancel_retired(&runtime, BTreeMap::from([(peer, "Remote".into())]))
            .unwrap();
        assert_eq!(sender.snapshot()["state"], "closed");
        closed_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        wait_for(|| sender.state.0.lock().unwrap().socket.is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }
    #[test]
    fn pinned_ordered_input_cancel_and_wrong_identity() {
        let files = tempfile::tempdir().unwrap();
        let local = Arc::new(Identity::generate());
        let remote = Arc::new(Identity::generate());
        let a = TrustStore::open(files.path().join("a")).unwrap();
        let b = TrustStore::open(files.path().join("b")).unwrap();
        a.remember(&remote.fingerprint()).unwrap();
        b.remember(&local.fingerprint()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let log = events.clone();
        let server_id = remote.clone();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let _ = session::serve_connection_with_cursor(
                socket,
                &server_id,
                &b,
                &mut None,
                |_, _| Decision::Once,
                &mut Sink(log),
            );
        });
        let sender =
            Sender::start(local.clone(), a.clone(), remote.fingerprint(), address).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while sender.snapshot()["state"] == "connecting" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sender.snapshot()["state"], "connected");
        assert!(sender.enqueue("stale", r#"{"kind":"release"}"#).is_err());
        assert!(sender
            .enqueue(&sender.id, r#"{"kind":"move","x":2,"y":0}"#)
            .is_err());
        for value in [
            r#"{"kind":"move","x":0.25,"y":0.75}"#,
            r#"{"kind":"button","button":0,"down":true,"clicks":1}"#,
            r#"{"kind":"scroll","dx":0,"dy":30}"#,
            r#"{"kind":"mac_key","code":0,"down":true,"repeat":false}"#,
        ] {
            sender.enqueue(&sender.id, value).unwrap();
        }
        while sender.snapshot()["sent"].as_u64().unwrap() < 4 || events.lock().unwrap().len() < 5 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        sender.cancel("stale");
        assert_eq!(sender.snapshot()["state"], "connected");
        sender.cancel(&sender.id);
        drop(sender);
        server.join().unwrap();
        let values = events.lock().unwrap();
        assert_eq!(values[0], "start");
        assert_eq!(values[1], "move 0.25 0.75");
        assert!(values[2].contains("button"));
        assert!(values[3].contains("scroll"));
        assert!(values[4].contains("mac_key"));
        assert_eq!(values.last().unwrap(), "released");
        drop(values);
        let unknown = Identity::generate();
        assert!(Sender::start(local.clone(), a.clone(), unknown.fingerprint(), address).is_err());
        // A remembered pin still cannot authenticate a different identity at its address.
        let wrong = Identity::generate();
        let bad = TrustStore::open(files.path().join("bad")).unwrap();
        bad.remember(&local.fingerprint()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let log = events.clone();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let _ = session::serve_connection_with_cursor(
                socket,
                &wrong,
                &bad,
                &mut None,
                |_, _| Decision::Once,
                &mut Sink(log),
            );
        });
        let sender = Sender::start(local, a, remote.fingerprint(), address).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while sender.snapshot()["state"] == "connecting" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sender.snapshot()["state"], "failed");
        drop(sender);
        server.join().unwrap();
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|v| *v == "start")
                .count(),
            1
        );
    }
}
