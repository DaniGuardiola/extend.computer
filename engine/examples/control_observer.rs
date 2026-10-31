//! Bounded encrypted control observer. Never captures or injects operating-system input.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    identity::Identity,
    input::InputEvent,
    pairing::PairingWindow,
    session::{self, CursorSink, Decision},
    trust::TrustStore,
};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

const LIMIT: usize = 256;
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    anyhow::bail!("Private observer receipts require a Unix host");
    Ok(options.open(path)?)
}
struct Observer {
    expected: String,
    session: usize,
    file: Arc<Mutex<File>>,
    events: Arc<Mutex<usize>>,
    started: Instant,
    active: bool,
    granted: bool,
}
impl Observer {
    fn record(&self, kind: &str, detail: Value) -> Result<()> {
        let mut file = self.file.lock().unwrap();
        serde_json::to_writer(
            &mut *file,
            &json!({"kind":kind,"session":self.session,
            "elapsed_ms":self.started.elapsed().as_millis(),"detail":detail}),
        )?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }
    fn grant(&mut self, peer: &str) -> Result<bool> {
        ensure!(peer == self.expected, "Unexpected observer peer");
        ensure!(!self.active, "Observer already granted");
        self.active = true;
        self.granted = true;
        self.record("grant", json!({"peer":peer,"text_supported":true}))?;
        Ok(true)
    }
}
impl CursorSink for Observer {
    fn supports_text_input(&self) -> bool {
        self.active
    }
    fn start_cursor(&mut self, _: &str) -> Result<bool> {
        Ok(false)
    }
    fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
        ensure!(self.active, "Input is not granted");
        Ok(())
    }
    fn start_control(&mut self, peer: &str) -> Result<bool> {
        self.grant(peer)
    }
    fn start_input(&mut self, peer: &str) -> Result<bool> {
        self.grant(peer)
    }
    fn input(&mut self, event: &InputEvent) -> Result<()> {
        ensure!(self.active, "Input is not granted");
        event.validate()?;
        let mut count = self.events.lock().unwrap();
        ensure!(*count < LIMIT, "Observer event limit reached");
        self.record("input", json!({"sequence":*count+1,"event":event}))?;
        *count += 1;
        Ok(())
    }
    fn device_name(&self) -> Option<String> {
        Some("IME observer".into())
    }
    fn finish(&mut self) {
        let was_active = std::mem::replace(&mut self.active, false);
        let _ = self.record("finish", json!({"had_grant":was_active}));
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 4,
        "usage: control_observer EXPECTED_ANDROID_FINGERPRINT PAIR_JSON EVENTS_JSONL"
    );
    let expected = args[1].clone();
    ensure!(
        expected.len() == 64
            && expected
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Invalid expected fingerprint"
    );
    let pairing_path = PathBuf::from(&args[2]);
    let mut pairing_file = private_file(&pairing_path)?;
    let file = Arc::new(Mutex::new(private_file(Path::new(&args[3]))?));
    let temporary = tempfile::tempdir()?;
    let identity = Identity::generate();
    let trust = TrustStore::open(temporary.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let window = PairingWindow::new(Duration::from_secs(120));
    serde_json::to_writer(
        &mut pairing_file,
        &json!({"address":listener.local_addr()?.to_string(),
        "fingerprint":identity.fingerprint(),"expected_peer":expected,"code":window.code(),
        "expires_after_seconds":120,"pid":std::process::id()}),
    )?;
    pairing_file.write_all(b"\n")?;
    pairing_file.flush()?;
    drop(pairing_file);
    println!(
        "observer ready; private pairing receipt: {}",
        pairing_path.display()
    );
    let socket = Arc::new(Mutex::new(None::<TcpStream>));
    let expired = Arc::new(AtomicBool::new(false));
    let (done, wait) = mpsc::channel::<()>();
    let watched = socket.clone();
    let timed = expired.clone();
    let timer = std::thread::spawn(move || {
        if wait.recv_timeout(Duration::from_secs(120)).is_err() {
            timed.store(true, Ordering::SeqCst);
            if let Some(socket) = watched.lock().unwrap().as_ref() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    });
    let events = Arc::new(Mutex::new(0));
    let started = Instant::now();
    let mut window = Some(window);
    let result = (|| -> Result<()> {
        // Health probes may precede the PAKE/name exchange and pinned control.
        // Bound all accepted sockets independently of the two relevant sessions.
        let mut relevant = 0;
        for index in 1..=8 {
            let stream = loop {
                ensure!(!expired.load(Ordering::SeqCst), "Observer deadline expired");
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            stream.set_nonblocking(false)?;
            *socket.lock().unwrap() = Some(stream.try_clone()?);
            // The deadline thread may expire between accept and publishing this socket.
            ensure!(!expired.load(Ordering::SeqCst), "Observer deadline expired");
            let paired_before = trust.peer(&expected)?.is_some();
            let mut observer = Observer {
                expected: expected.clone(),
                session: index,
                file: file.clone(),
                events: events.clone(),
                started,
                active: false,
                granted: false,
            };
            observer.record("session_open", json!({"expected_peer":expected}))?;
            let served = if !paired_before {
                session::serve_connection_with_cursor(
                    stream,
                    &identity,
                    &trust,
                    &mut window,
                    |peer, _| {
                        if peer == expected {
                            Decision::Remember
                        } else {
                            Decision::Deny
                        }
                    },
                    &mut observer,
                )
            } else {
                session::serve_connection_with_cursor_pinned(
                    stream,
                    &identity,
                    &trust,
                    &expected,
                    &mut observer,
                )
            };
            socket.lock().unwrap().take();
            observer.record("session_closed", json!({"ok":served.is_ok()}))?;
            served.context("Observer session ended with protocol error")?;
            let paired_after = trust.peer(&expected)?.is_some();
            if !paired_before && paired_after {
                relevant += 1;
                observer.record(
                    "paired",
                    json!({"peer":expected,"relevant_sessions":relevant}),
                )?;
            }
            if observer.granted {
                relevant += 1;
                observer.record(
                    "control_complete",
                    json!({"peer":expected,"relevant_sessions":relevant}),
                )?;
                ensure!(
                    paired_after && relevant == 2,
                    "Control completed without expected pairing"
                );
                return Ok(());
            }
        }
        anyhow::bail!("Observer accepted-connection limit reached before control")
    })();
    let _ = done.send(());
    let _ = timer.join();
    let _ = std::fs::remove_file(pairing_path);
    result
}

// Receipts use Unix private-file permissions; other hosts refuse them.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn grant_pin_text_exact_and_event_cap_without_os_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut sink = Observer {
            expected: "a".repeat(64),
            session: 2,
            file: Arc::new(Mutex::new(private_file(&path).unwrap())),
            events: Arc::new(Mutex::new(0)),
            started: Instant::now(),
            active: false,
            granted: false,
        };
        assert!(!sink.supports_text_input());
        assert!(sink.grant(&"b".repeat(64)).is_err());
        assert!(sink.input(&InputEvent::Release).is_err());
        sink.grant(&"a".repeat(64)).unwrap();
        let text = "Unicode e\u{301} café 👩‍💻 中文测试 日本語 ✅";
        sink.input(&InputEvent::Text { value: text.into() })
            .unwrap();
        for _ in 1..LIMIT {
            sink.input(&InputEvent::Release).unwrap();
        }
        assert!(sink.input(&InputEvent::Release).is_err());
        sink.finish();
        assert!(!sink.supports_text_input());
        assert!(sink.input(&InputEvent::Release).is_err());
        let lines: Vec<Value> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines[1]["detail"]["event"]["value"], text);
        assert_eq!(
            lines.iter().filter(|row| row["kind"] == "input").count(),
            LIMIT
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(private_file(&path).is_err());
    }
}
