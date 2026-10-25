//! Native helper boundary. Starts native resources after peer authentication.
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

pub(crate) struct Helper(
    pub(crate) Child,
    Option<DisplaySize>,
    bool,
    Option<crate::input::ShortcutPolicy>,
);
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
        false,
        None,
    );
    let mut ready = String::new();
    // Helper emits exactly one startup line before reading input or capturing.
    BufReader::new(child.0.stdout.as_mut().unwrap()).read_line(&mut ready)?;
    let fields: Vec<_> = ready.split_whitespace().collect();
    ensure!(
        (3..=5).contains(&fields.len()) && fields[0] == "READY",
        EngineError::HelperUnavailable
    );
    let input_mode = matches!(mode, "inject-control" | "inject-input");
    let mut text = false;
    let mut shortcut = None;
    for token in &fields[3..] {
        match *token {
            crate::protocol::INPUT_TEXT if !text => text = true,
            "input-shortcuts-command-v1" if shortcut.is_none() => {
                shortcut = Some(crate::input::ShortcutPolicy::Command)
            }
            "input-shortcuts-control-v1" if shortcut.is_none() => {
                shortcut = Some(crate::input::ShortcutPolicy::Control)
            }
            _ => anyhow::bail!(EngineError::HelperUnavailable),
        }
    }
    child.2 = input_mode && text;
    child.3 = if input_mode { shortcut } else { None };
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
    fn shortcut_policy(&self) -> Option<crate::input::ShortcutPolicy> {
        self.helper.as_ref().and_then(|helper| helper.3)
    }
    fn supports_text_input(&self) -> bool {
        self.helper.as_ref().is_some_and(|helper| helper.2)
    }
    /// The bundled macOS helper presses right-hand modifier keys.
    fn supports_modifier_sides(&self) -> bool {
        self.helper.is_some()
    }
    fn start_control(&mut self, _: &str) -> Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        self.helper = Some(spawn(path, "inject-control", &[])?);
        Ok(true)
    }
    fn start_input(&mut self, _: &str) -> Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        self.helper = Some(spawn(path, "inject-input", &[])?);
        Ok(true)
    }
    fn input(&mut self, event: &crate::input::InputEvent) -> Result<()> {
        if matches!(event, crate::input::InputEvent::Text { .. }) {
            ensure!(
                self.supports_text_input(),
                "Native helper does not support text"
            );
        }
        self.command(&serde_json::to_string(event)?)
    }
    fn heartbeat(&mut self) -> Result<()> {
        self.command("ALIVE")
    }

    fn start_cursor(&mut self, _: &str) -> Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
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
        let child = &mut self
            .helper
            .as_mut()
            .context("native cursor helper unavailable")?
            .0;
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

#[cfg(all(test, unix))]
mod text_contract_tests {
    use super::*;
    /// Linux refuses to exec a script another test thread's fork still holds
    /// open for writing (ETXTBSY); retry only that race.
    fn retry_busy<T>(mut start: impl FnMut() -> Result<T>) -> Result<T> {
        for _ in 0..20 {
            match start() {
                Err(error)
                    if error.chain().any(|cause| {
                        cause
                            .downcast_ref::<std::io::Error>()
                            .and_then(std::io::Error::raw_os_error)
                            == Some(26)
                    }) =>
                {
                    std::thread::sleep(Duration::from_millis(25))
                }
                result => return result,
            }
        }
        start()
    }
    #[test]
    fn shortcut_helper_policy_is_explicit_and_rejects_unknown_or_duplicate_tokens() {
        use std::os::unix::fs::PermissionsExt;
        for (tokens, expected, valid) in [
            ("", None, true),
            (
                " input-shortcuts-command-v1",
                Some(crate::input::ShortcutPolicy::Command),
                true,
            ),
            (
                " input-text-v1 input-shortcuts-control-v1",
                Some(crate::input::ShortcutPolicy::Control),
                true,
            ),
            (" input-shortcuts-windows-v1", None, false),
            (
                " input-shortcuts-command-v1 input-shortcuts-control-v1",
                None,
                false,
            ),
            (" input-text-v1 input-text-v1", None, false),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("helper");
            std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' 'READY 1920 1080{tokens}'\nwhile IFS= read -r line; do printf '%s\\n' OK; done\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut sink = NativeSink::new(Some(path));
            assert_eq!(sink.shortcut_policy(), None);
            assert_eq!(retry_busy(|| sink.start_control("peer")).is_ok(), valid);
            assert_eq!(sink.shortcut_policy(), expected);
            sink.finish();
            assert_eq!(sink.shortcut_policy(), None);
        }
    }
    #[test]
    fn native_text_requires_actual_ready_contract() {
        use std::os::unix::fs::PermissionsExt;
        for supported in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("helper");
            std::fs::write(&path,format!("#!/bin/sh\nprintf '%s\\n' 'READY 1920 1080{}'\nwhile IFS= read -r line; do printf '%s\\n' OK; done\n",if supported {" input-text-v1"}else{""})).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut sink = NativeSink::new(Some(path));
            assert!(!sink.supports_text_input());
            retry_busy(|| sink.start_input("peer")).unwrap();
            assert_eq!(sink.supports_text_input(), supported);
            let result = sink.input(&crate::input::InputEvent::Text {
                value: "é🙂".into(),
            });
            assert_eq!(result.is_ok(), supported);
            sink.finish();
            assert!(!sink.supports_text_input());
        }
    }
}
