//! Native helper boundary. Cursor and full-input sessions have distinct consent.
use crate::error::EngineError;
use crate::session::{Client, CursorSink, DisplaySize};
use anyhow::{ensure, Context, Result};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) struct Helper(pub(crate) Child, Option<DisplaySize>);
impl Drop for Helper {
    fn drop(&mut self) {
        // Let the desktop launcher observe EOF and remove its temporary job.
        drop(self.0.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub(crate) fn spawn(path: &Path, mode: &str, arguments: &[String]) -> Result<Helper> {
    let mut child = Helper(
        Command::new(path)
            .arg(mode)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context(EngineError::HelperUnavailable)?,
        None,
    );
    let mut ready = String::new();
    // Helper emits exactly one startup line before reading input or capturing.
    BufReader::new(child.0.stdout.as_mut().unwrap()).read_line(&mut ready)?;
    let fields: Vec<_> = ready.split_whitespace().collect();
    ensure!(
        fields.len() == 3 && fields[0] == "READY",
        EngineError::HelperUnavailable
    );
    child.1 = Some(
        DisplaySize {
            width: fields[1].parse()?,
            height: fields[2].parse()?,
        }
        .validate()?,
    );
    Ok(child)
}
pub struct NativeSink {
    path: Option<PathBuf>,
    helper: Option<Helper>,
    trace: crate::timing::Trace,
    updates: u64,
}
impl NativeSink {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            helper: None,
            trace: crate::timing::Trace::from_env(),
            updates: 0,
        }
    }
}
impl NativeSink {
    /// Start only after the embedding UI has obtained control consent.
    /// This boundary deliberately performs no stdin interaction.
    pub fn start_approved_control(&mut self) -> Result<()> {
        let path = self.path.as_ref().context(EngineError::HelperUnavailable)?;
        self.helper = Some(spawn(path, "inject-control", &[])?);
        Ok(())
    }
    fn command(&mut self, line: &str) -> Result<()> {
        let child = &mut self
            .helper
            .as_mut()
            .context(EngineError::HelperUnavailable)?
            .0;
        ensure!(child.try_wait()?.is_none(), "native helper stopped");
        writeln!(
            child.stdin.as_mut().context("helper input closed")?,
            "{line}"
        )?;
        let mut reply = String::new();
        BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut reply)?;
        ensure!(reply.trim() == "OK", "native input failed");
        Ok(())
    }
}
impl CursorSink for NativeSink {
    fn approve_control(
        &mut self,
        peer: &str,
        remembered: bool,
    ) -> Result<crate::session::ControlApproval> {
        use crate::session::ControlApproval;
        let Some(path) = &self.path else {
            return Ok(ControlApproval::Deny);
        };
        let approval = if remembered {
            eprintln!("Previously approved peer {peer} requests control until disconnect. Ctrl-C stops receiver.");
            ControlApproval::Once
        } else {
            eprintln!("Peer {peer} requests full control until disconnect. Ctrl-C stops receiver.");
            eprint!("Type allow-control (once), always-control (remember this device), or deny: ");
            std::io::stderr().flush()?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            match answer.trim() {
                "allow-control" => ControlApproval::Once,
                "always-control" => ControlApproval::Remember,
                _ => return Ok(ControlApproval::Deny),
            }
        };
        self.helper = Some(spawn(path, "inject-control", &[])?);
        Ok(approval)
    }

    fn approve_input(&mut self, peer: &str) -> Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        eprintln!("Peer {peer} requests MOUSE, SCROLLING, AND KEYBOARD CONTROL for 30 seconds. Ctrl-C stops receiver.");
        eprint!("Type allow-input to approve this session: ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if answer.trim() != "allow-input" {
            return Ok(false);
        }
        self.helper = Some(spawn(path, "inject-input", &[])?);
        Ok(true)
    }
    fn input(&mut self, event: &crate::input::InputEvent) -> Result<()> {
        self.command(&serde_json::to_string(event)?)
    }
    fn heartbeat(&mut self) -> Result<()> {
        self.command("ALIVE")
    }

    fn approve(&mut self, peer: &str) -> Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        eprintln!("Peer {peer} requests CURSOR MOVEMENT on this Mac for 30 seconds. No clicks or keys. Ctrl-C stops receiver.");
        eprint!("Type allow-cursor to approve this session: ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if answer.trim() != "allow-cursor" {
            return Ok(false);
        }
        self.helper = Some(spawn(path, "inject", &[])?);
        Ok(true)
    }
    fn display_size(&self) -> Result<DisplaySize> {
        self.helper
            .as_ref()
            .and_then(|h| h.1)
            .context("native display size unavailable")
    }
    fn finish(&mut self) {
        self.helper.take();
    }
    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.trace.record("inject_start", self.updates, 0);
        let child = &mut self.helper.as_mut().context("cursor not approved")?.0;
        ensure!(child.try_wait()?.is_none(), "native helper stopped");
        let input = child.stdin.as_mut().context("helper input closed")?;
        writeln!(input, "{x} {y}")?;
        input.flush()?;
        // Acknowledgment means native injection executed, not merely queued in a pipe.
        let mut reply = String::new();
        BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut reply)?;
        ensure!(reply.trim() == "OK", "native injection failed");
        self.trace.record("inject_end", self.updates, 0);
        self.updates += 1;
        Ok(())
    }
}

pub fn send(client: Client, path: &Path) -> Result<()> {
    send_with_policy(client, path, true, "left", 0.0)
}
pub fn send_with_policy(
    mut client: Client,
    path: &Path,
    low_jitter: bool,
    edge: &str,
    offset_y: f64,
) -> Result<()> {
    ensure!(
        matches!(edge, "left" | "right") && offset_y.is_finite(),
        "invalid edge layout"
    );
    let display = client.request_cursor()?;
    let _lease = crate::low_jitter::start_for_peer(client.address().ip(), low_jitter);
    let mut helper = spawn(
        path,
        &format!("capture-edge-{edge}"),
        &[
            display.width.to_string(),
            display.height.to_string(),
            offset_y.to_string(),
        ],
    )?;
    let output = helper.0.stdout.take().unwrap();
    let latest = Arc::new(Mutex::new(None));
    let writer = latest.clone();
    let trace = crate::timing::Trace::from_env();
    let capture_trace = trace.clone();
    let reader = std::thread::spawn(move || {
        for (capture_id, line) in BufReader::new(output).lines().enumerate() {
            let capture_id = capture_id as u64;
            let Ok(line) = line else { break };
            if line == "STOP" {
                break;
            }
            if line == "LOCAL" {
                *writer.lock().unwrap() = None;
                eprintln!("Control returned to this Mac.");
                continue;
            }
            if line == "REMOTE" {
                eprintln!("Cursor control moved to peer. Control-Option-Escape stops.");
                continue;
            }
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 2 {
                break;
            }
            let (Ok(x), Ok(y)) = (fields[0].parse::<f64>(), fields[1].parse::<f64>()) else {
                break;
            };
            capture_trace.record("capture_arrival", capture_id, 0);
            *writer.lock().unwrap() = Some((x, y, capture_id, Instant::now()));
        }
    });
    eprintln!("Edge handoff active for at most 30 seconds: peer {edge}, vertical offset {offset_y} points. Control-Option-Escape or Ctrl-C stops. Cursor only; keyboard stays local, remote clicks/scrolling blocked.");
    let start = Instant::now();
    let mut heartbeat = Instant::now();
    let mut sent = 0;
    let mut pending = 0;
    let result = (|| -> Result<()> {
        while start.elapsed() < Duration::from_secs(29) {
            if reader.is_finished() {
                break;
            }
            // Flush before taking the newest sample, never replay samples collected
            // while waiting for the network. At most four positions are in flight.
            if pending == 4 || (pending > 0 && heartbeat.elapsed() >= Duration::from_millis(16)) {
                trace.record("ack_start", sent, 0);
                client.probe()?;
                writeln!(
                    helper.0.stdin.as_mut().context("capture input closed")?,
                    "ALIVE"
                )?;
                trace.record("ack_end", sent, 0);
                pending = 0;
                heartbeat = Instant::now();
            }
            let point = latest.lock().unwrap().take();
            if let Some((x, y, capture_id, captured)) = point {
                trace.record("sample", sent, capture_id as u128);
                trace.record("send_start", sent, captured.elapsed().as_micros());
                client.queue_cursor(x, y)?;
                trace.record("send_end", sent, 0);
                sent += 1;
                pending += 1;
            } else if heartbeat.elapsed() > Duration::from_secs(1) {
                trace.record("ack_start", sent, 0);
                client.probe()?;
                writeln!(
                    helper.0.stdin.as_mut().context("capture input closed")?,
                    "ALIVE"
                )?;
                trace.record("ack_end", sent, 0);
                heartbeat = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        if pending > 0 {
            client.probe()?;
        }
        Ok(())
    })();
    drop(helper);
    let _ = reader.join();
    result?;
    println!("cursor_updates={sent}");
    client.close()
}
