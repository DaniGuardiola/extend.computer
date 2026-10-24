//! Desktop-compatible pinned signaling and encrypted media. Encoded frames never reach JS.
use anyhow::{ensure, Context, Result};
#[cfg(test)]
use extend_computer_agent::display::Transport;
use extend_computer_agent::{
    display::{DisplayEvent, Frame},
    identity::Identity,
    session::display::{
        settings::{ReceiverSettings, Settings},
        Config, DisplaySignal,
    },
    trust::TrustStore,
};
use std::{
    collections::VecDeque,
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
static IDS: AtomicU64 = AtomicU64::new(1);
#[path = "media_input.rs"]
mod input_adapter;
use input_adapter::ReceiverInput;
#[path = "media_settings.rs"]
mod settings_adapter;
use settings_adapter::ApplyGate;

// Independent latest-state slots: cursor delivery never waits for a video frame
// or allocates a queue proportional to pointer movement. Watermarks survive a
// consumer drain so reordered unreliable positions cannot replace newer state.
#[derive(Default)]
struct CursorMailbox {
    shape: Option<Vec<u8>>,
    position: Option<Vec<u8>>,
    shape_revision: Option<u64>,
    position_sequence: Option<u64>,
    received: u64,
    delivered: u64,
}
impl CursorMailbox {
    fn push(&mut self, bytes: Vec<u8>) -> Result<()> {
        extend_computer_agent::display::validate_cursor(&bytes)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value.get("png").is_some() {
            let revision = value["revision"]
                .as_u64()
                .context("Missing cursor revision")?;
            if self.shape_revision.is_some_and(|old| revision < old) {
                return Ok(());
            }
            self.shape_revision = Some(revision);
            self.shape = Some(bytes);
        } else {
            let sequence = value["sequence"]
                .as_u64()
                .context("Missing cursor sequence")?;
            if self.position_sequence.is_some_and(|old| sequence <= old) {
                return Ok(());
            }
            self.position_sequence = Some(sequence);
            self.position = Some(bytes);
        }
        self.received += 1;
        Ok(())
    }
    fn take(&mut self) -> Option<Vec<u8>> {
        let bytes = self.shape.take().or_else(|| self.position.take());
        if bytes.is_some() {
            self.delivered += 1;
        }
        bytes
    }
    fn clear(&mut self) {
        self.shape = None;
        self.position = None;
    }
    fn empty(&self) -> bool {
        self.shape.is_none() && self.position.is_none()
    }
}

const DECODER_STARTUP_FRAMES: usize = 64;
const DECODER_STREAMING_FRAMES: usize = 16;
const DECODER_QUEUE_BYTES: usize = 32 * 1024 * 1024;
const DECODER_STARTUP: Duration = Duration::from_secs(1);

// MediaCodec is created only after Java consumes the first encoded keyframe.
// RTC negotiation/capture startup must not spend that decoder's startup budget.
// Each revision gets the same bounded allowance; once rendering and consumption
// catch up, the smaller streaming bound applies immediately and stays latched.
#[derive(Default)]
struct DecoderBudget {
    first_frame: Option<Instant>,
    steady: bool,
}
impl DecoderBudget {
    fn observe_progress(&mut self, rendered: bool, queued: usize) {
        if self.first_frame.is_some() && rendered && queued < DECODER_STREAMING_FRAMES {
            self.steady = true;
        }
    }
    fn elapsed(&self, now: Instant) -> Option<Duration> {
        self.first_frame
            .map(|first| now.saturating_duration_since(first))
    }
    fn limit(&self, now: Instant) -> usize {
        if self.steady || self.elapsed(now).is_some_and(|age| age >= DECODER_STARTUP) {
            DECODER_STREAMING_FRAMES
        } else {
            DECODER_STARTUP_FRAMES
        }
    }
    fn admit(
        &mut self,
        now: Instant,
        queued: usize,
        bytes: usize,
        incoming: usize,
        rendered: bool,
    ) -> Result<()> {
        self.first_frame.get_or_insert(now);
        self.observe_progress(rendered, queued);
        let limit = self.limit(now);
        ensure!(
            queued < limit
                && bytes
                    .checked_add(incoming)
                    .is_some_and(|total| total <= DECODER_QUEUE_BYTES),
            "Hardware decoder isn't keeping up (queued_frames={queued}, queued_bytes={bytes}, incoming_bytes={incoming}, frame_limit={limit}, startup_ms={}, rendered={rendered})",
            self.elapsed(now).unwrap_or_default().as_millis()
        );
        Ok(())
    }
}
struct QueuedFrame {
    frame: Frame,
    arrived: Instant,
}
#[derive(Default)]
struct DecoderProgress {
    last_take: Option<Instant>,
    last_render_ack: Option<Instant>,
}
impl DecoderProgress {
    // Terminal diagnostics only. Ages share Rust's local monotonic clock;
    // source PTS spans remain separate and are never subtracted from that clock.
    fn diagnostic(
        &self,
        now: Instant,
        frames: &VecDeque<QueuedFrame>,
        incoming_pts: u64,
        received: u64,
        rendered: u64,
        revision: u64,
    ) -> serde_json::Value {
        let age = |instant: Option<Instant>| {
            instant.map(|value| now.saturating_duration_since(value).as_millis() as u64)
        };
        let first = frames.front();
        let last = frames.back();
        serde_json::json!({
            "age_clock":"rust-instant",
            "last_take_age_ms":age(self.last_take),
            "last_render_ack_age_ms":age(self.last_render_ack),
            "oldest_arrival_age_ms":age(first.map(|entry|entry.arrived)),
            "queued_first_pts_ns":first.map(|entry|entry.frame.timestamp),
            "queued_last_pts_ns":last.map(|entry|entry.frame.timestamp),
            "incoming_pts_ns":incoming_pts,
            "queued_pts_span_ns":first.zip(last).and_then(|(a,b)|b.frame.timestamp.checked_sub(a.frame.timestamp)),
            "incoming_after_oldest_pts_ns":first.and_then(|entry|incoming_pts.checked_sub(entry.frame.timestamp)),
            "received":received,"rendered":rendered,"revision":revision
        })
    }
}
struct Session {
    id: String,
    peer: String,
    route: &'static str,
    config: Config,
    phase: &'static str,
    ready: bool,
    revision: u64,
    rendered_revision: u64,
    error: String,
    frames: VecDeque<QueuedFrame>,
    decoder_budget: DecoderBudget,
    decoder_progress: DecoderProgress,
    cursor: CursorMailbox,
    input: ReceiverInput,
    settings: Arc<ReceiverSettings>,
    settings_gate: ApplyGate,
    bytes: usize,
    received: u64,
    rendered: u64,
    received_timestamp: u64,
    rendered_timestamp: u64,
    render_device_ns: u64,
    callback_device_ns: u64,
    render_clock_valid: bool,
}
struct State {
    running: bool,
    socket: Option<TcpStream>,
    session: Option<Session>,
}
struct Shared {
    capabilities: serde_json::Value,
    state: Mutex<State>,
    changed: Condvar,
}
pub struct Server {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
    port: u16,
}
impl Server {
    pub fn start(
        identity: Arc<Identity>,
        trust: TrustStore,
        capabilities: serde_json::Value,
    ) -> Result<Self> {
        Self::bind_mode(identity, trust, capabilities, "0.0.0.0:48179", true)
    }
    fn bind(
        identity: Arc<Identity>,
        trust: TrustStore,
        capabilities: serde_json::Value,
        address: &str,
    ) -> Result<Self> {
        Self::bind_mode(identity, trust, capabilities, address, false)
    }
    fn bind_mode(
        identity: Arc<Identity>,
        trust: TrustStore,
        capabilities: serde_json::Value,
        address: &str,
        optional_lan: bool,
    ) -> Result<Self> {
        ensure!(capabilities.is_object(), "Missing decoder capabilities");
        let listener = match TcpListener::bind(address) {
            Ok(listener) => Some(listener),
            Err(_) if optional_lan => None,
            Err(error) => return Err(error).context("Display receiving port is in use"),
        };
        let port = listener
            .as_ref()
            .map(|listener| listener.local_addr().map(|a| a.port()))
            .transpose()?
            .unwrap_or(0);
        let shared = Arc::new(Shared {
            capabilities: capabilities.clone(),
            state: Mutex::new(State {
                running: true,
                socket: None,
                session: None,
            }),
            changed: Condvar::new(),
        });
        let Some(listener) = listener else {
            return Ok(Self {
                shared,
                worker: Mutex::new(None),
                port,
            });
        };
        let active = shared.clone();
        let worker = thread::Builder::new()
            .name("extend-media-receiver".into())
            .spawn(move || {
                while active.state.lock().unwrap().running {
                    let socket = match listener.accept() {
                        Ok((socket, _)) => socket,
                        Err(_) => break,
                    };
                    {
                        let mut state = active.state.lock().unwrap();
                        if !state.running {
                            break;
                        }
                        if state.socket.is_some() {
                            continue;
                        }
                        let Ok(clone) = socket.try_clone() else {
                            continue;
                        };
                        state.socket = Some(clone);
                    }
                    let result = serve(socket, &identity, &trust, &capabilities, &active, None);
                    finish_connection(&active, result);
                }
                // User availability is independent of an optional LAN listener.
                active.changed.notify_all();
            })?;
        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
            port,
        })
    }
    pub fn accept_usb(
        self: &Arc<Self>,
        socket: TcpStream,
        identity: Arc<Identity>,
        trust: TrustStore,
        route: crate::usb::Route,
    ) -> Result<()> {
        ensure!(route.current(), "USB connection changed");
        {
            let mut state = self.shared.state.lock().unwrap();
            ensure!(state.running, "Receiving disabled");
            ensure!(state.socket.is_none(), "A display is already connected");
            state.socket = Some(socket.try_clone()?);
        }
        let active = self.shared.clone();
        if let Err(error) = thread::Builder::new()
            .name("extend-usb-display".into())
            .spawn(move || {
                let result = serve(
                    socket,
                    &identity,
                    &trust,
                    &active.capabilities,
                    &active,
                    Some(route),
                );
                finish_connection(&active, result);
            })
        {
            self.shared.state.lock().unwrap().socket = None;
            return Err(error.into());
        }
        Ok(())
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.shared.state.lock().unwrap();
        match &state.session {
            Some(s) => {
                let now = Instant::now();
                serde_json::json!({"state":s.phase,"id":s.id,"peer":s.peer,"route":s.route,"config":s.config,"revision":s.revision,"rendered_revision":s.rendered_revision,"error":s.error,"received":s.received,"rendered":s.rendered,"queued_frames":s.frames.len(),"queued_bytes":s.bytes,"decoder_queue_limit":s.decoder_budget.limit(now),"decoder_startup_ms":s.decoder_budget.elapsed(now).map(|age|age.as_millis() as u64),"decoder_queue_steady":s.decoder_budget.steady,"cursor_received":s.cursor.received,"cursor_delivered":s.cursor.delivered,"received_timestamp_ns":s.received_timestamp,"rendered_timestamp_ns":s.rendered_timestamp,"render_device_ns":s.render_device_ns,"callback_device_ns":s.callback_device_ns,"render_clock_valid":s.render_clock_valid})
            }
            None => serde_json::json!({"state":"idle"}),
        }
    }
    pub fn attach(&self, id: &str) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id && s.phase == "offered")
            .context("Display offer no longer active")?;
        session.ready = true;
        session.phase = "connecting";
        self.shared.changed.notify_all();
        Ok(())
    }
    pub fn receiver_settings(&self, id: &str) -> Result<serde_json::Value> {
        let mut state = self.shared.state.lock().unwrap();
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id)
            .context("Display session replaced")?;
        session.observe_settings();
        Ok(session.settings_gate.snapshot(
            &session.settings.snapshot(),
            &session.id,
            session.revision,
            &session.config,
            session.phase == "streaming" && session.rendered_revision > 0,
        ))
    }
    pub fn request_receiver_settings(&self, id: &str, revision: u64, json: &str) -> Result<u64> {
        ensure!(json.len() <= 1024, "Display settings too large");
        let settings: Settings = serde_json::from_str(json)?;
        let mut state = self.shared.state.lock().unwrap();
        ensure!(state.running, "Receiving disabled");
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id)
            .context("Display session replaced")?;
        session.observe_settings();
        session.settings_gate.can_request()?;
        ensure!(
            session.phase == "streaming" && session.rendered_revision > 0,
            "Wait for the display to connect"
        );
        ensure!(session.revision == revision, "Display settings changed");
        let snapshot = session.settings.snapshot();
        ensure!(snapshot.revision == revision, "Display settings changed");
        // Exact probed profiles are checked before requesting any remote change.
        prepare_live_config(
            settings.apply(&session.config)?,
            &session.config,
            &self.shared.capabilities,
        )?;
        let request = session.settings.request(revision, settings)?;
        session.settings_gate.started(request, Instant::now());
        self.shared.changed.notify_all();
        Ok(request)
    }
    pub fn input_state(&self, id: &str) -> Result<serde_json::Value> {
        let state = self.shared.state.lock().unwrap();
        let session = state
            .session
            .as_ref()
            .filter(|s| s.id == id)
            .context("Display session replaced")?;
        Ok(session.input.snapshot(session.revision))
    }
    pub fn set_input_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id)
            .context("Display session replaced")?;
        if enabled {
            ensure!(
                matches!(session.phase, "connecting" | "streaming"),
                "Display ended"
            );
        }
        session.input.enable(enabled)?;
        self.shared.changed.notify_all();
        Ok(())
    }
    pub fn input_command(&self, id: &str, revision: u64, command: &str) -> Result<()> {
        ensure!(
            command.len() <= extend_computer_agent::display::input::MAX_MESSAGE_BYTES,
            "Display input command too large"
        );
        let command = serde_json::from_str(command)?;
        let mut state = self.shared.state.lock().unwrap();
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id)
            .context("Display session replaced")?;
        let rendered = session.phase == "streaming" && session.rendered_revision > 0;
        session
            .input
            .enqueue(revision, session.revision, rendered, command)?;
        self.shared.changed.notify_all();
        Ok(())
    }
    pub fn configured(&self, id: &str, revision: u64) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id && s.revision == revision && s.phase == "configuring")
            .context("Display configuration replaced")?;
        session.ready = true;
        session.phase = "connecting";
        self.shared.changed.notify_all();
        Ok(())
    }
    pub fn rendered(
        &self,
        id: &str,
        revision: u64,
        timestamp: u64,
        device_ns: u64,
        callback_device_ns: u64,
    ) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id && matches!(s.phase, "connecting" | "streaming" | "configuring"))
            .context("Display ended")?;
        if revision != session.revision {
            return Ok(());
        }
        session.rendered += 1;
        session.rendered_revision += 1;
        session.decoder_progress.last_render_ack = Some(Instant::now());
        session
            .decoder_budget
            .observe_progress(true, session.frames.len());
        session.rendered_timestamp = timestamp;
        session.render_device_ns = device_ns;
        session.callback_device_ns = callback_device_ns;
        // Conservative clock sanity gate, not a latency measurement. Codec
        // callbacks may be delayed; preserve raw times even if clocks disagree.
        session.render_clock_valid = device_ns > 0
            && device_ns <= callback_device_ns
            && callback_device_ns - device_ns <= 1_000_000_000;
        session.phase = "streaming";
        session.observe_settings();
        Ok(())
    }
    pub fn take_frame(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let mut state = self.shared.state.lock().unwrap();
        if state.session.as_ref().is_some_and(|s| {
            s.id == id
                && s.frames.is_empty()
                && matches!(s.phase, "connecting" | "streaming" | "configuring")
        }) {
            state = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap()
                .0;
        }
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id)
            .context("Display session replaced")?;
        ensure!(
            matches!(session.phase, "connecting" | "streaming" | "configuring"),
            "Display ended: {}",
            session.error
        );
        Ok(session.frames.pop_front().map(|queued| {
            session.decoder_progress.last_take = Some(Instant::now());
            let frame = queued.frame;
            session.bytes -= frame.bytes.len();
            session
                .decoder_budget
                .observe_progress(session.rendered_revision > 0, session.frames.len());
            let mut bytes = Vec::with_capacity(frame.bytes.len() + 17);
            bytes.extend(frame.timestamp.to_be_bytes());
            bytes.push(u8::from(frame.keyframe));
            bytes.extend(session.revision.to_be_bytes());
            bytes.extend(frame.bytes);
            bytes
        }))
    }
    pub fn cancel(&self, id: Option<&str>) {
        let mut state = self.shared.state.lock().unwrap();
        if id.is_some_and(|id| !state.session.as_ref().is_some_and(|s| s.id == id)) {
            return;
        }
        if let Some(s) = state.session.as_mut() {
            s.phase = "closed";
            s.frames.clear();
            s.cursor.clear();
            s.input.closed();
            s.settings.close();
            s.settings_gate.close(s.revision);
            s.bytes = 0;
        }
        if let Some(socket) = &state.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.shared.changed.notify_all();
    }
    pub fn take_cursor(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let mut state = self.shared.state.lock().unwrap();
        if state.session.as_ref().is_some_and(|s| {
            s.id == id
                && s.cursor.empty()
                && matches!(
                    s.phase,
                    "offered" | "connecting" | "streaming" | "configuring"
                )
        }) {
            state = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(8))
                .unwrap()
                .0;
        }
        let session = state
            .session
            .as_mut()
            .filter(|s| s.id == id)
            .context("Display session replaced")?;
        ensure!(
            matches!(
                session.phase,
                "offered" | "connecting" | "streaming" | "configuring"
            ),
            "Display ended: {}",
            session.error
        );
        Ok(session.cursor.take())
    }
    pub fn stop(&self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.running = false;
        }
        self.cancel(None);
        if self.port != 0 {
            let _ = TcpStream::connect_timeout(
                &([127, 0, 0, 1], self.port).into(),
                Duration::from_millis(100),
            );
        }
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
impl Session {
    fn observe_settings(&mut self) {
        self.settings_gate.observe(
            &self.settings.snapshot(),
            self.revision,
            &self.config,
            self.rendered_revision,
            self.phase == "streaming",
            Instant::now(),
        );
    }
}

fn finish_connection(shared: &Arc<Shared>, result: Result<()>) {
    let mut state = shared.state.lock().unwrap();
    state.socket = None;
    if let Some(session) = state.session.as_mut() {
        if session.phase != "closed" {
            session.phase = if result.is_ok() { "closed" } else { "failed" };
            session.error = result.err().map(|e| format!("{e:#}")).unwrap_or_default();
        }
        session.frames.clear();
        session.cursor.clear();
        session.input.closed();
        session.settings.close();
        session.settings_gate.close(session.revision);
        session.bytes = 0;
    }
    shared.changed.notify_all();
}
fn reconfigure(shared: &Arc<Shared>, peer: &str, mut config: Config) -> Result<Config> {
    config.validate()?;
    let mut state = shared.state.lock().unwrap();
    ensure!(state.running, "Receiving disabled");
    let session = state.session.as_mut().context("Display ended")?;
    ensure!(
        session.peer == peer && matches!(session.phase, "connecting" | "streaming"),
        "Display ended"
    );
    config = prepare_live_config(config, &session.config, &shared.capabilities)?;
    session.config = config.clone();
    session
        .input
        .suspend(extend_computer_agent::display::input::Reason::Changed);
    session.revision += 1;
    session.rendered_revision = 0;
    session.ready = false;
    session.phase = "configuring";
    session.frames.clear();
    session.decoder_budget = DecoderBudget::default();
    session.decoder_progress = DecoderProgress::default();
    session.bytes = 0;
    let revision = session.revision;
    shared.changed.notify_all();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let session = state.session.as_ref().context("Display ended")?;
        ensure!(
            state.running
                && session.peer == peer
                && session.revision == revision
                && session.phase != "closed"
                && session.phase != "failed",
            "Display ended"
        );
        if session.ready {
            return Ok(config);
        }
        ensure!(
            Instant::now() < deadline,
            "Hardware decoder configuration timed out"
        );
        state = shared
            .changed
            .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
            .unwrap()
            .0;
    }
}

fn prepare_live_config(
    mut config: Config,
    current: &Config,
    caps: &serde_json::Value,
) -> Result<Config> {
    config.validate()?;
    ensure!(
        config.mode == current.mode
            && config.codec == current.codec
            && config.separate_cursor == current.separate_cursor,
        "Unsupported live configuration"
    );
    // Resolve Fit before invalidating media/input. Never infer arbitrary profile
    // support from a larger decoder maximum or from the configured MediaFormat.
    if config.auto_size {
        select_auto_size(&mut config, caps)?;
    }
    let scale = if config.retina { 2 } else { 1 };
    let supported = caps[config.codec.name()]
        .as_array()
        .context("Hardware codec unavailable")?;
    ensure!(
        supported.iter().any(
            |size| size["width"].as_u64() == Some((config.width * scale) as u64)
                && size["height"].as_u64() == Some((config.height * scale) as u64)
                && size["fps"]
                    .as_u64()
                    .is_some_and(|fps| fps >= config.fps as u64)
        ),
        "Hardware decoder doesn't support this display size and frame rate"
    );
    Ok(config)
}
fn preflight(shared: &Arc<Shared>, peer: &str, config: &Config) -> Result<()> {
    let state = shared.state.lock().unwrap();
    ensure!(state.running, "Receiving disabled");
    let session = state.session.as_ref().context("Display ended")?;
    ensure!(
        session.peer == peer && matches!(session.phase, "connecting" | "streaming"),
        "Display ended"
    );
    prepare_live_config(config.clone(), &session.config, &shared.capabilities)?;
    Ok(())
}

/// Logical desktop size follows Android dp geometry; raster sharpness is chosen
/// only from exact sizes that Java tested with areSizeAndRateSupported.
fn select_auto_size(config: &mut Config, caps: &serde_json::Value) -> Result<()> {
    let sizes = caps[config.codec.name()]
        .as_array()
        .context("Hardware codec unavailable")?;
    let supported: Vec<(u32, u32)> = sizes
        .iter()
        .filter_map(|s| {
            let width = u32::try_from(s["width"].as_u64()?).ok()?;
            let height = u32::try_from(s["height"].as_u64()?).ok()?;
            (s["fps"].as_u64()? >= config.fps as u64
                && (640..=4096).contains(&width)
                && (480..=4096).contains(&height))
            .then_some((width, height))
        })
        .collect();
    let preferred = caps.get("preferred_logical");
    let (width, height, retina) = if let Some(preferred) = preferred {
        let width = u32::try_from(
            preferred["width"]
                .as_u64()
                .context("Missing logical width")?,
        )?;
        let height = u32::try_from(
            preferred["height"]
                .as_u64()
                .context("Missing logical height")?,
        )?;
        ensure!(
            (640..=4096).contains(&width) && (480..=4096).contains(&height),
            "Invalid preferred logical size"
        );
        if config.retina && supported.contains(&(width * 2, height * 2)) {
            (width, height, true)
        } else if supported.contains(&(width, height)) {
            (width, height, false)
        } else {
            let mut candidates = Vec::new();
            for &(raster_width, raster_height) in &supported {
                for scale in [1, 2] {
                    if scale == 2
                        && (!config.retina || raster_width % 2 != 0 || raster_height % 2 != 0)
                    {
                        continue;
                    }
                    let (w, h) = (raster_width / scale, raster_height / scale);
                    if w < 640 || h < 480 {
                        continue;
                    }
                    let aspect_error =
                        ((w as f64 / h as f64) / (width as f64 / height as f64) - 1.).abs();
                    if aspect_error > 0.02 {
                        continue;
                    }
                    let distance = (w as f64 / width as f64 - 1.).powi(2)
                        + (h as f64 / height as f64 - 1.).powi(2);
                    candidates.push((distance, w, h, scale == 2));
                }
            }
            let (_, w, h, retina) = candidates
                .into_iter()
                .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.3.cmp(&a.3)))
                .context("Hardware decoder doesn't support this display aspect and frame rate")?;
            (w, h, retina)
        }
    } else {
        let &(width, height) = supported
            .iter()
            .max_by_key(|(w, h)| *w as u64 * *h as u64)
            .context("Hardware decoder doesn't support this frame rate")?;
        (width, height, false)
    };
    config.width = width;
    config.height = height;
    config.retina = retina;
    config.validate()?;
    Ok(())
}

fn serve(
    socket: TcpStream,
    identity: &Identity,
    trust: &TrustStore,
    caps: &serde_json::Value,
    shared: &Arc<Shared>,
    usb: Option<crate::usb::Route>,
) -> Result<()> {
    socket.set_nonblocking(false)?;
    let ip = socket.local_addr()?.ip();
    let shutdown = socket.try_clone()?;
    let mut media = None;
    let id = IDS.fetch_add(1, Ordering::Relaxed).to_string();
    let answer =
        |peer: &str,
         config: &mut Config,
         offer: &str,
         features: extend_computer_agent::session::display::DisplayFeatures| {
            ensure!(
                usb.as_ref()
                    .is_none_or(|route| route.current() && route.peer == peer),
                "USB connection changed"
            );
            if config.auto_size {
                select_auto_size(config, caps)?;
            }
            config.validate()?;
            let (mut transport, route) = match &usb {
                Some(route) => (route.transport(config.codec)?, "usb"),
                None => (
                    extend_computer_agent::display::Transport::with_codec(ip, config.codec)?,
                    "lan",
                ),
            };
            let answer = transport.answer_with_features(offer, features)?;
            let mut state = shared.state.lock().unwrap();
            ensure!(state.running, "Receiving disabled");
            state.session = Some(Session {
                id: id.clone(),
                peer: peer.into(),
                route,
                config: config.clone(),
                phase: "offered",
                ready: false,
                revision: 0,
                rendered_revision: 0,
                error: String::new(),
                frames: VecDeque::new(),
                decoder_budget: DecoderBudget::default(),
                decoder_progress: DecoderProgress::default(),
                cursor: CursorMailbox::default(),
                input: ReceiverInput::with_capabilities(
                    features.input_enabled(),
                    features.text_enabled(),
                    features.shortcuts_enabled(),
                ),
                settings: Arc::new(ReceiverSettings::new(false, config.clone())),
                settings_gate: ApplyGate::default(),
                bytes: 0,
                received: 0,
                rendered: 0,
                received_timestamp: 0,
                rendered_timestamp: 0,
                render_device_ns: 0,
                callback_device_ns: 0,
                render_clock_valid: false,
            });
            shared.changed.notify_all();
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                let session = state.session.as_ref().context("Display canceled")?;
                ensure!(
                    state.running && session.phase != "closed",
                    "Display canceled"
                );
                if session.ready {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "Open extend.computer to receive this display"
                );
                state = shared
                    .changed
                    .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                    .unwrap()
                    .0;
            }
            media = Some(transport);
            Ok(answer)
        };
    let (mut signal, peer, features) = match &usb {
        Some(route) => DisplaySignal::accept_with_settings_features_pinned(
            socket,
            identity,
            trust,
            &route.peer,
            true,
            answer,
        )?,
        None => {
            DisplaySignal::accept_with_settings_features(socket, identity, trust, true, answer)?
        }
    };
    let settings = {
        let mut state = shared.state.lock().unwrap();
        let session = state
            .session
            .as_mut()
            .filter(|session| session.id == id && session.phase != "closed")
            .context("Display canceled")?;
        session.settings = Arc::new(ReceiverSettings::new(
            signal.receiver_settings_supported(),
            session.config.clone(),
        ));
        session.settings.clone()
    };
    let ended = Arc::new(AtomicBool::new(false));
    let monitor = ended.clone();
    let shared_signal = shared.clone();
    let signal_trust = trust.clone();
    let remote = peer.clone();
    let heartbeat_id = id.clone();
    let heartbeat = thread::spawn(move || {
        while !monitor.load(Ordering::Acquire)
            && shared_signal.state.lock().unwrap().running
            && signal_trust.peer(&remote).ok().flatten().is_some()
        {
            match signal.respond_with_settings(
                &settings,
                |config| preflight(&shared_signal, &remote, config),
                |config| reconfigure(&shared_signal, &remote, config),
            ) {
                Ok(true) => {
                    // Record the source barrier as soon as its sole channel reader
                    // receives it. A subsequent validated render completes the UI.
                    let mut state = shared_signal.state.lock().unwrap();
                    if let Some(session) = state
                        .session
                        .as_mut()
                        .filter(|session| session.id == heartbeat_id)
                    {
                        session.observe_settings();
                    }
                }
                _ => break,
            }
        }
        monitor.store(true, Ordering::Release);
    });
    let result = (|| -> Result<()> {
        let mut transport = media.context("Missing media transport")?;
        let mut checked = Instant::now();
        let mut keyframe = false;
        let mut revision = 0;
        while !ended.load(Ordering::Acquire) {
            ensure!(
                usb.as_ref().is_none_or(|route| route.current()),
                "USB connection changed"
            );
            {
                let mut state = shared.state.lock().unwrap();
                if !state.running
                    || state
                        .session
                        .as_ref()
                        .is_none_or(|s| s.id != id || s.phase == "closed")
                {
                    break;
                }
                if features.input_enabled()
                    && state.session.as_ref().is_some_and(|s| s.input.needs_poll())
                {
                    let session = state.session.as_mut().context("Display ended")?;
                    ensure!(trust.peer(&peer)?.is_some(), "Device was forgotten");
                    let rendered = session.phase == "streaming" && session.rendered_revision > 0;
                    // Input failure revokes the native lease; decoding keeps running.
                    if session.input.flush(&mut transport, rendered).is_err() {
                        session.input.closed();
                    }
                }
            }
            if checked.elapsed() >= Duration::from_millis(500) {
                ensure!(trust.peer(&peer)?.is_some(), "Device was forgotten");
                checked = Instant::now();
            }
            match transport.poll()? {
                Some(DisplayEvent::Frame(frame)) => {
                    if !keyframe && !frame.keyframe {
                        transport.request_keyframe()?;
                        continue;
                    }
                    keyframe = true;
                    ensure!(
                        frame.bytes.len() <= 8 * 1024 * 1024,
                        "Encoded frame exceeds decoder limit"
                    );
                    let mut state = shared.state.lock().unwrap();
                    let session = state.session.as_mut().context("Display ended")?;
                    if session.phase == "configuring" {
                        continue;
                    }
                    if revision != session.revision {
                        revision = session.revision;
                        keyframe = false;
                        transport.request_keyframe()?;
                        if !frame.keyframe {
                            continue;
                        }
                        keyframe = true;
                    }
                    // Never discard encoded reference frames. Fail boundedly if the decoder stops consuming.
                    let arrived = Instant::now();
                    session
                        .decoder_budget
                        .admit(
                            arrived,
                            session.frames.len(),
                            session.bytes,
                            frame.bytes.len(),
                            session.rendered_revision > 0,
                        )
                        .map_err(|error| {
                            anyhow::anyhow!(
                                "{error}; decoder_progress={}",
                                session.decoder_progress.diagnostic(
                                    arrived,
                                    &session.frames,
                                    frame.timestamp,
                                    session.received,
                                    session.rendered,
                                    session.revision
                                )
                            )
                        })?;
                    session.received += 1;
                    session.received_timestamp = frame.timestamp;
                    session.bytes += frame.bytes.len();
                    session.frames.push_back(QueuedFrame { frame, arrived });
                    shared.changed.notify_all();
                }
                Some(DisplayEvent::Keyframe) => {
                    keyframe = false;
                    transport.request_keyframe()?;
                }
                Some(DisplayEvent::Cursor(bytes)) => {
                    let mut state = shared.state.lock().unwrap();
                    let session = state.session.as_mut().context("Display ended")?;
                    if session.id == id
                        && session.config.separate_cursor
                        && matches!(session.phase, "connecting" | "streaming" | "configuring")
                    {
                        session.cursor.push(bytes)?;
                        shared.changed.notify_all();
                    }
                }
                Some(DisplayEvent::Input(message)) => {
                    let mut state = shared.state.lock().unwrap();
                    let session = state.session.as_mut().context("Display ended")?;
                    if session.id == id {
                        session
                            .input
                            .receive(message, &session.config, session.revision)?;
                        shared.changed.notify_all();
                    }
                }
                Some(DisplayEvent::InputClosed) => {
                    let mut state = shared.state.lock().unwrap();
                    if let Some(session) = state.session.as_mut().filter(|s| s.id == id) {
                        session.input.closed();
                        shared.changed.notify_all();
                    }
                }
                Some(_) => {}
                None => thread::sleep(Duration::from_millis(1)),
            }
        }
        Ok(())
    })();
    ended.store(true, Ordering::Release);
    let _ = shutdown.shutdown(Shutdown::Both);
    let _ = heartbeat.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::session::display::{Mode, VideoCodec};
    #[test]
    fn live_profile_preflight_requires_exact_probed_raster_and_rate() {
        let config = Config {
            mode: Mode::Extend,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 8_000_000,
            retina: false,
            auto_size: false,
            codec: VideoCodec::H264,
            separate_cursor: true,
        };
        let caps = serde_json::json!({"h264":[{"width":1280,"height":800,"fps":60},{"width":2560,"height":1600,"fps":30}]});
        assert!(prepare_live_config(config.clone(), &config, &caps).is_ok());
        let mut next = config.clone();
        next.width = 1920;
        next.height = 1200;
        assert!(prepare_live_config(next, &config, &caps).is_err());
        let mut next = config.clone();
        next.retina = true;
        assert!(prepare_live_config(next.clone(), &config, &caps).is_err());
        next.fps = 30;
        assert!(prepare_live_config(next, &config, &caps).is_ok());
        let mut next = config.clone();
        next.mode = Mode::Mirror;
        assert!(prepare_live_config(next, &config, &caps).is_err());
        assert_eq!(config.width, 1280); // Preflight owns a clone, never the active decoder or lease.
    }
    #[test]
    fn receiver_settings_pinned_source_barrier_then_fresh_render_and_scoped_cancel() {
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let remote = Identity::generate();
        let trust = TrustStore::open(files.path()).unwrap();
        trust.remember(&remote.fingerprint()).unwrap();
        let server=Arc::new(Server::bind(identity.clone(),trust,
            serde_json::json!({"h264":[{"width":1280,"height":800,"fps":60},{"width":960,"height":600,"fps":30}]}),"127.0.0.1:0").unwrap());
        let addr: std::net::SocketAddr = ([127, 0, 0, 1], server.port).into();
        let pin = identity.fingerprint();
        let (advance, advances) = std::sync::mpsc::channel();
        let (barrier, barriers) = std::sync::mpsc::channel();
        let client = thread::spawn(move || {
            let config = Config {
                mode: Mode::Extend,
                width: 1280,
                height: 800,
                fps: 60,
                bitrate: 8_000_000,
                retina: false,
                auto_size: false,
                codec: VideoCodec::H264,
                separate_cursor: true,
            };
            let mut transport = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
            let (mut signal, _, answer, _) = DisplaySignal::connect_with_settings_offer(
                TcpStream::connect(addr).unwrap(),
                &remote,
                &pin,
                &config,
                true,
                |features| transport.offer_with_features(features),
            )
            .unwrap();
            assert!(signal.receiver_settings_supported());
            transport.accept_answer(&answer).unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                if matches!(transport.poll().unwrap(), Some(DisplayEvent::Connected)) {
                    break;
                }
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
            let frame = |timestamp| Frame {
                timestamp,
                keyframe: true,
                bytes: vec![0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21],
            };
            transport.write(frame(1_000_000_000)).unwrap();
            advances.recv_timeout(Duration::from_secs(8)).unwrap();
            signal
                .pulse_with_settings(1, |signal, config| {
                    let applied = signal.configure(&config)?;
                    barrier.send(()).unwrap();
                    // The real Mac helper barrier occupies this stage; keep it
                    // blocked to prove an early receiver render cannot finish Apply.
                    advances.recv_timeout(Duration::from_secs(8)).unwrap();
                    Ok(applied)
                })
                .unwrap();
            barrier.send(()).unwrap();
            transport.write(frame(2_000_000_000)).unwrap();
            advances.recv_timeout(Duration::from_secs(8)).unwrap();
            signal.close();
        });
        let deadline = Instant::now() + Duration::from_secs(8);
        while server.snapshot()["state"] != "offered" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let id = server.snapshot()["id"].as_str().unwrap().to_owned();
        server.attach(&id).unwrap();
        while server.take_frame(&id).unwrap().is_none() {
            assert!(Instant::now() < deadline);
        }
        server.rendered(&id, 0, 1_000_000_000, 1, 1).unwrap();
        assert_eq!(server.receiver_settings(&id).unwrap()["supported"], true);
        let request = r#"{"width":960,"height":600,"auto_size":false,"fps":30,"bitrate":4000000}"#;
        assert!(server
            .request_receiver_settings("retired", 0, request)
            .is_err());
        assert!(server.request_receiver_settings(&id, 1, request).is_err());
        let unsupported = request.replace("960", "1920").replace("600", "1200");
        assert!(server
            .request_receiver_settings(&id, 0, &unsupported)
            .is_err());
        assert_eq!(server.snapshot()["revision"], 0);
        let request_id = server.request_receiver_settings(&id, 0, request).unwrap();
        assert!(server.request_receiver_settings(&id, 0, request).is_err());
        advance.send(()).unwrap();
        while server.snapshot()["state"] != "configuring" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        server.configured(&id, 1).unwrap();
        barriers.recv_timeout(Duration::from_secs(2)).unwrap();
        // Synthetic Java callback: geometry/PTS permission is tested separately
        // by DecodedFrameGateTest. This regression tests the native phase gate.
        server.rendered(&id, 1, 1_500_000_000, 2, 2).unwrap();
        assert_eq!(
            server.receiver_settings(&id).unwrap()["pending"],
            request_id
        );
        assert_eq!(
            server.receiver_settings(&id).unwrap()["applied"]["revision"],
            0
        );
        advance.send(()).unwrap();
        barriers.recv_timeout(Duration::from_secs(2)).unwrap();
        while server
            .shared
            .state
            .lock()
            .unwrap()
            .session
            .as_ref()
            .unwrap()
            .settings
            .snapshot()
            .result
            .is_none()
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let status = server.receiver_settings(&id).unwrap();
        assert_eq!(status["pending"], request_id);
        assert!(server.request_receiver_settings(&id, 1, request).is_err());
        while server.take_frame(&id).unwrap().is_none() {
            assert!(Instant::now() < deadline);
        }
        server.rendered(&id, 0, 2_000_000_000, 3, 3).unwrap();
        assert_eq!(
            server.receiver_settings(&id).unwrap()["pending"],
            request_id
        );
        server.rendered(&id, 1, 2_000_000_000, 3, 3).unwrap();
        let status = server.receiver_settings(&id).unwrap();
        assert!(status["pending"].is_null());
        assert_eq!(status["result"]["outcome"], "applied");
        assert_eq!(status["applied"]["config"]["width"], 960);
        server.cancel(Some("retired"));
        assert_eq!(server.snapshot()["state"], "streaming");
        server.cancel(Some(&id));
        assert_eq!(server.receiver_settings(&id).unwrap()["closed"], true);
        assert!(server.request_receiver_settings(&id, 1, request).is_err());
        advance.send(()).unwrap();
        client.join().unwrap();
        server.stop();
    }
    #[test]
    fn auto_size_prefers_density_geometry_and_only_exact_probed_rasters() {
        let mut config = Config {
            mode: Mode::Extend,
            width: 1920,
            height: 1200,
            fps: 60,
            retina: true,
            auto_size: true,
            codec: VideoCodec::H264,
            bitrate: 8_000_000,
            separate_cursor: true,
        };
        let mut caps = serde_json::json!({"preferred_logical":{"width":1138,"height":710},
            "h264":[{"width":1920,"height":1200,"fps":60},{"width":1138,"height":710,"fps":60}]});
        select_auto_size(&mut config, &caps).unwrap();
        assert_eq!(
            (config.width, config.height, config.retina),
            (1138, 710, false)
        );
        // A larger decoder limit cannot imply support for arbitrary 2x sizes.
        caps["h264"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"width":2560,"height":1600,"fps":60}));
        config.retina = true;
        select_auto_size(&mut config, &caps).unwrap();
        assert_eq!(
            (config.width, config.height, config.retina),
            (1138, 710, false)
        );
        caps["h264"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"width":2276,"height":1420,"fps":60}));
        config.retina = true;
        select_auto_size(&mut config, &caps).unwrap();
        assert_eq!(
            (config.width, config.height, config.retina),
            (1138, 710, true)
        );
        config.retina = false;
        select_auto_size(&mut config, &caps).unwrap();
        assert_eq!(
            (config.width, config.height, config.retina),
            (1138, 710, false)
        );
        // If exact dp dimensions fail the hardware check, choose nearby geometry.
        caps["h264"] = serde_json::json!([{"width":1920,"height":1200,"fps":120},
            {"width":1280,"height":800,"fps":60},{"width":1138,"height":710,"fps":30}]);
        config.retina = true;
        select_auto_size(&mut config, &caps).unwrap();
        assert_eq!(
            (config.width, config.height, config.retina),
            (1280, 800, false)
        );
        // Legacy internal capabilities preserve their existing maximum-raster behavior.
        caps.as_object_mut().unwrap().remove("preferred_logical");
        select_auto_size(&mut config, &caps).unwrap();
        assert_eq!(
            (config.width, config.height, config.retina),
            (1920, 1200, false)
        );
        caps["preferred_logical"] = serde_json::json!({"width":1138,"height":710});
        caps["h264"] = serde_json::json!([{"width":1920,"height":1080,"fps":60}]);
        assert!(select_auto_size(&mut config, &caps).is_err());
        caps["preferred_logical"]["width"] = serde_json::json!(u64::MAX);
        assert!(select_auto_size(&mut config, &caps).is_err());
    }

    #[test]
    fn decoder_budget_ignores_transport_preroll_and_latches_after_render_and_drain() {
        let transport_started = Instant::now();
        let first_media = transport_started + Duration::from_secs(5);
        let mut budget = DecoderBudget::default();
        assert_eq!(budget.limit(first_media), DECODER_STARTUP_FRAMES);
        for queued in 0..24 {
            budget
                .admit(first_media, queued, queued * 100, 100, false)
                .unwrap();
        }
        // Starting the codec and its output does not prove the startup backlog
        // has drained. Transitioning here would reject healthy reference frames.
        budget.observe_progress(true, 24);
        assert_eq!(budget.limit(first_media), DECODER_STARTUP_FRAMES);
        budget.observe_progress(false, 0);
        assert!(!budget.steady);
        budget.observe_progress(true, 16);
        assert!(!budget.steady);
        budget.observe_progress(true, 15);
        assert_eq!(budget.limit(first_media), DECODER_STREAMING_FRAMES);
        budget.admit(first_media, 15, 1500, 100, true).unwrap();
        assert!(budget.admit(first_media, 16, 1600, 100, true).is_err());
        budget.observe_progress(false, 20);
        assert_eq!(budget.limit(first_media), DECODER_STREAMING_FRAMES);
    }

    #[test]
    fn decoder_startup_is_bounded_by_first_media_age_frames_and_bytes() {
        let now = Instant::now();
        let mut budget = DecoderBudget::default();
        budget.admit(now, 0, 0, 100, false).unwrap();
        budget
            .admit(now + Duration::from_millis(999), 63, 6300, 100, false)
            .unwrap();
        assert!(budget.admit(now, 64, 6400, 100, false).is_err());
        budget
            .admit(now, 1, DECODER_QUEUE_BYTES - 100, 100, false)
            .unwrap();
        assert!(budget.admit(now, 1, DECODER_QUEUE_BYTES, 1, false).is_err());
        assert!(budget.admit(now, 1, usize::MAX, 1, false).is_err());
        let overflow = budget
            .admit(now + DECODER_STARTUP, 20, 2000, 100, false)
            .unwrap_err()
            .to_string();
        assert!(overflow.contains("queued_frames=20"));
        assert!(overflow.contains("frame_limit=16"));
        assert!(overflow.contains("startup_ms=1000"));
        budget
            .admit(now + DECODER_STARTUP, 15, 1500, 100, false)
            .unwrap();
    }

    #[test]
    fn terminal_decoder_diagnostic_distinguishes_arrival_burst_from_feeder_stall() {
        let now = Instant::now();
        // Deliberately unrelated source clock; it must never become a local age.
        let first_pts = 9_000_000_000_000u64;
        let queue = |age_ms| {
            (0..16)
                .map(|index| QueuedFrame {
                    frame: Frame {
                        timestamp: first_pts + index * 16_666_667,
                        keyframe: index == 0,
                        bytes: vec![1; 100],
                    },
                    arrived: now - Duration::from_millis(age_ms),
                })
                .collect::<VecDeque<_>>()
        };
        let recent = DecoderProgress {
            last_take: Some(now - Duration::from_millis(1)),
            last_render_ack: Some(now - Duration::from_millis(16)),
        };
        let burst = recent.diagnostic(now, &queue(2), first_pts + 16 * 16_666_667, 140, 120, 3);
        assert_eq!(burst["age_clock"], "rust-instant");
        assert_eq!(burst["oldest_arrival_age_ms"], 2);
        assert_eq!(burst["last_take_age_ms"], 1);
        assert_eq!(burst["last_render_ack_age_ms"], 16);
        assert_eq!(burst["queued_pts_span_ns"], 250_000_005u64);
        assert_eq!(burst["incoming_after_oldest_pts_ns"], 266_666_672u64);
        assert_eq!(burst["received"], 140);
        assert_eq!(burst["rendered"], 120);
        assert_eq!(burst["revision"], 3);
        let stalled = DecoderProgress {
            last_take: Some(now - Duration::from_millis(350)),
            last_render_ack: Some(now - Duration::from_millis(400)),
        }
        .diagnostic(now, &queue(300), first_pts + 16 * 16_666_667, 140, 120, 3);
        assert_eq!(stalled["oldest_arrival_age_ms"], 300);
        assert_eq!(stalled["last_take_age_ms"], 350);
        assert_eq!(stalled["last_render_ack_age_ms"], 400);
        // Diagnostic recency does not change the existing fatal admission rule.
        for _ in [&burst, &stalled] {
            let mut budget = DecoderBudget::default();
            budget.admit(now, 0, 0, 100, false).unwrap();
            budget.observe_progress(true, 0);
            assert!(budget.admit(now, 16, 1600, 100, true).is_err());
        }
    }

    #[test]
    fn terminal_decoder_diagnostic_unknown_and_reversed_pts_do_not_wrap() {
        let now = Instant::now();
        let progress = DecoderProgress::default();
        let empty = progress.diagnostic(now, &VecDeque::new(), 7, 0, 0, 0);
        for key in [
            "last_take_age_ms",
            "last_render_ack_age_ms",
            "oldest_arrival_age_ms",
            "queued_pts_span_ns",
            "incoming_after_oldest_pts_ns",
        ] {
            assert!(empty[key].is_null(), "{key} invented progress");
        }
        let frames = VecDeque::from([
            QueuedFrame {
                frame: Frame {
                    timestamp: 10,
                    keyframe: true,
                    bytes: vec![],
                },
                arrived: now,
            },
            QueuedFrame {
                frame: Frame {
                    timestamp: 5,
                    keyframe: false,
                    bytes: vec![],
                },
                arrived: now,
            },
        ]);
        let reversed = progress.diagnostic(now, &frames, 2, 2, 0, 1);
        assert!(reversed["queued_pts_span_ns"].is_null());
        assert!(reversed["incoming_after_oldest_pts_ns"].is_null());
        assert_eq!(reversed["queued_first_pts_ns"], 10);
        assert_eq!(reversed["queued_last_pts_ns"], 5);
        assert_eq!(reversed["incoming_pts_ns"], 2);
    }

    #[test]
    fn cursor_mailbox_bounds_flood_preserves_shape_and_rejects_stale_state() {
        let mut mailbox = CursorMailbox::default();
        let shape =
            br#"{"revision":2,"png":"AA==","width":20,"height":30,"hotspot_x":3,"hotspot_y":5}"#
                .to_vec();
        mailbox.push(shape.clone()).unwrap();
        for sequence in 1..=10_000 {
            mailbox
                .push(
                    serde_json::to_vec(&serde_json::json!({
                        "revision":2,"sequence":sequence,"x":0.25,"y":0.75,"visible":true
                    }))
                    .unwrap(),
                )
                .unwrap();
        }
        assert_eq!(mailbox.received, 10_001);
        assert_eq!(mailbox.take().unwrap(), shape);
        let latest: serde_json::Value = serde_json::from_slice(&mailbox.take().unwrap()).unwrap();
        assert_eq!(latest["sequence"], 10_000);
        assert!(mailbox.empty());
        mailbox
            .push(br#"{"revision":2,"sequence":9999,"x":0,"y":0,"visible":true}"#.to_vec())
            .unwrap();
        mailbox.push(br#"{"revision":1,"png":"AA==","width":20,"height":30,"hotspot_x":3,"hotspot_y":5}"#.to_vec()).unwrap();
        assert!(mailbox.take().is_none());
        assert_eq!(mailbox.delivered, 2);
        assert!(mailbox
            .push(br#"{"revision":2,"sequence":10001,"x":1.1,"y":0,"visible":true}"#.to_vec())
            .is_err());
        assert!(mailbox.empty());
        mailbox
            .push(br#"{"revision":2,"sequence":10001,"x":0,"y":0,"visible":false}"#.to_vec())
            .unwrap();
        mailbox.clear();
        assert!(mailbox.take().is_none());
    }
    #[test]
    fn negotiated_display_input_uses_current_rendered_session_and_releases_on_disable() {
        use extend_computer_agent::display::input::{
            Binding, Command, Geometry, Message, Rect, Size,
        };
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let remote = Identity::generate();
        let trust = TrustStore::open(files.path()).unwrap();
        trust.remember(&remote.fingerprint()).unwrap();
        let server = Arc::new(
            Server::bind(
                identity.clone(),
                trust,
                serde_json::json!({"h264":[{"width":1280,"height":800,"fps":60}]}),
                "127.0.0.1:0",
            )
            .unwrap(),
        );
        let addr: std::net::SocketAddr = ([127, 0, 0, 1], server.port).into();
        let pin = identity.fingerprint();
        let (sent, received) = std::sync::mpsc::channel();
        let client = thread::spawn(move || {
            let config = Config {
                mode: Mode::Extend,
                width: 1280,
                height: 800,
                fps: 60,
                bitrate: 8_000_000,
                retina: false,
                auto_size: false,
                separate_cursor: true,
                codec: VideoCodec::H264,
            };
            let mut transport = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
            let (mut signal, _, answer, features) = DisplaySignal::connect_with_offer(
                TcpStream::connect(addr).unwrap(),
                &remote,
                &pin,
                &config,
                |features| transport.offer_with_features(features),
            )
            .unwrap();
            assert!(features.input_enabled());
            transport.accept_answer(&answer).unwrap();
            let binding = Binding::new(1, 0);
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut frame_sent = false;
            let mut stopped = false;
            while Instant::now() < deadline {
                match transport.poll().unwrap() {
                    Some(DisplayEvent::Connected) if !frame_sent => {
                        transport
                            .write(Frame {
                                timestamp: 1_000_000_000,
                                keyframe: true,
                                bytes: vec![0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21],
                            })
                            .unwrap();
                        frame_sent = true;
                    }
                    Some(DisplayEvent::Input(Message::Request { request })) => {
                        let size = Size {
                            width: 1280,
                            height: 800,
                        };
                        assert!(transport
                            .write_input(&Message::Granted {
                                text_supported: false,
                                shortcut_policy: None,
                                request,
                                binding: binding.clone(),
                                geometry: Geometry {
                                    content: Rect {
                                        x: 0.0,
                                        y: 0.0,
                                        width: 1.0,
                                        height: 1.0
                                    },
                                    canvas: size,
                                    raster: size,
                                    source_size: size
                                }
                            })
                            .unwrap());
                    }
                    Some(DisplayEvent::Input(message @ Message::Command { .. })) => {
                        let Message::Command {
                            binding: active,
                            sequence,
                            command,
                        } = &message
                        else {
                            unreachable!()
                        };
                        assert_eq!(*active, binding);
                        assert!(transport
                            .write_input(&Message::Applied {
                                binding: active.clone(),
                                sequence: *sequence
                            })
                            .unwrap());
                        stopped = matches!(command, Command::Stop);
                        sent.send(message).unwrap();
                        if stopped {
                            break;
                        }
                    }
                    _ => {}
                }
                thread::sleep(Duration::from_millis(1));
            }
            assert!(stopped, "Native receiver did not release input lease");
            signal.close();
        });
        let deadline = Instant::now() + Duration::from_secs(8);
        while server.snapshot()["state"] != "offered" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let id = server.snapshot()["id"].as_str().unwrap().to_owned();
        server.attach(&id).unwrap();
        while server.take_frame(&id).unwrap().is_none() {
            assert!(Instant::now() < deadline);
        }
        server.rendered(&id, 0, 1_000_000_000, 1, 1).unwrap();
        assert_eq!(server.receiver_settings(&id).unwrap()["supported"], false);
        assert!(server
            .request_receiver_settings(
                &id,
                0,
                r#"{"width":1280,"height":800,"auto_size":false,"fps":60,"bitrate":8000000}"#
            )
            .is_err());
        assert!(server.set_input_enabled("stale-id", true).is_err());
        server.set_input_enabled(&id, true).unwrap();
        while server.input_state(&id).unwrap()["state"] != "granted" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        // A failed automatic resize must not revoke a live native input lease.
        let before = server.snapshot();
        let grant = server.input_state(&id).unwrap();
        let mut unsupported: Config = serde_json::from_value(before["config"].clone()).unwrap();
        unsupported.auto_size = true;
        unsupported.fps = 120;
        assert!(reconfigure(
            &server.shared,
            before["peer"].as_str().unwrap(),
            unsupported
        )
        .is_err());
        assert_eq!(server.snapshot()["revision"], before["revision"]);
        assert_eq!(server.snapshot()["config"], before["config"]);
        assert_eq!(server.input_state(&id).unwrap(), grant);
        let command = r#"{"kind":"move","point":{"x":0.25,"y":0.75}}"#;
        assert!(server.input_command(&id, 1, command).is_err());
        server.input_command(&id, 0, command).unwrap();
        let movement = received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            movement,
            Message::Command {
                command: Command::Move { .. },
                ..
            }
        ));
        while server.input_state(&id).unwrap()["applied_sequence"]
            .as_u64()
            .unwrap()
            == 0
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        server.set_input_enabled(&id, false).unwrap();
        let release = received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            release,
            Message::Command {
                command: Command::Stop,
                ..
            }
        ));
        assert_eq!(server.input_state(&id).unwrap()["state"], "disabled");
        client.join().unwrap();
        server.stop();
    }
    #[test]
    fn pinned_offer_delayed_first_media_burst_preserves_frames_and_cancels() {
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let remote = Identity::generate();
        let trust = TrustStore::open(files.path()).unwrap();
        trust.remember(&remote.fingerprint()).unwrap();
        let caps = serde_json::json!({"h264":[{"width":960,"height":600,"fps":120},{"width":1280,"height":800,"fps":60}]});
        let server = Arc::new(Server::bind(identity.clone(), trust, caps, "127.0.0.1:0").unwrap());
        let addr: std::net::SocketAddr = ([127, 0, 0, 1], server.port).into();
        let config = Config {
            mode: Mode::Extend,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 8_000_000,
            retina: false,
            auto_size: true,
            codec: VideoCodec::H264,
            separate_cursor: true,
        };
        let mut unknown_transport = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
        let unknown_offer = unknown_transport.offer().unwrap();
        assert!(DisplaySignal::connect(
            TcpStream::connect(addr).unwrap(),
            &Identity::generate(),
            &identity.fingerprint(),
            &config,
            &unknown_offer
        )
        .is_err());
        assert_eq!(server.snapshot()["state"], "idle");
        let (sent, received) = std::sync::mpsc::channel();
        let remote_peer = remote.fingerprint();
        let pin = identity.fingerprint();
        let client = thread::spawn(move || {
            let mut media = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
            let offer = media.offer().unwrap();
            let (mut signal, selected, answer) = DisplaySignal::connect(
                TcpStream::connect(addr).unwrap(),
                &remote,
                &pin,
                &config,
                &offer,
            )
            .unwrap();
            assert!(selected.separate_cursor);
            assert_eq!(selected.width, 1280);
            media.accept_answer(&answer).unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut frame_sent = false;
            let mut connected = false;
            let mut shape_sent = false;
            let mut position_sent = false;
            let mut cursor_consumed = false;
            let mut first_frame_delay = None;
            while Instant::now() < deadline {
                if let Some(DisplayEvent::Connected) = media.poll().unwrap() {
                    connected = true;
                }
                if connected {
                    if !shape_sent {
                        shape_sent = media.write_cursor(br#"{"revision":1,"png":"AA==","width":20,"height":30,"hotspot_x":3,"hotspot_y":5}"#).unwrap();
                    }
                    if !position_sent {
                        position_sent = media
                            .write_cursor(
                                br#"{"revision":1,"sequence":1,"x":0.25,"y":0.75,"visible":true}"#,
                            )
                            .unwrap();
                    }
                }
                if !cursor_consumed && shape_sent && position_sent && received.try_recv().is_ok() {
                    cursor_consumed = true;
                    first_frame_delay = Some(Instant::now());
                }
                if cursor_consumed
                    && !frame_sent
                    && first_frame_delay
                        .is_some_and(|since| since.elapsed() >= Duration::from_millis(1100))
                {
                    // Delay capture beyond the old transport-age grace, then
                    // deliver more than its streaming limit while the decoder
                    // consumer is starting. Every reference frame must survive.
                    for index in 0..24 {
                        assert!(media
                            .write(Frame {
                                timestamp: 1_000_000_000 + index * 16_666_667,
                                keyframe: index == 0,
                                bytes: vec![
                                    0,
                                    0,
                                    0,
                                    1,
                                    if index == 0 { 0x65 } else { 0x41 },
                                    0x88,
                                    0x84,
                                    0x21
                                ],
                            })
                            .unwrap());
                    }
                    frame_sent = true;
                }
                if frame_sent && received.try_recv().is_ok() {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            assert!(frame_sent, "Encrypted media did not connect");
            signal.close();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.snapshot()["state"] != "offered" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let snapshot = server.snapshot();
        assert_eq!(snapshot["peer"], remote_peer);
        assert_eq!(snapshot["received"], 0);
        let id = snapshot["id"].as_str().unwrap();
        assert!(server.attach("stale-id").is_err());
        server.attach(id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut got_shape = false;
        let mut got_position = false;
        while !got_shape || !got_position {
            if let Some(bytes) = server.take_cursor(id).unwrap() {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if value.get("png").is_some() {
                    got_shape = true;
                } else {
                    got_position = true;
                    assert_eq!(value["sequence"], 1);
                }
            }
            assert!(
                Instant::now() < deadline,
                "Cursor did not arrive independently of video"
            );
        }
        assert_eq!(server.snapshot()["received"], 0);
        assert!(server.take_cursor("stale-id").is_err());
        sent.send(()).unwrap();
        while server.snapshot()["received"] != 24 {
            assert_ne!(
                server.snapshot()["state"],
                "failed",
                "{}",
                server.snapshot()
            );
            assert!(Instant::now() < deadline, "Startup burst did not arrive");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(server.snapshot()["queued_frames"], 24);
        assert_eq!(server.snapshot()["decoder_queue_limit"], 64);
        let frame = loop {
            if let Some(frame) = server.take_frame(id).unwrap() {
                break frame;
            }
            assert!(Instant::now() < deadline, "No encrypted frame received");
        };
        assert_eq!(
            u64::from_be_bytes(frame[..8].try_into().unwrap()),
            1_000_000_000
        );
        assert_eq!(frame[8], 1);
        assert_eq!(&frame[17..], &[0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21]);
        server
            .rendered(id, 0, 1_000_000_000, 1_000_000_000, 2_100_000_000)
            .unwrap();
        assert_eq!(server.snapshot()["render_clock_valid"], false);
        server
            .rendered(id, 0, 1_000_000_000, 2_000_000_000, 2_010_000_000)
            .unwrap();
        assert_eq!(server.snapshot()["render_device_ns"], 2_000_000_000u64);
        assert_eq!(server.snapshot()["callback_device_ns"], 2_010_000_000u64);
        assert_eq!(server.snapshot()["render_clock_valid"], true);
        assert_eq!(server.snapshot()["decoder_queue_steady"], false);
        for index in 1..24 {
            let frame = server.take_frame(id).unwrap().unwrap();
            let timestamp = u64::from_be_bytes(frame[..8].try_into().unwrap());
            assert!(timestamp.abs_diff(1_000_000_000 + index * 16_666_667) < 20_000);
            assert_eq!(frame[8], 0);
            assert_eq!(&frame[17..], &[0, 0, 0, 1, 0x41, 0x88, 0x84, 0x21]);
        }
        assert_eq!(server.snapshot()["queued_frames"], 0);
        assert_eq!(server.snapshot()["queued_bytes"], 0);
        assert_eq!(server.snapshot()["decoder_queue_steady"], true);
        assert_eq!(server.snapshot()["decoder_queue_limit"], 16);
        assert_eq!(server.snapshot()["state"], "streaming");
        sent.send(()).unwrap();
        client.join().unwrap();
        server.cancel(Some(id));
        server.stop();
        assert!(TcpStream::connect(addr).is_err());
        assert!(server.take_frame(id).is_err());
        assert!(server.take_cursor(id).is_err());
    }
    #[test]
    fn android_sender_pins_media_delivers_frame_and_closes_on_unpair() {
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let remote = Arc::new(Identity::generate());
        let trust = TrustStore::open(files.path().join("sender")).unwrap();
        let remote_trust = TrustStore::open(files.path().join("receiver")).unwrap();
        trust.remember(&remote.fingerprint()).unwrap();
        remote_trust.remember(&identity.fingerprint()).unwrap();
        let server = Server::bind(
            remote.clone(),
            remote_trust,
            serde_json::json!({
                "preferred_logical":{"width":1138,"height":710},
                "h264":[
                    {"width":640,"height":480,"fps":30},
                    {"width":960,"height":600,"fps":30},
                    {"width":1138,"height":710,"fps":30},
                    {"width":2276,"height":1420,"fps":30}
                ]
            }),
            "127.0.0.1:0",
        )
        .unwrap();
        let config = Config {
            mode: Mode::Mirror,
            width: 640,
            height: 480,
            fps: 30,
            bitrate: 1_000_000,
            retina: false,
            auto_size: false,
            codec: VideoCodec::H264,
            separate_cursor: false,
        };
        let sender = crate::sender::Sender::start(
            identity,
            trust.clone(),
            remote.fingerprint(),
            ([127, 0, 0, 1], server.port).into(),
            config.clone(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.snapshot()["state"] != "offered" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let id = server.snapshot()["id"].as_str().unwrap().to_string();
        server.attach(&id).unwrap();
        while sender.snapshot()["state"] != "streaming" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let capture_id = sender.snapshot()["id"].as_str().unwrap().to_string();
        assert!(server.snapshot()["decoder_startup_ms"].is_null());
        let bytes = vec![0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21];
        sender
            .enqueue(
                &capture_id,
                Frame {
                    timestamp: 1_000_000_000,
                    keyframe: true,
                    bytes: bytes.clone(),
                },
            )
            .unwrap();
        let frame = loop {
            if let Some(frame) = server.take_frame(&id).unwrap() {
                break frame;
            }
            assert!(Instant::now() < deadline);
        };
        assert_eq!(&frame[17..], bytes);
        assert_eq!(frame[8], 1);
        server.rendered(&id, 0, 1_000_000_000, 1, 1).unwrap();
        assert_eq!(server.snapshot()["decoder_queue_steady"], true);
        let mut next = config.clone();
        next.width = 960;
        next.height = 600;
        next.bitrate = 2_000_000;
        sender.configure(&capture_id, next.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.snapshot()["state"] != "configuring" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(server.snapshot()["id"], id);
        assert_eq!(server.snapshot()["revision"], 1);
        assert!(server.snapshot()["decoder_startup_ms"].is_null());
        assert_eq!(server.snapshot()["decoder_queue_limit"], 64);
        assert_eq!(server.snapshot()["decoder_queue_steady"], false);
        assert!(server.take_frame(&id).unwrap().is_none());
        assert!(server.configured(&id, 0).is_err());
        server.rendered(&id, 0, 1_000_000_000, 1, 1).unwrap();
        assert_eq!(server.snapshot()["rendered_revision"], 0);
        assert_eq!(server.snapshot()["decoder_queue_steady"], false);
        server.configured(&id, 1).unwrap();
        while sender.snapshot()["revision"] != 1 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(sender.snapshot()["config"]["width"], 960);
        sender
            .enqueue(
                &capture_id,
                Frame {
                    timestamp: 2_000_000_000,
                    keyframe: true,
                    bytes: bytes.clone(),
                },
            )
            .unwrap();
        let resized = loop {
            if let Some(frame) = server.take_frame(&id).unwrap() {
                break frame;
            }
            assert!(Instant::now() < deadline);
        };
        assert_eq!(u64::from_be_bytes(resized[9..17].try_into().unwrap()), 1);
        assert_eq!(&resized[17..], bytes);
        assert!(!server.snapshot()["decoder_startup_ms"].is_null());
        server.rendered(&id, 1, 2_000_000_000, 2, 2).unwrap();
        assert_eq!(server.snapshot()["rendered_revision"], 1);
        let mut wrong_codec = next.clone();
        wrong_codec.codec = VideoCodec::Hevc;
        assert!(reconfigure(&server.shared, &remote.fingerprint(), wrong_codec).is_err());
        assert_eq!(
            server.snapshot()["config"],
            serde_json::to_value(&next).unwrap()
        );
        // A valid explicit size need not appear in the automatic size probes.
        // Switching back to Fit Display must negotiate exact probed rasters,
        // keep the session, and wait for the new decoder revision to attach.
        for (revision, retina) in [(2, false), (3, true)] {
            let mut fit = next.clone();
            fit.auto_size = true;
            fit.retina = retina;
            let shared = server.shared.clone();
            let peer = server.snapshot()["peer"].as_str().unwrap().to_string();
            let request = thread::spawn(move || reconfigure(&shared, &peer, fit));
            let deadline = Instant::now() + Duration::from_secs(5);
            while server.snapshot()["revision"] != revision {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(server.snapshot()["id"], id);
            assert_eq!(server.snapshot()["config"]["width"], 1138);
            assert_eq!(server.snapshot()["config"]["height"], 710);
            assert_eq!(server.snapshot()["config"]["retina"], retina);
            assert!(server.configured(&id, revision - 1).is_err());
            assert!(server.take_frame(&id).unwrap().is_none());
            server.configured(&id, revision).unwrap();
            let negotiated = request.join().unwrap().unwrap();
            assert_eq!((negotiated.width, negotiated.height), (1138, 710));
            assert_eq!(negotiated.retina, retina);
            assert!(negotiated.auto_size);
        }
        let before = server.snapshot();
        let input_before = server.input_state(&id).unwrap();
        let mut unsupported = next;
        unsupported.auto_size = true;
        unsupported.fps = 60;
        let peer = before["peer"].as_str().unwrap();
        assert!(reconfigure(&server.shared, peer, unsupported).is_err());
        let after = server.snapshot();
        assert_eq!(after["revision"], before["revision"]);
        assert_eq!(after["config"], before["config"]);
        assert_eq!(after["state"], before["state"]);
        assert_eq!(server.input_state(&id).unwrap(), input_before);
        trust.forget(&remote.fingerprint()).unwrap();
        while sender.snapshot()["state"] != "failed" {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(sender.snapshot()["error"]
            .as_str()
            .unwrap()
            .contains("forgotten"));
        server.stop();
    }
}
