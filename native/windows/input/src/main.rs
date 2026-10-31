//! Windows input helper. Protocol: docs/architecture/native-helpers.md.
#[cfg(windows)]
mod capture;
#[cfg_attr(not(windows), allow(dead_code))]
mod edge;
#[cfg(windows)]
mod inject;
#[cfg_attr(not(windows), allow(dead_code))]
#[path = "../../shared/keys.rs"]
mod keys;

#[cfg(not(windows))]
fn main() {
    eprintln!("The Windows input helper only runs on Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    let code = match windows_main::run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(code);
}

#[cfg(windows)]
mod windows_main {
    use crate::{
        capture,
        edge::Side,
        inject::{primary, Injector},
    };
    use anyhow::{bail, ensure, Context, Result};
    use extend_computer_agent::{
        helper::input::{ControlReady, Status, Token},
        input::InputEvent,
    };
    use std::{
        io::{BufRead, Write},
        sync::mpsc,
        time::{Duration, Instant},
    };
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    const HEARTBEAT: Duration = Duration::from_secs(2);

    pub fn run() -> Result<()> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            // Windows needs no grant to read or post input in the user's session.
            Some("status") => {
                let status = Status {
                    listen: true,
                    post: true,
                };
                println!("{status}");
                Ok(())
            }
            Some("inject-control") => receive(),
            Some(mode) if mode.starts_with("capture-control-") => {
                let side = Side::parse(&mode["capture-control-".len()..])
                    .context("Unsupported input helper mode")?;
                capture::run(side, &args[1..])
            }
            Some(mode) => bail!("Unsupported input helper mode: {mode}"),
            None => bail!("Choose an input helper mode."),
        }
    }

    fn lines() -> mpsc::Receiver<String> {
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        receive
    }

    fn reply(text: &str) -> Result<()> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{text}")?;
        out.flush()?;
        Ok(())
    }

    /// Injects input from the peer into this computer.
    fn receive() -> Result<()> {
        let dry_run = std::env::var("EXTEND_COMPUTER_INPUT_DRY_RUN").is_ok_and(|v| v == "1");
        let mut injector = Injector::new(dry_run);
        let (display, width, height) = primary();
        let input = lines();
        let ready = ControlReady {
            width,
            height,
            tokens: vec![Token::InputText, Token::ShortcutsControl],
        };
        reply(&ready.to_string())?;
        let mut last = Instant::now();
        let result = loop {
            match input.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    if let Err(error) = apply(&mut injector, line.trim()) {
                        break Err(error);
                    }
                    last = Instant::now();
                    reply("OK")?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if last.elapsed() > HEARTBEAT {
                break Err(anyhow::anyhow!("Input heartbeat stopped"));
            }
            let (now, _, _) = primary();
            if now != display {
                break Err(anyhow::anyhow!("The display changed"));
            }
        };
        let released = injector.release();
        if dry_run {
            reply(&format!("RELEASED {}", injector.summary()))?;
        }
        reply("STOP")?;
        if let Err(error) = result {
            eprintln!("Input stopped: {error:#}");
        }
        released
    }

    fn apply(injector: &mut Injector, line: &str) -> Result<()> {
        if line == "ALIVE" {
            return Ok(());
        }
        if line.starts_with('{') {
            let event: InputEvent = serde_json::from_str(line)?;
            return injector.event(&event);
        }
        let mut parts = line.split_whitespace();
        let (Some(x), Some(y), None) = (parts.next(), parts.next(), parts.next()) else {
            bail!("Invalid input line");
        };
        let (x, y): (f64, f64) = (
            x.parse().context("Invalid pointer position")?,
            y.parse().context("Invalid pointer position")?,
        );
        ensure!(x.is_finite() && y.is_finite(), "Invalid pointer position");
        injector.move_to(x, y)
    }
}
