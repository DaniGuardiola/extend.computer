//! Opt-in delivery diagnostic. Uses the application's real USB session adapter,
//! but never starts an event tap or observes local keyboard/mouse activity.
use super::*;
use extend_computer_agent::{
    display::usb_stream::UsbService,
    input::InputEvent,
    session::{Client, Decision, DisplaySize},
};
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Write},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
};

const UNICODE: &str = "Café é — 中文测试 👩‍💻 🧑🏽‍🚀";
const MAX_COMMANDS: u64 = 16;
const MAX_LINE: usize = 1024;
const LIFETIME: Duration = Duration::from_secs(90);
const HEARTBEAT: Duration = Duration::from_millis(200);

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Click { sequence: u64, x: f64, y: f64 },
    Type { sequence: u64 },
    Unicode { sequence: u64 },
    Hold { sequence: u64, x: f64, y: f64 },
    Close { sequence: u64 },
}
impl Command {
    fn sequence(&self) -> u64 {
        match self {
            Self::Click { sequence, .. }
            | Self::Type { sequence }
            | Self::Unicode { sequence }
            | Self::Hold { sequence, .. }
            | Self::Close { sequence } => *sequence,
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Self::Click { .. } => "click",
            Self::Type { .. } => "type",
            Self::Unicode { .. } => "unicode",
            Self::Hold { .. } => "hold",
            Self::Close { .. } => "closed",
        }
    }
    fn validate(&self, previous: u64, held: bool) -> Result<()> {
        ensure!(
            self.sequence() == previous + 1 && self.sequence() <= MAX_COMMANDS,
            "Invalid diagnostic command sequence"
        );
        ensure!(
            !held || matches!(self, Self::Close { .. }),
            "Close the diagnostic after holding a contact"
        );
        if let Self::Click { x, y, .. } | Self::Hold { x, y, .. } = self {
            ensure!(
                x.is_finite()
                    && y.is_finite()
                    && (0.0..=1.0).contains(x)
                    && (0.0..=1.0).contains(y),
                "Invalid diagnostic target point"
            );
        }
        Ok(())
    }
}

// fill_buf is capped before copying: malformed stdin cannot allocate an
// unbounded line or accumulate an unbounded event queue.
fn read_command(input: &mut impl BufRead) -> Result<Option<Command>> {
    let mut line = Vec::new();
    loop {
        let bytes = input.fill_buf()?;
        if bytes.is_empty() {
            ensure!(line.is_empty(), "Incomplete diagnostic command");
            return Ok(None);
        }
        let count = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|n| n + 1)
            .unwrap_or(bytes.len());
        ensure!(
            line.len() + count <= MAX_LINE,
            "Diagnostic command too large"
        );
        let complete = bytes[count - 1] == b'\n';
        line.extend_from_slice(&bytes[..count]);
        input.consume(count);
        if complete {
            return Ok(Some(serde_json::from_slice(&line)?));
        }
    }
}
fn read_commands(
    input: &mut impl BufRead,
    commands: SyncSender<Result<Command>>,
    overflow: Arc<AtomicBool>,
) {
    loop {
        let command = match read_command(input) {
            Ok(Some(command)) => Ok(command),
            Ok(None) => return,
            Err(error) => Err(error),
        };
        let terminal = command.is_err();
        match commands.try_send(command) {
            Ok(()) if !terminal => {}
            Ok(()) | Err(TrySendError::Disconnected(_)) => return,
            Err(TrySendError::Full(_)) => {
                overflow.store(true, Ordering::Release);
                return;
            }
        }
    }
}

struct ReceiptFile(PathBuf);
impl ReceiptFile {
    fn create(path: PathBuf, value: &serde_json::Value) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "Use an absolute diagnostic receipt path"
        );
        let parent = path.parent().context("Missing receipt directory")?;
        let metadata = std::fs::symlink_metadata(parent)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Invalid receipt directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "Receipt directory must be private"
            );
        }
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.persist_noclobber(&path)?;
        Ok(Self(path))
    }
    fn save(&self, value: &serde_json::Value) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(self.0.parent().unwrap())?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.persist(&self.0)?;
        Ok(())
    }
}

fn apply(client: &mut Client, command: &Command) -> Result<()> {
    match command {
        Command::Click { x, y, .. } | Command::Hold { x, y, .. } => {
            client.queue_cursor(*x, *y)?;
            client.queue_input(InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            })?;
            if matches!(command, Command::Click { .. }) {
                client.queue_input(InputEvent::Button {
                    button: 0,
                    down: false,
                    clicks: 1,
                })?;
            }
        }
        Command::Type { .. } => {
            for code in [14, 7, 17, 14, 45, 2] {
                for down in [true, false] {
                    client.queue_input(InputEvent::MacKey {
                        code,
                        down,
                        repeat: false,
                    })?;
                }
                // The receiver permits at most four unacknowledged input
                // events. Each physical pair has its own authenticated ACK.
                client.probe()?;
            }
        }
        Command::Unicode { .. } => {
            ensure!(
                client.supports_text_input(),
                "Receiver did not grant committed text"
            );
            client.queue_input(InputEvent::Text {
                value: UNICODE.into(),
            })?;
        }
        Command::Close { .. } => {
            client.queue_input(InputEvent::Release)?;
        }
    }
    // This confirms authenticated processing, never native editor delivery.
    client.probe()?;
    Ok(())
}

fn initial_receipt(peer: &str, local: &str) -> serde_json::Value {
    serde_json::json!({
        "status":"diagnostic-only", "state":"starting", "route":"usb", "peer":peer,
        "local_identity":local, "command_sequence":0,
        "text_supported":false, "unicode_fixture":UNICODE,
        "unicode_utf16_units":UNICODE.encode_utf16().count(), "native_delivery_verified":false,
        "global_input_capture":false,
    })
}

impl Desktop {
    pub fn start_usb_control_test(
        self: &Arc<Self>,
        peer: String,
        vendor: String,
        serial: String,
        path: PathBuf,
    ) -> Result<()> {
        ensure!(
            peer.len() == 64
                && peer
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "Invalid paired device identity"
        );
        ensure!(
            self.store()?.peer(&peer)?.is_some(),
            "Pair this device first"
        );
        let mut receipt = initial_receipt(&peer, &self.identity()?.fingerprint());
        let receipt_file = ReceiptFile::create(path, &receipt)?;
        let (tx, rx) = mpsc::sync_channel(MAX_COMMANDS as usize);
        let overflow = Arc::new(AtomicBool::new(false));
        let reader_overflow = overflow.clone();
        std::thread::Builder::new()
            .name("usb-control-test-stdin".into())
            .spawn(move || {
                read_commands(&mut BufReader::new(std::io::stdin()), tx, reader_overflow);
            })?;
        let app = self.clone();
        std::thread::Builder::new()
            .name("usb-control-test".into())
            .spawn(move || {
                let startup = (|| -> Result<()> {
                    app.connect_usb(&peer, &vendor, &serial)?;
                    app.run_usb_control_test(&peer, &rx, &overflow, &receipt_file, &mut receipt)
                })();
                if startup.is_err()
                    && receipt["state"] != "disconnected"
                    && receipt["state"] != "expired"
                {
                    receipt["state"] = "failed".into();
                    // Do not serialize remote errors, URLs, credentials or typed input.
                    receipt["reason"] = "diagnostic-gate-failed".into();
                    let _ = receipt_file.save(&receipt);
                }
            })?;
        Ok(())
    }

    fn run_usb_control_test(
        self: &Arc<Self>,
        peer: &str,
        commands: &Receiver<Result<Command>>,
        overflow: &AtomicBool,
        file: &ReceiptFile,
        receipt: &mut serde_json::Value,
    ) -> Result<()> {
        let route = self
            .selected_usb_route(peer)
            .context("Select this paired USB device first")?;
        ensure!(route.current(self), "USB carrier changed");
        let (id, cancelled) = self.reserve(SessionKind::Outgoing, Some(peer.into()))?;
        receipt["session"] = id.into();
        let deadline = Instant::now() + LIFETIME;
        let done = Arc::new(AtomicBool::new(false));
        let timed_out = Arc::new(AtomicBool::new(false));
        let weak = Arc::downgrade(self);
        let monitor_done = done.clone();
        let monitor_timeout = timed_out.clone();
        // Shutdown wakes a blocked handshake/probe too. Retire only this job;
        // a late watchdog can never disconnect a replacement session.
        let watchdog = std::thread::spawn(move || {
            while !monitor_done.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    monitor_timeout.store(true, Ordering::Release);
                    if let Some(app) = weak.upgrade() {
                        let mut inner = app.inner.lock().unwrap();
                        if let Some(job) = inner.job.as_mut().filter(|job| job.view.id == id) {
                            job.cancelled.store(true, Ordering::Release);
                            job.view.advance(Phase::Disconnecting);
                            if let Some(socket) = &job.socket {
                                let _ = socket.shutdown(Shutdown::Both);
                            }
                        }
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let result = (|| -> Result<()> {
            let socket = route.open(self, UsbService::Control, &cancelled)?;
            self.set_socket_route(id, &socket, Some("usb"))?;
            let identity = self.identity()?;
            let mut client = Client::connect(
                socket,
                &identity,
                &self.store()?,
                None,
                Some(peer),
                |_, _| Decision::Once,
            )?;
            let DisplaySize { width, height } = client.request_control()?;
            ensure!(
                !cancelled.load(Ordering::Acquire) && route.current(self),
                "Control cancelled"
            );
            self.stage(id, Phase::Connected, None);
            receipt["state"] = "connected".into();
            receipt["authenticated_peer"] = client.peer().into();
            receipt["text_supported"] = client.supports_text_input().into();
            receipt["width"] = width.into();
            receipt["height"] = height.into();
            file.save(receipt)?;
            let mut sequence = 0;
            let mut held = false;
            let mut typed = false;
            let mut committed = false;
            loop {
                ensure!(
                    !cancelled.load(Ordering::Acquire) && route.current(self),
                    "Control cancelled"
                );
                ensure!(
                    !overflow.load(Ordering::Acquire),
                    "Diagnostic command queue full"
                );
                let command = match commands.recv_timeout(HEARTBEAT) {
                    Ok(command) => command?,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        client.probe()?;
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        client.queue_input(InputEvent::Release)?;
                        client.probe()?;
                        client.close()?;
                        receipt["state"] = "closed".into();
                        receipt["reason"] = "stdin-eof".into();
                        return file.save(receipt);
                    }
                };
                ensure!(
                    !cancelled.load(Ordering::Acquire) && route.current(self),
                    "Control cancelled"
                );
                command.validate(sequence, held)?;
                ensure!(
                    !matches!(command, Command::Type { .. }) || !typed,
                    "ASCII sample already sent"
                );
                ensure!(
                    !matches!(command, Command::Unicode { .. }) || !committed,
                    "Unicode sample already sent"
                );
                apply(&mut client, &command)?;
                ensure!(
                    !cancelled.load(Ordering::Acquire) && route.current(self),
                    "Control cancelled"
                );
                sequence = command.sequence();
                receipt["command_sequence"] = sequence.into();
                receipt["state"] = command.kind().into();
                file.save(receipt)?;
                held |= matches!(command, Command::Hold { .. });
                typed |= matches!(command, Command::Type { .. });
                committed |= matches!(command, Command::Unicode { .. });
                if matches!(command, Command::Close { .. }) {
                    client.close()?;
                    return Ok(());
                }
            }
        })();
        done.store(true, Ordering::Release);
        let _ = watchdog.join();
        if timed_out.load(Ordering::Acquire) {
            receipt["state"] = "expired".into();
            receipt["reason"] = "lifetime".into();
        } else if cancelled.load(Ordering::Acquire) {
            receipt["state"] = "disconnected".into();
            receipt["reason"] = "session-cancelled".into();
        } else if result.is_err() {
            receipt["state"] = "failed".into();
            receipt["reason"] = "diagnostic-gate-failed".into();
        }
        let saved = file.save(receipt);
        let failed = result.is_err() && !cancelled.load(Ordering::Acquire);
        self.finish(
            id,
            if cancelled.load(Ordering::Acquire) {
                Ok(())
            } else {
                result
            },
        );
        saved?;
        ensure!(!failed, "Diagnostic gate failed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stdin_is_strict_bounded_and_ordered() {
        let mut reader =
            std::io::Cursor::new(b"{\"kind\":\"click\",\"sequence\":1,\"x\":0.2,\"y\":0.3}\n");
        read_command(&mut reader)
            .unwrap()
            .unwrap()
            .validate(0, false)
            .unwrap();
        for value in [
            "{\"kind\":\"unicode\",\"sequence\":1,\"text\":\"private\"}\n",
            "{\"kind\":\"move\",\"sequence\":1,\"x\":0.2,\"y\":0.3}\n",
            "{\"kind\":\"type\",\"sequence\":1}",
        ] {
            assert!(read_command(&mut std::io::Cursor::new(value)).is_err());
        }
        assert!(read_command(&mut std::io::Cursor::new(vec![b' '; MAX_LINE + 1])).is_err());
        assert!(Command::Type { sequence: 1 }.validate(1, false).is_err());
        assert!(Command::Type { sequence: 1 }.validate(0, true).is_err());
        assert!(Command::Close { sequence: 1 }.validate(0, true).is_ok());
        assert!(Command::Click {
            sequence: 1,
            x: f64::NAN,
            y: 0.
        }
        .validate(0, false)
        .is_err());
        assert!(Command::Hold {
            sequence: 1,
            x: 1.01,
            y: 0.
        }
        .validate(0, false)
        .is_err());
    }
    #[test]
    fn receipt_is_private_atomic_and_never_replaces_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let path = directory.path().join("receipt.json");
        let first = serde_json::json!({"state":"connected"});
        let file = ReceiptFile::create(path.clone(), &first).unwrap();
        assert!(ReceiptFile::create(path.clone(), &serde_json::json!({})).is_err());
        file.save(&serde_json::json!({"state":"closed"})).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap()
                ["state"],
            "closed"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn full_queue_fails_instead_of_blocking_or_dropping_a_transition() {
        let (tx, rx) = mpsc::sync_channel(1);
        let overflow = Arc::new(AtomicBool::new(false));
        let mut reader = std::io::Cursor::new(
            b"{\"kind\":\"type\",\"sequence\":1}\n{\"kind\":\"close\",\"sequence\":2}\n",
        );
        read_commands(&mut reader, tx, overflow.clone());
        assert!(overflow.load(Ordering::Acquire));
        assert!(matches!(
            rx.try_recv().unwrap().unwrap(),
            Command::Type { sequence: 1 }
        ));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn real_usb_pinned_delivery_and_gui_disconnect_without_capture_or_lan() {
        use extend_computer_agent::{
            display::usb_link::{run_io, UsbHub},
            session::{self, CursorSink},
        };
        use std::{net::TcpListener, os::unix::fs::PermissionsExt};

        #[derive(Default)]
        struct Received {
            starts: usize,
            finishes: usize,
            held: bool,
            events: Vec<InputEvent>,
            points: Vec<(f64, f64)>,
        }
        struct Sink(Arc<Mutex<Received>>);
        impl CursorSink for Sink {
            fn supports_text_input(&self) -> bool {
                true
            }
            fn start_control(&mut self, _: &str) -> Result<bool> {
                self.0.lock().unwrap().starts += 1;
                Ok(true)
            }
            fn display_size(&self) -> Result<DisplaySize> {
                Ok(DisplaySize {
                    width: 1000.,
                    height: 800.,
                })
            }
            fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
                self.0.lock().unwrap().points.push((x, y));
                Ok(())
            }
            fn input(&mut self, event: &InputEvent) -> Result<()> {
                let mut received = self.0.lock().unwrap();
                if let InputEvent::Button { down, .. } = event {
                    received.held = *down;
                }
                if matches!(event, InputEvent::Release) {
                    received.held = false;
                }
                received.events.push(event.clone());
                Ok(())
            }
            fn finish(&mut self) {
                let mut received = self.0.lock().unwrap();
                received.held = false;
                received.finishes += 1;
            }
        }
        fn wait(check: impl Fn() -> bool) {
            let until = Instant::now() + Duration::from_secs(10);
            while !check() {
                assert!(Instant::now() < until, "USB diagnostic did not settle");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        fn state(path: &std::path::Path) -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        }

        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let raw = TcpListener::bind("127.0.0.1:0").unwrap();
        let helper = directory
            .path()
            .join("helpers/ExtendComputerUsb/ExtendComputerUsb");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(
            &helper,
            format!(
                r#"#!/usr/bin/python3
import socket,select,sys
left=socket.socket(socket.AF_UNIX); left.connect(sys.argv[4])
right=socket.create_connection(('127.0.0.1',{}))
while True:
 ready,_,_=select.select([left,right],[],[],1)
 for src in ready:
  data=src.recv(37)
  if not data: sys.exit(0)
  (right if src is left else left).sendall(data)
"#,
                raw.local_addr().unwrap().port()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Missing native input helper: this test cannot accidentally pass by
        // starting capture-control, an event tap or a LAN input listener.
        let app = Desktop::new(
            directory.path().join("app"),
            directory
                .path()
                .join("helpers/Cursor.app/Contents/MacOS/missing-cursor"),
        )
        .unwrap();
        app.set_test_identity(Identity::generate());
        let local = app.identity().unwrap().fingerprint();
        let remote = Identity::generate();
        let peer = remote.fingerprint();
        app.store().unwrap().remember(&peer).unwrap();
        let remote_trust = TrustStore::open(directory.path().join("remote")).unwrap();
        remote_trust.remember(&local).unwrap();
        let (hub, driver) =
            UsbHub::new("192.0.2.2".parse().unwrap(), "192.0.2.1".parse().unwrap()).unwrap();
        hub.enable_streams().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let carrier = std::thread::spawn(move || {
            let stream = raw.accept().unwrap().0;
            stream.set_nonblocking(true).unwrap();
            run_io(driver, stream.try_clone().unwrap(), stream, &stop)
        });
        let received = Arc::new(Mutex::new(Received::default()));
        let server_received = received.clone();
        let server_hub = hub.clone();
        let server = std::thread::spawn(move || {
            for expected in [
                UsbService::Presence,
                UsbService::Control,
                UsbService::Control,
            ] {
                let until = Instant::now() + Duration::from_secs(15);
                let (service, socket) = loop {
                    if let Some(stream) = server_hub.accept_stream().unwrap() {
                        break stream;
                    }
                    assert!(Instant::now() < until, "No USB diagnostic stream");
                    std::thread::sleep(Duration::from_millis(5));
                };
                assert_eq!(service, expected);
                if service == UsbService::Presence {
                    session::serve_presence_pinned(socket, &remote, &remote_trust, &local, || {
                        session::Presence {
                            receiving: true,
                            port: 48177,
                            name: "Test tablet".into(),
                        }
                    })
                    .unwrap();
                } else {
                    let _ = session::serve_connection_with_cursor_pinned(
                        socket,
                        &remote,
                        &remote_trust,
                        &local,
                        &mut Sink(server_received.clone()),
                    );
                }
            }
        });
        app.connect_usb(&peer, "0000", "local-fixture").unwrap();
        assert!(
            app.devices.lock().unwrap().is_empty(),
            "Private USB addresses must not be saved as LAN hints"
        );
        for round in 0..2 {
            let path = directory.path().join(format!("receipt-{round}.json"));
            let mut receipt = initial_receipt(&peer, &app.identity().unwrap().fingerprint());
            let file = ReceiptFile::create(path.clone(), &receipt).unwrap();
            let (tx, rx) = mpsc::sync_channel(16);
            let worker = app.clone();
            let pin = peer.clone();
            let thread = std::thread::spawn(move || {
                worker.run_usb_control_test(&pin, &rx, &AtomicBool::new(false), &file, &mut receipt)
            });
            wait(|| state(&path)["state"] == "connected");
            assert_eq!(state(&path)["authenticated_peer"], peer);
            assert_eq!(state(&path)["route"], "usb");
            assert_eq!(state(&path)["text_supported"], true);
            assert_eq!(state(&path)["unicode_utf16_units"], 28);
            assert_eq!(app.snapshot().unwrap().session.unwrap().route, Some("usb"));
            tx.send(Ok(Command::Click {
                sequence: 1,
                x: 0.2,
                y: 0.3,
            }))
            .unwrap();
            tx.send(Ok(Command::Type { sequence: 2 })).unwrap();
            tx.send(Ok(Command::Unicode { sequence: 3 })).unwrap();
            if round == 0 {
                tx.send(Ok(Command::Close { sequence: 4 })).unwrap();
                thread.join().unwrap().unwrap();
                assert_eq!(state(&path)["state"], "closed");
            } else {
                tx.send(Ok(Command::Hold {
                    sequence: 4,
                    x: 0.4,
                    y: 0.5,
                }))
                .unwrap();
                wait(|| state(&path)["state"] == "hold" && received.lock().unwrap().held);
                app.disconnect(); // Exact normal GUI command's implementation.
                thread.join().unwrap().unwrap();
                assert_eq!(state(&path)["state"], "disconnected");
                assert_eq!(state(&path)["command_sequence"], 4);
            }
            wait(|| received.lock().unwrap().finishes == round + 1);
            assert!(!received.lock().unwrap().held);
            assert!(app.snapshot().unwrap().session.is_none());
        }
        server.join().unwrap();
        let received = received.lock().unwrap();
        assert_eq!(received.starts, 2);
        assert_eq!(
            received
                .events
                .iter()
                .filter(|event| matches!(event,InputEvent::Text { value } if value==UNICODE))
                .count(),
            2
        );
        assert_eq!(
            received
                .events
                .iter()
                .filter(|event| matches!(event, InputEvent::MacKey { down: true, .. }))
                .count(),
            12
        );
        assert_eq!(
            received
                .events
                .iter()
                .filter(|event| matches!(event, InputEvent::MacKey { down: false, .. }))
                .count(),
            12
        );
        assert_eq!(received.points, vec![(0.2, 0.3), (0.2, 0.3), (0.4, 0.5)]);
        drop(received);
        assert!(app.devices.lock().unwrap().is_empty());
        assert!(app.store().unwrap().peer(&peer).unwrap().is_some());
        app.disconnect_usb();
        stopped.store(true, Ordering::Release);
        hub.close();
        let _ = carrier.join().unwrap();
    }
}
