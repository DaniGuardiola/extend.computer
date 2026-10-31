//! Windows display helper (mirror source). Protocol:
//! docs/architecture/native-helpers.md.
// Spawned by the GUI app over pipes; a console subsystem would flash a window.
#![windows_subsystem = "windows"]
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod encoder;

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
    use anyhow::{bail, ensure, Context, Result};
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
        fn packet(&self, kind: u8, payload: &[u8]) -> Result<()> {
            let mut out = self.0.lock().unwrap();
            out.write_all(&[kind])?;
            out.write_all(&(payload.len() as u32).to_be_bytes())?;
            out.write_all(payload)?;
            out.flush()?;
            Ok(())
        }
        fn control(&self, value: serde_json::Value) -> Result<()> {
            self.packet(2, &serde_json::to_vec(&value)?)
        }
    }

    enum Control {
        Keyframe,
        Pause(bool),
        Bitrate(u32),
        Configure {
            id: u64,
            width: u32,
            height: u32,
            fps: u32,
            bitrate: u32,
        },
        Stop,
    }

    fn number(args: &[String], index: usize, range: std::ops::RangeInclusive<u32>) -> Result<u32> {
        let value: u32 = args
            .get(index)
            .and_then(|v| v.parse().ok())
            .context("Invalid display configuration.")?;
        ensure!(range.contains(&value), "Invalid display configuration.");
        Ok(value)
    }

    pub fn run() -> Result<()> {
        crate::capture::dpi_aware();
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("screen") => {
                let (width, height) = crate::capture::primary_size()?;
                // Report physical pixels at scale 1 so mirroring encodes at
                // native resolution instead of the scaled desktop size.
                println!(
                    "{}",
                    serde_json::json!({"width": width, "height": height, "scale": 1.0})
                );
                Ok(())
            }
            Some("status") => {
                println!("capture=true virtual=false");
                Ok(())
            }
            Some("mirror") => mirror(&args),
            Some("extend") => bail!("Extended displays are not supported on Windows yet."),
            Some("viewer") => bail!("Viewing another screen is not supported on Windows yet."),
            _ => bail!("Choose extend, mirror or viewer."),
        }
    }

    fn mirror(args: &[String]) -> Result<()> {
        let width = number(args, 1, 640..=4096)?;
        let height = number(args, 2, 480..=4096)?;
        let fps = number(args, 3, 30..=120)?;
        let bitrate = number(args, 4, 1_000_000..=80_000_000)?;
        let retina = args.get(5).is_some_and(|v| v == "retina");
        ensure!(
            args.get(7).is_none_or(|codec| codec == "h264"),
            "Only H.264 is supported on Windows."
        );
        let scale = if retina { 2 } else { 1 };
        let (width, height) = (even(width * scale), even(height * scale));
        ensure!(
            width <= 4096 && height <= 4096,
            "Video capture size exceeds supported limit."
        );

        let output = Arc::new(Output(Mutex::new(std::io::stdout())));
        let (controls, control) = mpsc::channel();
        spawn_stdin(controls);

        let mut capture = Capture::new(width, height)?;
        let mut encoder = Encoder::new(capture.device(), width, height, fps, bitrate)?;
        eprintln!("Display encoder: {}", encoder.name);
        output.packet(
            0,
            &serde_json::to_vec(&serde_json::json!({
                "ready": true, "width": width / scale, "height": height / scale, "retina": retina
            }))?,
        )?;

        let paused = AtomicBool::new(false);
        let frame_interval = AtomicU32::new(1_000_000 / fps);
        let mut force_keyframe = true;
        let mut last_encode = Instant::now() - IDLE_REFRESH;
        loop {
            loop {
                match control.try_recv() {
                    Ok(Control::Keyframe) => force_keyframe = true,
                    Ok(Control::Pause(value)) => {
                        let resumed = paused.swap(value, Ordering::SeqCst) && !value;
                        force_keyframe |= resumed;
                    }
                    Ok(Control::Bitrate(value)) => encoder.set_bitrate(value)?,
                    Ok(Control::Configure {
                        id,
                        width: w,
                        height: h,
                        fps: f,
                        bitrate: b,
                    }) => {
                        let result = (|| -> Result<()> {
                            let (w, h) = (even(w * scale), even(h * scale));
                            ensure!(
                                w <= 4096 && h <= 4096,
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
                        output.control(
                            serde_json::json!({"type": "configured", "id": id, "ok": result.is_ok()}),
                        )?;
                    }
                    Ok(Control::Stop) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            let interval = Duration::from_micros(frame_interval.load(Ordering::SeqCst) as u64);
            let changed = capture.next_frame(interval)?;
            if paused.load(Ordering::SeqCst) {
                continue;
            }
            if !changed && !force_keyframe && last_encode.elapsed() < IDLE_REFRESH {
                continue;
            }
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
                let mut payload = Vec::with_capacity(frame.bytes.len() + 9);
                payload.extend_from_slice(&frame.timestamp.to_be_bytes());
                payload.push(u8::from(frame.keyframe));
                payload.extend_from_slice(&frame.bytes);
                output.packet(1, &payload)?;
            }
            last_encode = Instant::now();
        }
    }

    fn even(value: u32) -> u32 {
        value & !1
    }

    fn spawn_stdin(controls: mpsc::Sender<Control>) {
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                let u32_field =
                    |name: &str| value[name].as_u64().and_then(|v| u32::try_from(v).ok());
                let control = match value["type"].as_str() {
                    Some("keyframe") => Control::Keyframe,
                    Some("pause") => match value["value"].as_bool() {
                        Some(paused) => Control::Pause(paused),
                        None => continue,
                    },
                    Some("bitrate") => match u32_field("value") {
                        Some(bitrate) => Control::Bitrate(bitrate),
                        None => continue,
                    },
                    Some("configuration") => {
                        let (Some(id), Some(width), Some(height), Some(fps), Some(bitrate)) = (
                            value["id"].as_u64(),
                            u32_field("width"),
                            u32_field("height"),
                            u32_field("fps"),
                            u32_field("bitrate"),
                        ) else {
                            continue;
                        };
                        Control::Configure {
                            id,
                            width,
                            height,
                            fps,
                            bitrate,
                        }
                    }
                    Some("stop") => Control::Stop,
                    _ => continue,
                };
                let stop = matches!(control, Control::Stop);
                if controls.send(control).is_err() || stop {
                    return;
                }
            }
            // Stdin closes when the app exits or drops the helper.
            let _ = controls.send(Control::Stop);
        });
    }
}
