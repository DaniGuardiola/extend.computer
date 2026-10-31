//! Bounded, local-only native capture -> WebRTC -> native GPU presentation check.
use anyhow::{ensure, Result};
use extend_computer_agent::{
    display::{DisplayEvent, Transport},
    session::display::{Config, Mode},
};
use std::{
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
#[path = "../../desktop/src-tauri/src/runtime/display/helper.rs"]
#[allow(dead_code, unexpected_cfgs)]
// Shared app helper also exposes screen negotiation, unused by this fixed-size smoke test.
mod helper;
#[path = "../../desktop/src-tauri/src/runtime/display/receiver.rs"]
#[allow(dead_code)]
mod receiver;
#[allow(dead_code)]
mod input {
    // The smoke test does not forward input, so it accepts any reported capture target.
    #[derive(Clone, Debug, serde::Deserialize)]
    pub struct CaptureTarget(serde_json::Value);
    impl CaptureTarget {
        pub fn validate(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }
}

fn main() -> Result<()> {
    let mode = match std::env::args().nth(1).as_deref() {
        Some("mirror") => Mode::Mirror,
        Some("extend") => Mode::Extend,
        _ => anyhow::bail!("Choose mirror or extend"),
    };
    let config = Config {
        mode,
        width: 1280,
        height: 720,
        fps: 60,
        bitrate: 12_000_000,
        retina: std::env::args().any(|arg| arg == "retina"),
        auto_size: false,
        codec: extend_computer_agent::session::display::VideoCodec::Hevc,
        separate_cursor: true,
    };
    config.validate()?;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/extend.computer Display.app/Contents/MacOS/ExtendComputerDisplay");
    let ip = "127.0.0.1".parse()?;
    let mut sender = Transport::with_codec(ip, config.codec)?;
    sender.set_target_bitrate(config.bitrate)?;
    let mut receiver = Transport::with_codec(ip, config.codec)?;
    let offer = sender.offer()?;
    let answer = receiver.answer(&offer)?;
    sender.accept_answer(&answer)?;
    let until = Instant::now() + Duration::from_secs(15);
    let mut connected = false;
    let mut capture = None;
    let mut viewer = None;
    let mut encoded = 0;
    let result = (|| -> Result<()> {
        loop {
            if let Some(event) = sender.poll()? {
                match event {
                    DisplayEvent::Connected => connected = true,
                    DisplayEvent::Keyframe => {
                        if let Some(capture) = &capture {
                            helper::Helper::request_keyframe(capture)?;
                        }
                    }
                    DisplayEvent::Bitrate(estimate) => {
                        if let Some(capture) = &capture {
                            capture.set_bitrate(
                                (estimate * 85 / 100).clamp(1_000_000, config.bitrate as u64)
                                    as u32,
                            )?;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(event) = receiver.poll()? {
                match event {
                    DisplayEvent::Connected => {
                        viewer = Some(helper::Helper::start(&path, &config, false, false, &[])?)
                    }
                    DisplayEvent::Frame(frame) => {
                        if let Some(viewer) = &viewer {
                            ensure!(viewer.present(frame), "Viewer queue overflow");
                        }
                    }
                    DisplayEvent::Cursor(bytes) => {
                        if let Some(viewer) = &viewer {
                            viewer.cursor(bytes);
                        }
                    }
                    DisplayEvent::Keyframe => receiver.request_keyframe()?,
                    _ => {}
                }
            }
            if connected && capture.is_none() {
                capture = Some(helper::Helper::start(&path, &config, true, false, &[])?);
            }
            if let Some(capture) = &capture {
                ensure!(
                    !capture.closed.load(Ordering::SeqCst),
                    "{}",
                    capture.failure()
                );
                if capture.keyframe.swap(false, Ordering::SeqCst) {
                    capture.request_keyframe()?;
                }
                for slot in &capture.cursors {
                    let bytes = slot.lock().unwrap().take();
                    if let Some(bytes) = bytes {
                        if !sender.write_cursor(&bytes)? {
                            slot.lock().unwrap().get_or_insert(bytes);
                        }
                    }
                }
                if let Ok(frame) = capture.frames.try_recv() {
                    encoded += 1;
                    sender.write(frame)?;
                }
            }
            if let Some(viewer) = &viewer {
                ensure!(
                    !viewer.closed.load(Ordering::SeqCst),
                    "{}",
                    viewer.failure()
                );
                if viewer.keyframe.swap(false, Ordering::SeqCst) {
                    receiver.request_keyframe()?;
                }
                if viewer.presented.load(Ordering::SeqCst) > 0 {
                    println!("PASS: {:?}, retina={}, {} encoded frames; native GPU presentation completed over encrypted WebRTC", mode, config.retina, encoded);
                    break;
                }
            }
            ensure!(
                Instant::now() < until,
                "Native display round trip timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    })();
    drop(capture);
    drop(viewer);
    result
}
