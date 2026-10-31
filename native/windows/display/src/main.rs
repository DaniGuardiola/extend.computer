//! Windows display helper: mirror source and viewer. Protocol:
//! docs/architecture/native-helpers.md.
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod encoder;
#[allow(dead_code)]
#[path = "../../shared/keys.rs"]
mod keys;
#[cfg(windows)]
mod self_test;
mod viewer;

#[cfg(not(windows))]
fn main() {
    eprintln!("The Windows display helper only runs on Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_main::run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_main {
    use crate::{
        capture::Capture,
        encoder::{Encoder, Input},
    };
    use anyhow::{bail, ensure, Result};
    use extend_computer_agent::{
        helper::{
            display::{
                Abilities, Configuration, HelperMessage, Launch, Screen, SourceCommand, Startup,
            },
            packet::{frame, write_packet, Kind},
        },
        session::display::{VideoCodec, CONFIG_LIMITS},
    };
    use std::{
        io::{BufRead, Write},
        sync::{
            atomic::{AtomicBool, AtomicU32, Ordering},
            mpsc, Arc, Mutex,
        },
        time::{Duration, Instant},
    };

    /// Frames with no screen change are repeated at this interval so a
    /// receiver that joins or drops a frame recovers without user activity.
    const IDLE_REFRESH: Duration = Duration::from_millis(500);

    struct Output(Mutex<std::io::Stdout>);
    impl Output {
        fn packet(&self, kind: Kind, payload: &[u8]) -> Result<()> {
            let mut out = self.0.lock().unwrap();
            write_packet(&mut *out, kind, payload)?;
            out.flush()?;
            Ok(())
        }
        fn control(&self, message: &HelperMessage) -> Result<()> {
            self.packet(Kind::Control, &message.to_bytes()?)
        }
    }

    pub fn run() -> Result<()> {
        crate::capture::dpi_aware();
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("screen") => {
                let (width, height) = crate::capture::primary_size()?;
                // Report physical pixels at scale 1 so mirroring encodes at
                // native resolution instead of the scaled desktop size.
                let screen = Screen {
                    width,
                    height,
                    scale: 1.0,
                    usable_width: None,
                    usable_height: None,
                    viewport_cg: None,
                    window_number: None,
                };
                println!("{}", String::from_utf8(screen.to_bytes()?)?);
                Ok(())
            }
            Some("status") => {
                println!("capture=true virtual=false");
                Ok(())
            }
            // What this build implements; the app's Windows defaults match it.
            Some("capabilities") => {
                let abilities = Abilities {
                    version: Abilities::VERSION,
                    source: vec!["mirror".into()],
                    target: false,
                    viewer: true,
                    viewer_input: true,
                    encode: vec!["h264".into()],
                    decode: vec!["h264".into()],
                    cursor_send: false,
                    cursor_overlay: false,
                };
                println!("{}", String::from_utf8(abilities.to_bytes()?)?);
                Ok(())
            }
            Some("self-test") => crate::self_test::run(&args[1..]),
            Some("mirror") => mirror(Launch::parse(&args)?),
            Some("extend") => bail!("Extended displays are not supported on Windows yet."),
            Some("viewer") => crate::viewer::run(Launch::parse(&args)?),
            _ => bail!("Choose extend, mirror or viewer."),
        }
    }

    fn mirror(launch: Launch) -> Result<()> {
        let Launch {
            width,
            height,
            fps,
            bitrate,
            retina,
            ..
        } = launch;
        ensure!(
            launch.codec == VideoCodec::H264,
            "Only H.264 is supported on Windows."
        );
        let scale = if retina { 2 } else { 1 };
        let (width, height) = (even(width * scale), even(height * scale));
        ensure!(
            width <= CONFIG_LIMITS.encoded && height <= CONFIG_LIMITS.encoded,
            "Video capture size exceeds supported limit."
        );

        let output = Arc::new(Output(Mutex::new(std::io::stdout())));
        let (controls, control) = mpsc::channel();
        spawn_stdin(controls);

        let mut capture = Capture::new(width, height)?;
        let mut encoder = Encoder::new(capture.device(), width, height, fps, bitrate)?;
        eprintln!("Display encoder: {}", encoder.name);
        let startup = Startup {
            ready: true,
            width: Some(width / scale),
            height: Some(height / scale),
            retina: Some(retina),
            target: None,
        };
        output.packet(Kind::Startup, &startup.to_bytes()?)?;

        let paused = AtomicBool::new(false);
        let frame_interval = AtomicU32::new(1_000_000 / fps);
        let mut force_keyframe = true;
        let mut last_encode = Instant::now() - IDLE_REFRESH;
        let mut dirty = false;
        loop {
            loop {
                match control.try_recv() {
                    Ok(SourceCommand::Keyframe) => force_keyframe = true,
                    Ok(SourceCommand::Pause { value }) => {
                        let resumed = paused.swap(value, Ordering::SeqCst) && !value;
                        force_keyframe |= resumed;
                    }
                    Ok(SourceCommand::Bitrate { value }) => encoder.set_bitrate(value)?,
                    // Retina stays as launched.
                    Ok(SourceCommand::Configuration(Configuration {
                        id,
                        width: w,
                        height: h,
                        fps: f,
                        bitrate: b,
                        ..
                    })) => {
                        let result = (|| -> Result<()> {
                            let (w, h) = (even(w * scale), even(h * scale));
                            ensure!(
                                w <= CONFIG_LIMITS.encoded && h <= CONFIG_LIMITS.encoded,
                                "Video capture size exceeds supported limit."
                            );
                            let next = Capture::new(w, h)?;
                            encoder = Encoder::new(next.device(), w, h, f, b)?;
                            capture = next;
                            frame_interval.store(1_000_000 / f, Ordering::SeqCst);
                            Ok(())
                        })();
                        if let Err(error) = &result {
                            eprintln!("Display settings: {error:#}");
                        }
                        force_keyframe = true;
                        output.control(&HelperMessage::Configured {
                            id,
                            ok: result.is_ok(),
                        })?;
                    }
                    Ok(SourceCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                        return Ok(())
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            let interval = Duration::from_micros(frame_interval.load(Ordering::SeqCst) as u64);
            // A change seen before the frame interval is up waits for the next slot,
            // so a fast-refreshing screen is still encoded at the configured rate.
            let wait = if dirty {
                interval.saturating_sub(last_encode.elapsed())
            } else {
                interval
            };
            dirty |= capture.next_frame(wait)?;
            if paused.load(Ordering::SeqCst) {
                continue;
            }
            let due = last_encode.elapsed() >= interval;
            if !force_keyframe && !(dirty && due) && last_encode.elapsed() < IDLE_REFRESH {
                continue;
            }
            let started = Instant::now();
            let Some(slot) = capture.convert()? else {
                continue;
            };
            let timestamp = crate::capture::monotonic_ns();
            let input = if encoder.wants_texture() {
                Input::Texture(capture.texture(slot))
            } else {
                Input::Memory(capture.read(slot)?)
            };
            for frame in encoder.encode(input, timestamp, std::mem::take(&mut force_keyframe))? {
                output.packet(Kind::Frame, &frame::encode(&frame))?;
            }
            // Pace from the start of the encode so its cost doesn't lower the rate.
            last_encode = started;
            dirty = false;
        }
    }

    fn even(value: u32) -> u32 {
        value & !1
    }

    fn spawn_stdin(controls: mpsc::Sender<SourceCommand>) {
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let Some(command) = SourceCommand::parse_line(&line) else {
                    continue;
                };
                let stop = command == SourceCommand::Stop;
                if controls.send(command).is_err() || stop {
                    return;
                }
            }
            // Stdin closes when the app exits or drops the helper.
            let _ = controls.send(SourceCommand::Stop);
        });
    }
}
