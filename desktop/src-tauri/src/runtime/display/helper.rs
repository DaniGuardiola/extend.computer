use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    display::Frame,
    session::display::{Config, Mode, VideoCodec},
};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const DIAGNOSTIC_TAIL_BYTES: usize = 8192;
const DIAGNOSTIC_DRAIN_GRACE: Duration = Duration::from_millis(100);

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderReady {
    #[serde(rename = "type")]
    kind: String,
    ready: bool,
}

fn update_render_ready(
    message: &serde_json::Value,
    source: bool,
    ready: &AtomicBool,
) -> Result<()> {
    ensure!(!source, "Render readiness is a viewer-only milestone");
    let message: RenderReady = serde_json::from_value(message.clone())?;
    ensure!(
        message.kind == "render_ready" && message.ready,
        "Invalid native render readiness"
    );
    // GPU completion establishes render readiness. It does not substitute for
    // a compositor presentation timestamp or count presented frames.
    ready.store(true, Ordering::SeqCst);
    Ok(())
}

fn rendering_ready(ready: &AtomicBool, presented: &AtomicUsize) -> bool {
    ready.load(Ordering::SeqCst) || presented.load(Ordering::SeqCst) > 0
}

#[derive(Default)]
struct ViewerInput {
    events: VecDeque<super::receiver::NativeEvent>,
    overflow: bool,
}
impl ViewerInput {
    fn push(&mut self, event: super::receiver::NativeEvent) {
        use super::receiver::NativeEvent;
        use extend_computer_agent::display::input::Command;
        if self.overflow {
            return;
        }
        if matches!(event, NativeEvent::Focus { active: false, .. }) {
            self.events.clear();
        } else if matches!(
            event,
            NativeEvent::Input {
                command: Command::Move { .. },
                ..
            }
        ) && self.events.back().is_some_and(|previous| {
            matches!(
                previous,
                NativeEvent::Input {
                    command: Command::Move { .. },
                    ..
                }
            )
        }) {
            self.events.pop_back();
        }
        if self.events.len() >= 64 {
            self.events.clear();
            self.overflow = true;
            return;
        }
        self.events.push_back(event);
    }
}

fn automatic_low_latency(
    explicit: Option<&std::ffi::OsStr>,
    codec: VideoCodec,
    source: bool,
    physical_usb: bool,
) -> bool {
    explicit.is_none() && source && physical_usb && codec == VideoCodec::Hevc
}

#[derive(serde::Deserialize)]
struct Screen {
    width: u32,
    height: u32,
    scale: f64,
    usable_width: Option<u32>,
    usable_height: Option<u32>,
    viewport_cg: Option<ViewportBounds>,
    window_number: Option<u32>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct ViewportBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn log_viewport_evidence(pid: u32, screen: &Screen, fullscreen: bool) -> Result<()> {
    if !fullscreen || std::env::var_os("EXTEND_DISPLAY_METRICS").is_none() {
        return Ok(());
    }
    if let (Some(bounds), Some(number)) = (&screen.viewport_cg, screen.window_number) {
        ensure!(
            [
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                screen.scale
            ]
            .into_iter()
            .all(f64::is_finite)
                && bounds.width > 0.
                && bounds.height > 0.
                && screen.scale > 0.
                && number > 0,
            "Invalid native viewer viewport evidence"
        );
        eprintln!(
            "Display viewport evidence: {}",
            serde_json::json!({
                "viewer_pid": pid, "window_number": number,
                "viewport_cg": bounds, "width": screen.width, "height": screen.height,
                "scale": screen.scale, "fullscreen": fullscreen,
            })
        );
    }
    Ok(())
}

fn fit_screen(screen: &Screen, config: &mut Config) -> Result<()> {
    let (width, height) = if config.mode == Mode::Extend {
        (
            screen
                .usable_width
                .unwrap_or(screen.width)
                .min(screen.width),
            screen
                .usable_height
                .unwrap_or(screen.height)
                .min(screen.height),
        )
    } else {
        (screen.width, screen.height)
    };
    ensure!(
        width > 0 && height > 0 && screen.scale.is_finite() && screen.scale > 0.0,
        "Invalid display geometry"
    );
    let retina = config.retina && screen.scale >= 1.5;
    let scale = if retina { 2.0 } else { 1.0 };
    let fit = (4096.0 / (width as f64 * scale))
        .min(4096.0 / (height as f64 * scale))
        .min(1.0);
    // Align encoded pixels, not logical points: Retina may have odd logical
    // dimensions while its backing raster remains even for 4:2:0 codecs.
    let alignment = if retina { 1 } else { 2 };
    config.width = (width as f64 * fit) as u32 / alignment * alignment;
    config.height = (height as f64 * fit) as u32 / alignment * alignment;
    config.retina = retina;
    config.validate()
}

fn drain_diagnostics(mut reader: impl Read, diagnostics: &Mutex<String>) {
    let mut tail = Vec::with_capacity(DIAGNOSTIC_TAIL_BYTES);
    let mut chunk = [0u8; 2048];
    loop {
        // Never hold the shared diagnostic lock while waiting for more output.
        let count = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let discard = (tail.len() + count).saturating_sub(DIAGNOSTIC_TAIL_BYTES);
        if discard > 0 {
            tail.drain(..discard);
        }
        tail.extend_from_slice(&chunk[..count]);
        *diagnostics.lock().unwrap() = String::from_utf8_lossy(&tail).trim().into();
    }
}

fn finish_helper_output(source: bool, diagnostics_done: &Receiver<()>, closed: &AtomicBool) {
    // The child closes stdout and stderr independently. Give the diagnostic
    // reader a bounded chance to retain its final failure before the source
    // loop observes EOF. Only this auxiliary reader waits; control and media
    // threads continue, and no diagnostic mutex is held during the wait.
    if source {
        let _ = diagnostics_done.recv_timeout(DIAGNOSTIC_DRAIN_GRACE);
    }
    closed.store(true, Ordering::SeqCst);
}

fn wake_writer(wake: &OnceLock<std::thread::Thread>) {
    if let Some(worker) = wake.get() {
        worker.unpark();
    }
}

fn register_writer(
    wake: &OnceLock<std::thread::Thread>,
    worker: std::thread::Thread,
    closed: &AtomicBool,
) {
    let _ = wake.set(worker);
    // EOF can precede registration after the worker's first empty check.
    // Register before checking closed; the reader stores closed before waking.
    if closed.load(Ordering::SeqCst) {
        wake_writer(wake);
    }
}

fn enqueue_cursor(
    cursors: &[Arc<Mutex<Option<Vec<u8>>>>; 2],
    wake: &OnceLock<std::thread::Thread>,
    shape: bool,
    bytes: Vec<u8>,
) {
    *cursors[usize::from(shape)].lock().unwrap() = Some(bytes);
    wake_writer(wake);
}

fn enqueue_frame(
    send: &SyncSender<Frame>,
    wake: &OnceLock<std::thread::Thread>,
    frame: Frame,
) -> bool {
    match send.try_send(frame) {
        Ok(()) => {
            wake_writer(wake);
            true
        }
        Err(TrySendError::Full(_)) => {
            if std::env::var_os("EXTEND_DISPLAY_METRICS").is_some() {
                eprintln!("Display viewer encoded queue overflow");
            }
            false
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn run_viewer_writer(
    input: &Mutex<impl Write>,
    receive: &Receiver<Frame>,
    cursors: &[Arc<Mutex<Option<Vec<u8>>>>; 2],
    closed: &AtomicBool,
    mut before_park: impl FnMut(),
) {
    loop {
        if closed.load(Ordering::SeqCst) {
            break;
        }
        let frame = match receive.try_recv() {
            Ok(frame) => Some(frame),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => break,
        };
        let has_cursor = cursors.iter().any(|slot| slot.lock().unwrap().is_some());
        if frame.is_none() && !has_cursor {
            // Producers enqueue before unpark. Its retained token covers an
            // arrival between these empty checks and park, without idle polls.
            // No cursor or child-input lock is held while waiting.
            before_park();
            std::thread::park();
            continue;
        }
        let mut input = input.lock().unwrap();
        if closed.load(Ordering::SeqCst) {
            break;
        }
        for (kind, slot) in cursors.iter().enumerate().rev() {
            if closed.load(Ordering::SeqCst) {
                break;
            }
            let bytes = slot.lock().unwrap().take();
            if let Some(bytes) = bytes {
                extend_computer_agent::display::metrics::cursor_event(
                    if kind == 1 {
                        "cursor_shape_pipe_started"
                    } else {
                        "cursor_position_pipe_started"
                    },
                    &bytes,
                );
                if input
                    .write_all(&[3])
                    .and_then(|_| input.write_all(&(bytes.len() as u32).to_be_bytes()))
                    .and_then(|_| input.write_all(&bytes))
                    .is_err()
                {
                    closed.store(true, Ordering::SeqCst);
                    return;
                }
                extend_computer_agent::display::metrics::cursor_event(
                    if kind == 1 {
                        "cursor_shape_pipe_written"
                    } else {
                        "cursor_position_pipe_written"
                    },
                    &bytes,
                );
            }
        }
        if closed.load(Ordering::SeqCst) {
            break;
        }
        if let Some(frame) = frame {
            let bytes = frame_payload(frame);
            if input
                .write_all(&[1])
                .and_then(|_| input.write_all(&(bytes.len() as u32).to_be_bytes()))
                .and_then(|_| input.write_all(&bytes))
                .is_err()
            {
                break;
            }
        }
    }
    closed.store(true, Ordering::SeqCst);
}

pub(super) struct Helper {
    child: Child,
    input: Arc<Mutex<ChildStdin>>,
    pub closed: Arc<AtomicBool>,
    pub keyframe: Arc<AtomicBool>,
    pub frames: Receiver<Frame>,
    pub cursors: [Arc<Mutex<Option<Vec<u8>>>>; 2],
    pub presented: Arc<AtomicUsize>,
    render_ready: Arc<AtomicBool>,
    pub target: Arc<Mutex<Option<super::input::CaptureTarget>>>,
    writer: Option<SyncSender<Frame>>,
    writer_wake: Arc<OnceLock<std::thread::Thread>>,
    error: Arc<Mutex<String>>,
    source: bool,
    configuration_sequence: AtomicUsize,
    configuration_reply: Mutex<Receiver<(u64, bool)>>,
    viewport_reply: Mutex<Receiver<(u64, Screen, bool)>>,
    viewer_input: Arc<Mutex<ViewerInput>>,
}
fn frame_payload(frame: Frame) -> Vec<u8> {
    let mut bytes = frame.timestamp.to_be_bytes().to_vec();
    bytes.push(u8::from(frame.keyframe));
    bytes.extend(frame.bytes);
    bytes
}
impl Helper {
    pub fn rendering_ready(&self) -> bool {
        rendering_ready(&self.render_ready, &self.presented)
    }

    pub fn screen_size(path: &Path, config: &mut Config) -> Result<()> {
        let output = Command::new(path).arg("screen").output()?;
        ensure!(output.status.success(), "Could not read display size");
        let screen: Screen = serde_json::from_slice(&output.stdout)?;
        fit_screen(&screen, config)
    }
    pub fn start(path: &Path, config: &Config, source: bool, physical_usb: bool) -> Result<Self> {
        ensure!(
            path.is_file(),
            "Display helper is missing. Rebuild extend.computer."
        );
        let command = if !source {
            "viewer"
        } else if config.mode == Mode::Extend {
            "extend"
        } else {
            "mirror"
        };
        let mut process = Command::new(path);
        if automatic_low_latency(
            std::env::var_os("EXTEND_DISPLAY_LOW_LATENCY_ENCODER").as_deref(),
            config.codec,
            source,
            physical_usb,
        ) {
            process.env("EXTEND_DISPLAY_LOW_LATENCY_ENCODER", "auto");
        }
        #[cfg(not(any(debug_assertions, feature = "display-benchmark")))]
        process.env_remove("EXTEND_DISPLAY_FIXED_BITRATE");
        let mut child = process
            .args([
                command,
                &config.width.to_string(),
                &config.height.to_string(),
                &config.fps.to_string(),
                &config.bitrate.to_string(),
                if config.retina { "retina" } else { "standard" },
                "0",
                config.codec.name(),
                if config.separate_cursor {
                    "cursor"
                } else {
                    "embedded"
                },
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let cursors: [Arc<Mutex<Option<Vec<u8>>>>; 2] =
            std::array::from_fn(|_| Arc::new(Mutex::new(None)));
        let cursor_read = cursors.clone();
        let input = Arc::new(Mutex::new(
            child.stdin.take().context("Display input unavailable")?,
        ));
        let mut output = child.stdout.take().context("Display output unavailable")?;
        let stderr = child
            .stderr
            .take()
            .context("Display diagnostics unavailable")?;
        let error = Arc::new(Mutex::new(String::new()));
        let diagnostics = error.clone();
        let (diagnostics_finished, diagnostics_done) = mpsc::channel();
        std::thread::spawn(move || {
            drain_diagnostics(stderr, &diagnostics);
            let _ = diagnostics_finished.send(());
        });
        let closed = Arc::new(AtomicBool::new(false));
        let writer_wake = Arc::new(OnceLock::new());
        let writer_wake_read = writer_wake.clone();
        let ended = closed.clone();
        let keyframe = Arc::new(AtomicBool::new(false));
        let requested = keyframe.clone();
        let presented = Arc::new(AtomicUsize::new(0));
        let rendered = presented.clone();
        let render_ready = Arc::new(AtomicBool::new(false));
        let render_ready_read = render_ready.clone();
        let (send, frames) = mpsc::sync_channel(2);
        let target = Arc::new(Mutex::new(None));
        let target_read = target.clone();
        let (configured, configuration_reply) = mpsc::channel();
        let (viewport, viewport_reply) = mpsc::channel();
        let viewer_input = Arc::new(Mutex::new(ViewerInput::default()));
        let viewer_input_read = viewer_input.clone();
        let helper_pid = child.id();
        std::thread::spawn(move || {
            let mut waiting_keyframe = false;
            let result = (|| -> Result<()> {
                loop {
                    let mut header = [0u8; 5];
                    output.read_exact(&mut header)?;
                    let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
                    ensure!(length <= 16 * 1024 * 1024, "Invalid helper packet length");
                    if header[0] == 4 {
                        ensure!(
                            !source && length > 0 && length <= 1024,
                            "Invalid native viewer input packet length"
                        );
                    }
                    let mut bytes = vec![0; length];
                    output.read_exact(&mut bytes)?;
                    match header[0] {
                        0 if source => {
                            let message: serde_json::Value = serde_json::from_slice(&bytes)?;
                            if let Some(value) = message.get("target") {
                                let parsed = serde_json::from_value::<super::input::CaptureTarget>(
                                    value.clone(),
                                )?;
                                parsed.validate()?;
                                *target_read.lock().unwrap() = Some(parsed);
                            }
                        }
                        0 => {}
                        1 if source => {
                            ensure!(bytes.len() > 9 && bytes[8] < 2, "Invalid helper frame");
                            let frame = Frame {
                                timestamp: u64::from_be_bytes(bytes[..8].try_into().unwrap()),
                                keyframe: bytes[8] == 1,
                                bytes: bytes[9..].to_vec(),
                            };
                            if waiting_keyframe && !frame.keyframe {
                                continue;
                            }
                            match send.try_send(frame) {
                                Ok(()) => waiting_keyframe = false,
                                Err(TrySendError::Full(_)) => {
                                    if std::env::var_os("EXTEND_DISPLAY_METRICS").is_some() {
                                        eprintln!("Display source encoded queue overflow");
                                    }
                                    waiting_keyframe = true;
                                    requested.store(true, Ordering::SeqCst);
                                }
                                Err(TrySendError::Disconnected(_)) => break,
                            }
                        }
                        3 if source => {
                            extend_computer_agent::display::validate_cursor(&bytes)?;
                            let shape = serde_json::from_slice::<serde_json::Value>(&bytes)?
                                .get("png")
                                .is_some();
                            extend_computer_agent::display::metrics::cursor_event(
                                if shape {
                                    "cursor_shape_helper_pipe_received"
                                } else {
                                    "cursor_position_helper_pipe_received"
                                },
                                &bytes,
                            );
                            *cursor_read[usize::from(shape)].lock().unwrap() = Some(bytes);
                        }
                        4 if !source => {
                            let event: super::receiver::NativeEvent =
                                serde_json::from_slice(&bytes)?;
                            if let super::receiver::NativeEvent::Input { command, .. } = &event {
                                command.validate()?;
                            }
                            viewer_input_read.lock().unwrap().push(event);
                        }
                        2 => {
                            let message: serde_json::Value = serde_json::from_slice(&bytes)?;
                            if message["type"] == "render_ready" {
                                update_render_ready(&message, source, &render_ready_read)?;
                            }
                            if !source && message["type"] == "viewport_evidence" {
                                let screen = serde_json::from_value::<Screen>(message.clone())?;
                                log_viewport_evidence(
                                    helper_pid,
                                    &screen,
                                    message["fullscreen"].as_bool() == Some(true),
                                )?;
                            }
                            if source && message["type"] == "target" {
                                let parsed = serde_json::from_value::<super::input::CaptureTarget>(
                                    message["target"].clone(),
                                )?;
                                parsed.validate()?;
                                *target_read.lock().unwrap() = Some(parsed);
                            }
                            if message["type"] == "viewport" {
                                if let (Some(id), Some(fullscreen), Ok(screen)) = (
                                    message["id"].as_u64(),
                                    message["fullscreen"].as_bool(),
                                    serde_json::from_value::<Screen>(message.clone()),
                                ) {
                                    let _ = viewport.send((id, screen, fullscreen));
                                }
                            }
                            if message["type"] == "configured" {
                                if let (Some(id), Some(ok)) =
                                    (message["id"].as_u64(), message["ok"].as_bool())
                                {
                                    let _ = configured.send((id, ok));
                                }
                            }
                            if message["type"] == "closed" {
                                break;
                            }
                            if message["type"] == "keyframe" {
                                requested.store(true, Ordering::SeqCst);
                            }
                            if message["type"] == "presented" {
                                if let Some(count) = message["frames"].as_u64() {
                                    rendered.store(count as usize, Ordering::SeqCst);
                                }
                            }
                        }
                        _ => anyhow::bail!("Unexpected display helper packet"),
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_none_or(|error| error.kind() != std::io::ErrorKind::UnexpectedEof)
                {
                    eprintln!("Display helper stream ended: {error}");
                }
            }
            finish_helper_output(source, &diagnostics_done, &ended);
            wake_writer(&writer_wake_read);
        });
        let writer = if source {
            None
        } else {
            let (send, receive) = mpsc::sync_channel::<Frame>(4);
            let input = input.clone();
            let cursor_write = cursors.clone();
            let ended = closed.clone();
            let worker = std::thread::spawn(move || {
                run_viewer_writer(&input, &receive, &cursor_write, &ended, || {});
            });
            register_writer(&writer_wake, worker.thread().clone(), &closed);
            Some(send)
        };
        Ok(Self {
            child,
            input,
            closed,
            keyframe,
            frames,
            cursors,
            presented,
            render_ready,
            target,
            writer,
            writer_wake,
            error,
            source,
            configuration_sequence: AtomicUsize::new(0),
            configuration_reply: Mutex::new(configuration_reply),
            viewport_reply: Mutex::new(viewport_reply),
            viewer_input,
        })
    }
    pub fn take_viewer_input(&self) -> (bool, Vec<super::receiver::NativeEvent>) {
        let mut input = self.viewer_input.lock().unwrap();
        let overflow = std::mem::take(&mut input.overflow);
        (overflow, input.events.drain(..).collect())
    }
    pub fn input_state(&self, state: &super::receiver::State) -> Result<()> {
        ensure!(!self.source, "Only a viewer receives input state");
        let bytes = serde_json::to_vec(state)?;
        ensure!(
            bytes.len() <= 1024,
            "Native viewer input state is too large"
        );
        let mut input = self.input.lock().unwrap();
        input.write_all(&[2])?;
        input.write_all(&(bytes.len() as u32).to_be_bytes())?;
        input.write_all(&bytes)?;
        Ok(())
    }
    pub fn fit_viewport(&self, config: &mut Config) -> Result<()> {
        ensure!(!self.source, "Only a viewer can report its viewport");
        let deadline = Instant::now() + Duration::from_secs(4);
        let replies = self.viewport_reply.lock().unwrap();
        loop {
            let id = self.configuration_sequence.fetch_add(1, Ordering::SeqCst) as u64 + 1;
            let bytes = serde_json::to_vec(&serde_json::json!({"type":"viewport","id":id}))?;
            {
                let mut input = self.input.lock().unwrap();
                input.write_all(&[2])?;
                input.write_all(&(bytes.len() as u32).to_be_bytes())?;
                input.write_all(&bytes)?;
            }
            let screen = loop {
                let (received, screen, fullscreen) = replies
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .context("Native viewer fullscreen geometry did not respond")?;
                if received != id {
                    continue;
                }
                break fullscreen.then_some(screen);
            };
            if let Some(screen) = screen {
                log_viewport_evidence(self.child.id(), &screen, true)?;
                fit_screen(&screen, config)?;
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "Display viewer could not enter fullscreen"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    pub fn configure(&self, config: &Config) -> Result<()> {
        config.validate()?;
        let id = self.configuration_sequence.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        let bytes = serde_json::to_vec(&serde_json::json!({"type":"configuration","id":id,
            "width":config.width,"height":config.height,"retina":config.retina,"fps":config.fps,"bitrate":config.bitrate}))?;
        {
            let mut input = self.input.lock().unwrap();
            if self.source {
                input.write_all(&bytes)?;
                input.write_all(b"\n")?;
            } else {
                input.write_all(&[2])?;
                input.write_all(&(bytes.len() as u32).to_be_bytes())?;
                input.write_all(&bytes)?;
            }
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let replies = self.configuration_reply.lock().unwrap();
        loop {
            let (received, ok) = replies
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .context("Native display settings did not respond")?;
            if received == id {
                ensure!(ok, "Native display settings could not be applied");
                return Ok(());
            }
        }
    }
    pub fn pause_capture(&self, paused: bool) -> Result<()> {
        let control = if paused {
            b"{\"type\":\"pause\",\"value\":true}\n".as_slice()
        } else {
            b"{\"type\":\"pause\",\"value\":false}\n".as_slice()
        };
        self.input.lock().unwrap().write_all(control)?;
        Ok(())
    }
    pub fn request_keyframe(&self) -> Result<()> {
        self.input
            .lock()
            .unwrap()
            .write_all(b"{\"type\":\"keyframe\"}\n")?;
        Ok(())
    }
    pub fn set_bitrate(&self, value: u32) -> Result<()> {
        writeln!(
            self.input.lock().unwrap(),
            "{{\"type\":\"bitrate\",\"value\":{value}}}"
        )?;
        Ok(())
    }
    pub fn cursor(&self, bytes: Vec<u8>) {
        let shape = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|v| v.get("png").is_some());
        extend_computer_agent::display::metrics::cursor_event(
            if shape {
                "cursor_shape_runtime_received"
            } else {
                "cursor_position_runtime_received"
            },
            &bytes,
        );
        enqueue_cursor(&self.cursors, &self.writer_wake, shape, bytes);
    }
    pub fn present(&self, frame: Frame) -> bool {
        let Some(send) = self.writer.as_ref() else {
            return false;
        };
        enqueue_frame(send, &self.writer_wake, frame)
    }
    pub fn failure(&self) -> anyhow::Error {
        let error = self.error.lock().unwrap().clone();
        anyhow::anyhow!(if error.is_empty() {
            "Display helper stopped".into()
        } else {
            error
        })
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        self.writer.take();
        self.closed.store(true, Ordering::SeqCst);
        wake_writer(&self.writer_wake);
        // Capture needs a graceful stop to release its virtual display before exit.
        if let Ok(mut input) = self.input.try_lock() {
            if self.source {
                let _ = input.write_all(b"{\"type\":\"stop\"}\n");
            } else {
                let payload = b"{\"type\":\"stop\"}";
                let _ = input
                    .write_all(&[2])
                    .and_then(|_| input.write_all(&(payload.len() as u32).to_be_bytes()))
                    .and_then(|_| input.write_all(payload));
            }
        }
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod viewer_writer_tests {
    use super::*;

    struct PacketWriter {
        pending: Vec<u8>,
        packets: mpsc::Sender<(u8, Vec<u8>)>,
        fail: bool,
    }
    impl Write for PacketWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.pending.extend_from_slice(bytes);
            while self.pending.len() >= 5 {
                let length = u32::from_be_bytes(self.pending[1..5].try_into().unwrap()) as usize;
                if self.pending.len() < length + 5 {
                    break;
                }
                let packet = self.pending.drain(..length + 5).collect::<Vec<_>>();
                self.packets
                    .send((packet[0], packet[5..].to_vec()))
                    .unwrap();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Harness {
        send: Option<SyncSender<Frame>>,
        input: Arc<Mutex<PacketWriter>>,
        cursors: [Arc<Mutex<Option<Vec<u8>>>>; 2],
        closed: Arc<AtomicBool>,
        wake: Arc<OnceLock<std::thread::Thread>>,
        packets: Receiver<(u8, Vec<u8>)>,
        done: Receiver<()>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl Harness {
        fn new(before_park: impl FnMut() + Send + 'static, register: bool) -> Self {
            let (send, receive) = mpsc::sync_channel(4);
            let (packets, received) = mpsc::channel();
            let input = Arc::new(Mutex::new(PacketWriter {
                pending: vec![],
                packets,
                fail: false,
            }));
            let cursors: [Arc<Mutex<Option<Vec<u8>>>>; 2] =
                std::array::from_fn(|_| Arc::new(Mutex::new(None)));
            let closed = Arc::new(AtomicBool::new(false));
            let wake = Arc::new(OnceLock::new());
            let (finished, done) = mpsc::channel();
            let worker_input = input.clone();
            let worker_cursors = cursors.clone();
            let worker_closed = closed.clone();
            let worker = std::thread::spawn(move || {
                run_viewer_writer(
                    &worker_input,
                    &receive,
                    &worker_cursors,
                    &worker_closed,
                    before_park,
                );
                finished.send(()).unwrap();
            });
            if register {
                register_writer(&wake, worker.thread().clone(), &closed);
            }
            Self {
                send: Some(send),
                input,
                cursors,
                closed,
                wake,
                packets: received,
                done,
                worker: Some(worker),
            }
        }
        fn frame(&self, id: u64) -> bool {
            enqueue_frame(self.send.as_ref().unwrap(), &self.wake, frame(id))
        }
        fn cursor(&self, shape: bool, bytes: &[u8]) {
            enqueue_cursor(&self.cursors, &self.wake, shape, bytes.to_vec());
        }
        fn packet(&self) -> (u8, Vec<u8>) {
            self.packets
                .recv_timeout(Duration::from_secs(1))
                .expect("Writer did not deliver work")
        }
        fn join(&mut self) {
            self.done
                .recv_timeout(Duration::from_secs(1))
                .expect("Writer did not stop");
            self.worker.take().unwrap().join().unwrap();
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.closed.store(true, Ordering::SeqCst);
            self.send.take();
            if let Some(worker) = self.worker.take() {
                worker.thread().unpark();
                if self.done.recv_timeout(Duration::from_secs(1)).is_ok() {
                    let _ = worker.join();
                }
            }
        }
    }
    fn frame(id: u64) -> Frame {
        Frame {
            timestamp: id,
            keyframe: false,
            bytes: vec![id as u8],
        }
    }
    fn first_park_gate() -> (impl FnMut() + Send, Receiver<()>, Arc<std::sync::Barrier>) {
        let (checked, empty) = mpsc::channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let resume = release.clone();
        let mut first = true;
        (
            move || {
                if first {
                    first = false;
                    checked.send(()).unwrap();
                    // A channel recv can itself consume this thread's unpark
                    // token. A barrier keeps the tested park token untouched.
                    resume.wait();
                }
            },
            empty,
            release,
        )
    }

    #[test]
    fn arrival_between_empty_check_and_park_delivers_latest_cursor_without_video() {
        let (gate, empty, release) = first_park_gate();
        let mut test = Harness::new(gate, true);
        empty.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(test.input.try_lock().is_ok());
        assert!(test.cursors.iter().all(|slot| slot.try_lock().is_ok()));
        test.cursor(false, br#"{"revision":1,"sequence":1}"#);
        test.cursor(false, br#"{"revision":1,"sequence":2}"#);
        // Both notifications precede park and coalesce into its retained token.
        release.wait();
        assert_eq!(
            test.packet(),
            (3, br#"{"revision":1,"sequence":2}"#.to_vec())
        );
        assert!(test.packets.try_recv().is_err());
        test.closed.store(true, Ordering::SeqCst);
        wake_writer(&test.wake);
        test.join();
    }

    #[test]
    fn shape_then_latest_position_precede_bounded_frames() {
        let (gate, empty, release) = first_park_gate();
        let mut test = Harness::new(gate, true);
        empty.recv_timeout(Duration::from_secs(1)).unwrap();
        for id in 1..=4 {
            assert!(test.frame(id));
        }
        assert!(!test.frame(5), "The original four-frame bound changed");
        test.cursor(false, br#"{"revision":2,"sequence":3}"#);
        test.cursor(false, br#"{"revision":2,"sequence":4}"#);
        test.cursor(true, br#"{"revision":2,"png":"shape"}"#);
        release.wait();
        assert_eq!(
            test.packet(),
            (3, br#"{"revision":2,"png":"shape"}"#.to_vec())
        );
        assert_eq!(
            test.packet(),
            (3, br#"{"revision":2,"sequence":4}"#.to_vec())
        );
        for id in 1..=4 {
            assert_eq!(test.packet(), (1, frame_payload(frame(id))));
        }
        test.send.take();
        wake_writer(&test.wake);
        test.join();
        assert!(test.closed.load(Ordering::SeqCst));
    }

    #[test]
    fn frame_and_cursor_arrivals_wake_an_idle_writer_independently() {
        let (parked, idle) = mpsc::channel();
        let mut test = Harness::new(
            move || {
                parked.send(()).unwrap();
            },
            true,
        );
        idle.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(test.frame(7));
        assert_eq!(test.packet(), (1, frame_payload(frame(7))));
        idle.recv_timeout(Duration::from_secs(1)).unwrap();
        test.cursor(false, br#"{"revision":1,"sequence":8}"#);
        assert_eq!(
            test.packet(),
            (3, br#"{"revision":1,"sequence":8}"#.to_vec())
        );
        test.closed.store(true, Ordering::SeqCst);
        wake_writer(&test.wake);
        test.join();
    }

    #[test]
    fn eof_before_wake_registration_covers_the_empty_check_park_race() {
        let (gate, empty, release) = first_park_gate();
        let mut test = Harness::new(gate, false);
        empty.recv_timeout(Duration::from_secs(1)).unwrap();
        let (_stderr_pending, diagnostics_done) = mpsc::channel();
        finish_helper_output(false, &diagnostics_done, &test.closed);
        wake_writer(&test.wake);
        assert!(test.wake.get().is_none());
        register_writer(
            &test.wake,
            test.worker.as_ref().unwrap().thread().clone(),
            &test.closed,
        );
        release.wait();
        test.join();
        assert!(test.packets.try_recv().is_err());
    }

    #[test]
    fn output_eof_wakes_registered_idle_writer_without_final_media() {
        let (parked, idle) = mpsc::channel();
        let mut test = Harness::new(
            move || {
                parked.send(()).unwrap();
            },
            true,
        );
        idle.recv_timeout(Duration::from_secs(1)).unwrap();
        let (_stderr_pending, diagnostics_done) = mpsc::channel();
        finish_helper_output(false, &diagnostics_done, &test.closed);
        wake_writer(&test.wake);
        test.join();
        assert!(test.packets.try_recv().is_err());
    }

    #[test]
    fn sender_disconnection_wakes_parked_writer() {
        let (parked, idle) = mpsc::channel();
        let mut test = Harness::new(
            move || {
                parked.send(()).unwrap();
            },
            true,
        );
        idle.recv_timeout(Duration::from_secs(1)).unwrap();
        test.send.take();
        wake_writer(&test.wake);
        test.join();
        assert!(test.closed.load(Ordering::SeqCst));
        assert!(test.packets.try_recv().is_err());
    }

    #[test]
    fn closed_while_waiting_for_input_lock_discards_queued_work() {
        let (send, receive) = mpsc::sync_channel(4);
        for id in 1..=4 {
            send.try_send(frame(id)).unwrap();
        }
        let (packets, observed) = mpsc::channel();
        let input = Arc::new(Mutex::new(PacketWriter {
            pending: vec![],
            packets,
            fail: false,
        }));
        let locked = input.lock().unwrap();
        let cursors: [Arc<Mutex<Option<Vec<u8>>>>; 2] =
            std::array::from_fn(|_| Arc::new(Mutex::new(Some(vec![1]))));
        let closed = Arc::new(AtomicBool::new(false));
        let worker_input = input.clone();
        let worker_closed = closed.clone();
        let (finished, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_viewer_writer(&worker_input, &receive, &cursors, &worker_closed, || {});
            finished.send(()).unwrap();
        });
        // A free queue slot proves the worker passed its initial closed check
        // and consumed a frame; the held input lock still prevents any write.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match send.try_send(frame(5)) {
                Ok(()) => break,
                Err(TrySendError::Full(_)) if Instant::now() < deadline => std::thread::yield_now(),
                _ => panic!("Writer did not reach the input lock"),
            }
        }
        closed.store(true, Ordering::SeqCst);
        worker.thread().unpark();
        drop(locked);
        done.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
        assert!(observed.try_recv().is_err());
    }

    #[test]
    fn broken_pipe_closes_writer_and_rejects_further_frames() {
        let (gate, empty, release) = first_park_gate();
        let mut test = Harness::new(gate, true);
        empty.recv_timeout(Duration::from_secs(1)).unwrap();
        test.input.lock().unwrap().fail = true;
        assert!(test.frame(9));
        release.wait();
        test.join();
        assert!(test.closed.load(Ordering::SeqCst));
        assert!(!test.frame(10));
        assert!(test.packets.try_recv().is_err());
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::{
        automatic_low_latency, drain_diagnostics, finish_helper_output, fit_screen, Config, Mode,
        Screen, DIAGNOSTIC_DRAIN_GRACE, DIAGNOSTIC_TAIL_BYTES,
    };
    use extend_computer_agent::session::display::VideoCodec;
    use std::{io::Read, sync::Mutex};

    #[test]
    fn source_eof_waits_for_final_native_failure_without_holding_diagnostic_lock() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        };
        let diagnostics = Arc::new(Mutex::new(String::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let (finished, done) = mpsc::channel();
        let (started, running) = mpsc::channel();
        let diagnostic_read = diagnostics.clone();
        let ended = closed.clone();
        let reader = std::thread::spawn(move || {
            started.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
            assert!(!ended.load(Ordering::SeqCst));
            assert!(diagnostic_read.try_lock().is_ok());
            drain_diagnostics(
                "Screen capture did not provide video frames.".as_bytes(),
                &diagnostic_read,
            );
            finished.send(()).unwrap();
        });
        running.recv().unwrap();
        finish_helper_output(true, &done, &closed);
        reader.join().unwrap();
        assert!(closed.load(Ordering::SeqCst));
        assert_eq!(
            *diagnostics.lock().unwrap(),
            "Screen capture did not provide video frames."
        );
    }

    #[test]
    fn source_eof_grace_is_bounded_and_viewer_eof_does_not_wait_for_stderr() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        let (_pending, done) = mpsc::channel();
        let viewer_closed = AtomicBool::new(false);
        let before = std::time::Instant::now();
        finish_helper_output(false, &done, &viewer_closed);
        assert!(viewer_closed.load(Ordering::SeqCst));
        assert!(before.elapsed() < DIAGNOSTIC_DRAIN_GRACE);
        let source_closed = AtomicBool::new(false);
        let before = std::time::Instant::now();
        finish_helper_output(true, &done, &source_closed);
        assert!(source_closed.load(Ordering::SeqCst));
        assert!(before.elapsed() >= DIAGNOSTIC_DRAIN_GRACE);
        assert!(before.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn rendered_viewer_can_be_ready_without_a_compositor_presentation_callback() {
        use super::{rendering_ready, update_render_ready, AtomicBool, AtomicUsize, Ordering};
        let ready = AtomicBool::new(false);
        let presented = AtomicUsize::new(0);
        assert!(!rendering_ready(&ready, &presented));
        let message = serde_json::from_slice(br#"{"type":"render_ready","ready":true}"#).unwrap();
        update_render_ready(&message, false, &ready).unwrap();
        assert!(rendering_ready(&ready, &presented));
        assert_eq!(presented.load(Ordering::SeqCst), 0);
        update_render_ready(&message, false, &ready).unwrap();
        assert_eq!(presented.load(Ordering::SeqCst), 0);

        // An older helper can establish readiness through its actual
        // compositor callback without sending the new milestone.
        let old_helper_ready = AtomicBool::new(false);
        presented.store(1, Ordering::SeqCst);
        assert!(rendering_ready(&old_helper_ready, &presented));
    }

    #[test]
    fn malformed_or_source_render_readiness_cannot_advance_viewer_startup() {
        use super::{update_render_ready, AtomicBool, Ordering};
        for bytes in [
            br#"{"type":"render_ready"}"#.as_slice(),
            br#"{"type":"render_ready","ready":false}"#,
            br#"{"type":"render_ready","ready":"true"}"#,
            br#"{"type":"render_ready","ready":true,"frames":60}"#,
            br#"{"type":"presented","ready":true}"#,
        ] {
            let ready = AtomicBool::new(false);
            let message = serde_json::from_slice(bytes).unwrap();
            assert!(update_render_ready(&message, false, &ready).is_err());
            assert!(!ready.load(Ordering::SeqCst));
        }
        let ready = AtomicBool::new(false);
        let message = serde_json::from_slice(br#"{"type":"render_ready","ready":true}"#).unwrap();
        assert!(update_render_ready(&message, true, &ready).is_err());
        assert!(!ready.load(Ordering::SeqCst));
    }

    #[test]
    fn native_viewer_queue_bounds_transitions_coalesces_pointer_and_prioritizes_focus_loss() {
        use super::super::receiver::NativeEvent;
        use extend_computer_agent::display::input::{Command, Point};
        let mut queue = super::ViewerInput::default();
        for x in [0.1, 0.2, 0.3] {
            queue.push(NativeEvent::Input {
                revision: 1,
                command: Command::Move {
                    point: Point { x, y: 0.5 },
                },
            });
        }
        assert_eq!(queue.events.len(), 1);
        queue.push(NativeEvent::Focus {
            active: false,
            revision: 1,
        });
        assert_eq!(queue.events.len(), 1);
        assert!(matches!(
            queue.events.front(),
            Some(NativeEvent::Focus { active: false, .. })
        ));
        for _ in 0..64 {
            queue.push(NativeEvent::Focus {
                active: true,
                revision: 1,
            });
        }
        assert!(queue.overflow && queue.events.is_empty());
        queue.push(NativeEvent::Focus {
            active: true,
            revision: 1,
        });
        assert!(queue.events.is_empty());
    }

    #[test]
    fn low_latency_preference_requires_hevc_source_on_physical_usb() {
        assert!(automatic_low_latency(None, VideoCodec::Hevc, true, true));
        assert!(!automatic_low_latency(None, VideoCodec::Hevc, true, false));
        assert!(!automatic_low_latency(None, VideoCodec::Hevc, false, true));
        assert!(!automatic_low_latency(None, VideoCodec::H264, true, true));
        for value in ["0", "1", "auto", "unexpected"] {
            assert!(!automatic_low_latency(
                Some(std::ffi::OsStr::new(value)),
                VideoCodec::Hevc,
                true,
                true
            ));
        }
    }

    fn config(mode: Mode, retina: bool) -> Config {
        Config {
            mode,
            width: 1440,
            height: 900,
            fps: 60,
            bitrate: 20_000_000,
            retina,
            auto_size: true,
            codec: VideoCodec::Hevc,
            separate_cursor: true,
        }
    }

    #[test]
    fn extend_fits_fullscreen_safe_area_while_mirror_keeps_full_panel() {
        let screen = Screen {
            width: 1728,
            height: 1117,
            scale: 2.0,
            usable_width: Some(1728),
            usable_height: Some(1084),
            viewport_cg: None,
            window_number: None,
        };
        let mut extend = config(Mode::Extend, true);
        fit_screen(&screen, &mut extend).unwrap();
        assert_eq!((extend.width, extend.height), (1728, 1084));
        let mut mirror = config(Mode::Mirror, true);
        fit_screen(&screen, &mut mirror).unwrap();
        assert_eq!((mirror.width, mirror.height), (1728, 1117));
    }

    #[test]
    fn retina_preserves_odd_logical_height_but_standard_aligns_encoded_pixels() {
        let screen = Screen {
            width: 1728,
            height: 1085,
            scale: 2.0,
            usable_width: None,
            usable_height: None,
            viewport_cg: None,
            window_number: None,
        };
        let mut retina = config(Mode::Extend, true);
        fit_screen(&screen, &mut retina).unwrap();
        assert_eq!(retina.height, 1085);
        let mut standard = config(Mode::Extend, false);
        fit_screen(&screen, &mut standard).unwrap();
        assert_eq!(standard.height, 1084);
    }

    #[test]
    fn drains_past_retention_limit_and_keeps_latest_error_without_blocking_lock() {
        let diagnostics = Mutex::new(String::new());
        let mut data = vec![b'x'; DIAGNOSTIC_TAIL_BYTES * 3];
        data.extend_from_slice(b"\nLast diagnostic: encoder failed.");
        struct CheckedReader<'a> {
            data: &'a [u8],
            diagnostics: &'a Mutex<String>,
            consumed: usize,
            interrupted: bool,
        }
        impl Read for CheckedReader<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                assert!(
                    self.diagnostics.try_lock().is_ok(),
                    "Diagnostic lock held during read"
                );
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                let count = output.len().min(self.data.len() - self.consumed);
                output[..count].copy_from_slice(&self.data[self.consumed..self.consumed + count]);
                self.consumed += count;
                Ok(count)
            }
        }
        let mut reader = CheckedReader {
            data: &data,
            diagnostics: &diagnostics,
            consumed: 0,
            interrupted: false,
        };
        drain_diagnostics(&mut reader, &diagnostics);
        assert_eq!(reader.consumed, data.len());
        let expected = String::from_utf8_lossy(&data[data.len() - DIAGNOSTIC_TAIL_BYTES..]);
        assert_eq!(*diagnostics.lock().unwrap(), expected);
    }
}
