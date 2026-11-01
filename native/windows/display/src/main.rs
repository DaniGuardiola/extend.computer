//! Windows display helper: mirror and extend source, and viewer. Protocol:
//! docs/architecture/native-helpers.md.
#[cfg(windows)]
mod capture;
#[cfg(windows)]
#[path = "../../shared/display_driver.rs"]
mod display_driver;
#[cfg(windows)]
mod encoder;
mod extend;
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
        extend::{Extend, Stopped},
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
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Mutex,
        },
        time::{Duration, Instant},
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D},
        System::{
            Power::{
                SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
            },
            Threading::{WaitForSingleObject, INFINITE},
        },
    };

    /// Frames with no screen change are repeated at this interval so a
    /// receiver that joins or drops a frame recovers without user activity.
    const IDLE_REFRESH: Duration = Duration::from_millis(500);
    /// How long a source may take to clean up after its parent exits.
    const EXIT_GRACE: Duration = Duration::from_secs(10);

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
                println!("capture=true virtual={}", crate::extend::available());
                Ok(())
            }
            // What this build implements; the app's Windows defaults match it
            // without the display driver, which extending needs.
            Some("capabilities") => {
                let mut source = vec!["mirror".to_string()];
                if crate::extend::available() {
                    source.push("extend".into());
                }
                let abilities = Abilities {
                    version: Abilities::VERSION,
                    source,
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
            Some("extend") => extend(Launch::parse(&args)?),
            Some("viewer") => crate::viewer::run(Launch::parse(&args)?),
            _ => bail!("Choose extend, mirror or viewer."),
        }
    }

    /// What the stream loop needs from a mirror or extend source.
    trait Source {
        /// The device encoders read converted frames on. It can change, when
        /// Windows moves the extended display to another GPU.
        fn device(&self) -> &ID3D11Device;
        /// Waits up to `timeout` for a change; the latest image stays available.
        fn next_frame(&mut self, timeout: Duration) -> Result<bool>;
        /// Converts the latest image to NV12; returns its ring slot.
        fn convert(&mut self) -> Result<Option<usize>>;
        fn texture(&self, slot: usize) -> &ID3D11Texture2D;
        fn read(&mut self, slot: usize) -> Result<&[u8]>;
        /// Applies new settings; returns the new encoded size.
        fn configure(&mut self, configuration: &Configuration) -> Result<(u32, u32)>;
    }

    /// The primary display, with Retina as launched.
    struct Mirror {
        capture: Capture,
        scale: u32,
    }

    impl Source for Mirror {
        fn device(&self) -> &ID3D11Device {
            self.capture.device()
        }
        fn next_frame(&mut self, timeout: Duration) -> Result<bool> {
            self.capture.next_frame(timeout)
        }
        fn convert(&mut self) -> Result<Option<usize>> {
            self.capture.convert()
        }
        fn texture(&self, slot: usize) -> &ID3D11Texture2D {
            self.capture.texture(slot)
        }
        fn read(&mut self, slot: usize) -> Result<&[u8]> {
            self.capture.read(slot)
        }
        fn configure(&mut self, configuration: &Configuration) -> Result<(u32, u32)> {
            let (width, height) = (
                even(configuration.width * self.scale),
                even(configuration.height * self.scale),
            );
            ensure!(
                width <= CONFIG_LIMITS.encoded && height <= CONFIG_LIMITS.encoded,
                "Video capture size exceeds supported limit."
            );
            self.capture = Capture::new(width, height)?;
            Ok((width, height))
        }
    }

    impl Source for Extend {
        fn device(&self) -> &ID3D11Device {
            Extend::device(self)
        }
        fn next_frame(&mut self, timeout: Duration) -> Result<bool> {
            Extend::next_frame(self, timeout)
        }
        fn convert(&mut self) -> Result<Option<usize>> {
            Extend::convert(self)
        }
        fn texture(&self, slot: usize) -> &ID3D11Texture2D {
            Extend::texture(self, slot)
        }
        fn read(&mut self, slot: usize) -> Result<&[u8]> {
            Extend::read(self, slot)
        }
        fn configure(&mut self, configuration: &Configuration) -> Result<(u32, u32)> {
            Extend::configure(self, configuration)
        }
    }

    fn mirror(launch: Launch) -> Result<()> {
        ensure!(
            launch.codec == VideoCodec::H264,
            "Only H.264 is supported on Windows."
        );
        let scale = if launch.retina { 2 } else { 1 };
        let (width, height) = (even(launch.width * scale), even(launch.height * scale));
        ensure!(
            width <= CONFIG_LIMITS.encoded && height <= CONFIG_LIMITS.encoded,
            "Video capture size exceeds supported limit."
        );
        let (controls, _) = spawn_stdin();
        let mut source = Mirror {
            capture: Capture::new(width, height)?,
            scale,
        };
        let startup = Startup {
            ready: true,
            width: Some(width / scale),
            height: Some(height / scale),
            retina: Some(launch.retina),
            target: None,
        };
        stream(&mut source, (width, height), &launch, startup, controls)
    }

    fn extend(launch: Launch) -> Result<()> {
        ensure!(
            launch.codec == VideoCodec::H264,
            "Only H.264 is supported on Windows."
        );
        // Commands queue while the monitor is set up; stop cuts that short.
        let (controls, stopping) = spawn_stdin();
        // Windows draws on no monitor while the display sleeps, so it stays
        // awake while this PC extends, as it does while the viewer shows.
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
        }
        let mut source = match Extend::start(&launch, stopping) {
            Ok(source) => source,
            Err(error) if error.is::<Stopped>() => return Ok(()),
            Err(error) => return Err(error),
        };
        let ((width, height), retina) = source.size();
        let scale = if retina { 2 } else { 1 };
        let startup = Startup {
            ready: true,
            width: Some(width / scale),
            height: Some(height / scale),
            retina: Some(retina),
            target: None,
        };
        // Dropping the source removes the monitor and restores the layout.
        stream(&mut source, (width, height), &launch, startup, controls)
    }

    /// Encodes `source` and answers the app's commands until stop, the end
    /// of stdin or the parent's exit.
    fn stream(
        source: &mut dyn Source,
        size: (u32, u32),
        launch: &Launch,
        startup: Startup,
        control: mpsc::Receiver<SourceCommand>,
    ) -> Result<()> {
        let output = Output(Mutex::new(std::io::stdout()));
        let (mut size, mut fps, mut bitrate) = (size, launch.fps, launch.bitrate);
        let mut encoder = Encoder::new(source.device(), size.0, size.1, fps, bitrate)?;
        let mut encoder_device = source.device().clone();
        eprintln!("Display encoder: {}", encoder.name);
        output.packet(Kind::Startup, &startup.to_bytes()?)?;

        let mut paused = false;
        let mut force_keyframe = true;
        let mut last_encode = Instant::now() - IDLE_REFRESH;
        let mut dirty = false;
        loop {
            loop {
                match control.try_recv() {
                    Ok(SourceCommand::Keyframe) => force_keyframe = true,
                    Ok(SourceCommand::Pause { value }) => {
                        force_keyframe |= paused && !value;
                        paused = value;
                    }
                    Ok(SourceCommand::Bitrate { value }) => {
                        encoder.set_bitrate(value)?;
                        bitrate = value;
                    }
                    Ok(SourceCommand::Configuration(configuration)) => {
                        let result = (|| -> Result<()> {
                            let next = source.configure(&configuration)?;
                            encoder = Encoder::new(
                                source.device(),
                                next.0,
                                next.1,
                                configuration.fps,
                                configuration.bitrate,
                            )?;
                            encoder_device = source.device().clone();
                            (size, fps, bitrate) = (next, configuration.fps, configuration.bitrate);
                            Ok(())
                        })();
                        if let Err(error) = &result {
                            eprintln!("Display settings: {error:#}");
                        }
                        force_keyframe = true;
                        output.control(&HelperMessage::Configured {
                            id: configuration.id,
                            ok: result.is_ok(),
                        })?;
                    }
                    Ok(SourceCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                        return Ok(())
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            let interval = Duration::from_micros(1_000_000 / u64::from(fps));
            // A change seen before the frame interval is up waits for the next slot,
            // so a fast-refreshing screen is still encoded at the configured rate.
            let wait = if dirty {
                interval.saturating_sub(last_encode.elapsed())
            } else {
                interval
            };
            dirty |= source.next_frame(wait)?;
            if source.device() != &encoder_device {
                // The extended display moved to another GPU.
                encoder = Encoder::new(source.device(), size.0, size.1, fps, bitrate)?;
                encoder_device = source.device().clone();
                eprintln!("Display encoder: {}", encoder.name);
                force_keyframe = true;
            }
            if paused {
                continue;
            }
            let due = last_encode.elapsed() >= interval;
            if !force_keyframe && !(dirty && due) && last_encode.elapsed() < IDLE_REFRESH {
                continue;
            }
            let started = Instant::now();
            let Some(slot) = source.convert()? else {
                continue;
            };
            let timestamp = crate::capture::monotonic_ns();
            let input = if encoder.wants_texture() {
                Input::Texture(source.texture(slot))
            } else {
                Input::Memory(source.read(slot)?)
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

    /// Reads the app's commands. Stop also comes when stdin closes, as when
    /// the app exits or drops the helper, and when the app's process ends,
    /// in case another process inherited the pipe. The flag is set as stop
    /// comes, ahead of the commands queued before it.
    fn spawn_stdin() -> (mpsc::Receiver<SourceCommand>, Arc<AtomicBool>) {
        let (controls, control) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        if let Some(parent) = crate::viewer::parent_process() {
            let parent = parent.0 as isize;
            let controls = controls.clone();
            let stopping = stopping.clone();
            std::thread::spawn(move || {
                unsafe { WaitForSingleObject(HANDLE(parent as _), INFINITE) };
                stopping.store(true, Ordering::Relaxed);
                let _ = controls.send(SourceCommand::Stop);
                // Don't outlive a stuck cleanup; the driver departs the
                // monitor when this process's handle closes.
                std::thread::sleep(EXIT_GRACE);
                std::process::exit(0);
            });
        }
        let stdin_stopping = stopping.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let Some(command) = SourceCommand::parse_line(&line) else {
                    continue;
                };
                let stop = command == SourceCommand::Stop;
                if stop {
                    stdin_stopping.store(true, Ordering::Relaxed);
                }
                if controls.send(command).is_err() || stop {
                    return;
                }
            }
            stdin_stopping.store(true, Ordering::Relaxed);
            let _ = controls.send(SourceCommand::Stop);
        });
        (control, stopping)
    }
}
