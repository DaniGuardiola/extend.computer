//! Ordered input forwarding. Coalesce adjacent motion only; never drop a
//! button/key transition. Overflow closes the session and releases input.
use crate::{cursor::spawn, input::InputEvent, session::Client};
use anyhow::{bail, ensure, Result};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Debug)]
enum Captured {
    Move(f64, f64),
    Input(InputEvent),
}
#[derive(Default)]
struct Queue {
    events: VecDeque<Captured>,
    ended: bool,
    user_stopped: bool,
    failed: bool,
    stop_reason: Option<String>,
}
impl Queue {
    fn stopped(&mut self, line: &str) -> bool {
        if line != "STOP" && !line.starts_with("STOP ") {
            return false;
        }
        self.user_stopped = line == "STOP user";
        self.stop_reason = Some(line.to_owned());
        self.ended = true;
        true
    }
    fn push(&mut self, event: Captured) {
        if matches!(event, Captured::Move(..))
            && matches!(self.events.back(), Some(Captured::Move(..)))
        {
            self.events.pop_back();
        }
        if self.events.len() >= 256 {
            self.failed = true;
            self.ended = true;
            return;
        }
        self.events.push_back(event);
    }
}
pub fn send(client: Client, path: &Path, low_jitter: bool, edge: &str, offset: f64) -> Result<()> {
    run(
        client,
        path,
        low_jitter,
        edge,
        offset,
        Some(Duration::from_secs(29)),
        None,
        || {},
        || None,
    )
}
/// New capture helpers start locally, including after reconnect.
pub fn send_session(
    client: Client,
    path: &Path,
    low_jitter: bool,
    edge: &str,
    offset: f64,
    duration: Option<Duration>,
    remembered_only: bool,
) -> Result<()> {
    run(
        client,
        path,
        low_jitter,
        edge,
        offset,
        duration,
        Some(remembered_only),
        || {},
        || None,
    )
}
/// GUI entry point: report readiness only after consent and native capture startup.
#[allow(clippy::too_many_arguments)]
pub fn send_session_with_ready(
    client: Client,
    path: &Path,
    low_jitter: bool,
    edge: &str,
    offset: f64,
    duration: Option<Duration>,
    remembered_only: bool,
    ready: impl FnOnce(),
    desired_edge: impl Fn() -> Option<String>,
) -> Result<()> {
    run(
        client,
        path,
        low_jitter,
        edge,
        offset,
        duration,
        Some(remembered_only),
        ready,
        desired_edge,
    )
}
#[allow(clippy::too_many_arguments)]
fn run(
    mut client: Client,
    path: &Path,
    low_jitter: bool,
    edge: &str,
    offset: f64,
    duration: Option<Duration>,
    session: Option<bool>,
    ready: impl FnOnce(),
    desired_edge: impl Fn() -> Option<String>,
) -> Result<()> {
    ensure!(
        matches!(edge, "left" | "right") && offset.is_finite(),
        "invalid layout"
    );
    let display = if let Some(remembered_only) = session {
        client.request_control(remembered_only)?
    } else {
        client.request_input()?
    };
    let mut lease = if session.is_some() && low_jitter {
        crate::low_jitter::start_required(client.address().ip())?
    } else {
        crate::low_jitter::start_for_peer(client.address().ip(), low_jitter)
    };
    let mut helper = spawn(
        path,
        &format!(
            "capture-{}-{edge}",
            if session.is_some() {
                "control"
            } else {
                "input"
            }
        ),
        &[
            display.width.to_string(),
            display.height.to_string(),
            offset.to_string(),
        ],
    )?;
    let output = helper.0.stdout.take().unwrap();
    let queue = Arc::new(Mutex::new(Queue::default()));
    let writer = queue.clone();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let Ok(line) = line else {
                writer.lock().unwrap().failed = true;
                break;
            };
            if writer.lock().unwrap().stopped(&line) {
                break;
            }
            if line == "REMOTE" {
                eprintln!("Input control moved to peer.");
                continue;
            }
            if line == "LOCAL" {
                eprintln!("Input control returned locally.");
                continue;
            }
            let event = (|| -> Result<Captured> {
                if line.starts_with('{') {
                    let input: InputEvent = serde_json::from_str(&line)?;
                    input.validate()?;
                    Ok(Captured::Input(input))
                } else {
                    let fields: Vec<_> = line.split_whitespace().collect();
                    ensure!(fields.len() == 2, "invalid capture");
                    let (x, y): (f64, f64) = (fields[0].parse()?, fields[1].parse()?);
                    ensure!(
                        x.is_finite()
                            && y.is_finite()
                            && (0.0..=1.0).contains(&x)
                            && (0.0..=1.0).contains(&y),
                        "invalid capture point"
                    );
                    Ok(Captured::Move(x, y))
                }
            })();
            let mut q = writer.lock().unwrap();
            match event {
                Ok(event) => q.push(event),
                Err(_) => {
                    q.failed = true;
                    q.ended = true;
                }
            }
            if q.ended {
                break;
            }
        }
        writer.lock().unwrap().ended = true;
    });
    eprintln!("Full input active {}. Cross {edge}; Control-Option-Escape stops. Release held keys/buttons before crossing.", if session.is_some() { "until stopped or disconnected" } else { "for 30 seconds" });
    ready();
    let trace = crate::timing::Trace::from_env();
    let start = Instant::now();
    let mut ack = Instant::now();
    let mut current_edge = edge.to_owned();
    let mut pending = 0;
    let mut count = 0;
    let mut reached_limit = false;
    let result = (|| -> Result<()> {
        while duration.is_none_or(|limit| start.elapsed() < limit) {
            if pending == 4
                || (pending > 0 && ack.elapsed() >= Duration::from_millis(16))
                || ack.elapsed() >= Duration::from_millis(500)
            {
                if session.is_some() && low_jitter {
                    crate::low_jitter::maintain_required(&mut lease)?;
                } else {
                    crate::low_jitter::maintain(&mut lease);
                }
                trace.record("ack_start", count, 0);
                client.probe()?;
                trace.record("ack_end", count, 0);
                writeln!(helper.0.stdin.as_mut().unwrap(), "ALIVE")?;
                if let Some(edge) = desired_edge() {
                    ensure!(matches!(edge.as_str(), "left" | "right"), "invalid layout");
                    if edge != current_edge {
                        writeln!(helper.0.stdin.as_mut().unwrap(), "EDGE {edge}")?;
                        current_edge = edge;
                    }
                }
                pending = 0;
                ack = Instant::now();
            }
            let next = {
                let mut q = queue.lock().unwrap();
                if q.failed {
                    bail!("input capture failed or queue overflowed");
                }
                if q.ended {
                    if session.is_some() && duration.is_none() && !q.user_stopped {
                        let detail = match q.stop_reason.as_deref() {
                            Some("STOP display-changed") => "The display layout changed. Reconnect to use the new layout.",
                            Some("STOP capture-disabled") => "macOS stopped input capture. Check Accessibility and Input Monitoring, then reconnect.",
                            Some("STOP heartbeat-timeout") => "The input connection stopped responding. Reconnect and try again.",
                            _ => "The input helper stopped unexpectedly. Reconnect and try again.",
                        };
                        bail!(detail);
                    }
                    break;
                }
                q.events.pop_front()
            };
            if let Some(next) = next {
                trace.record("send_start", count, 0);
                match next {
                    Captured::Move(x, y) => client.queue_cursor(x, y)?,
                    Captured::Input(event) => client.queue_input(event)?,
                }
                trace.record("send_end", count, 0);
                pending += 1;
                count += 1;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        reached_limit = duration.is_some_and(|limit| start.elapsed() >= limit);
        // Drain the bounded wire window, then release before closing the peer.
        client.probe()?;
        client.queue_input(InputEvent::Release)?;
        client.probe()?;
        Ok(())
    })();
    drop(helper);
    let _ = reader.join();
    let intentional_stop = queue.lock().unwrap().user_stopped || reached_limit;
    if intentional_stop {
        // A simultaneous network error must not turn emergency escape or a
        // requested duration limit into an automatic reconnect.
        let _ = client.close();
        println!("input_events={count}");
        return Ok(());
    }
    result?;
    println!("input_events={count}");
    client.close()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_stop_reasons_preserve_emergency_escape() {
        for reason in [
            "STOP",
            "STOP display-changed",
            "STOP capture-disabled",
            "STOP heartbeat-timeout",
        ] {
            let mut queue = Queue::default();
            assert!(queue.stopped(reason));
            assert!(queue.ended && !queue.user_stopped);
            assert_eq!(queue.stop_reason.as_deref(), Some(reason));
        }
        let mut queue = Queue::default();
        assert!(!queue.stopped("REMOTE"));
        assert!(!queue.ended);
        assert!(queue.stopped("STOP user"));
        assert!(queue.ended && queue.user_stopped);
    }
    #[test]
    fn motion_coalesces_only_between_transitions() {
        let mut q = Queue::default();
        q.push(Captured::Move(0.1, 0.2));
        q.push(Captured::Move(0.2, 0.2));
        q.push(Captured::Input(InputEvent::Button {
            button: 0,
            down: true,
            clicks: 1,
        }));
        q.push(Captured::Move(0.3, 0.2));
        q.push(Captured::Move(0.4, 0.2));
        q.push(Captured::Input(InputEvent::Button {
            button: 0,
            down: false,
            clicks: 1,
        }));
        assert_eq!(q.events.len(), 4);
        assert!(!q.failed);
    }
    #[test]
    fn transition_overflow_ends_session_instead_of_dropping_keys() {
        let mut q = Queue::default();
        for _ in 0..257 {
            q.push(Captured::Input(InputEvent::Release));
        }
        assert!(q.failed && q.ended);
        assert_eq!(q.events.len(), 256);
    }
}
