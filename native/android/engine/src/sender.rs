//! Pinned outgoing display. Encoded frames remain native and queues stay bounded.
use anyhow::{ensure, Result};
use extend_computer_agent::{
    display::{DisplayEvent, Frame},
    identity::Identity,
    session::display::{Config, DisplaySignal},
    trust::TrustStore,
};
use std::{
    collections::VecDeque,
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
static IDS: AtomicU64 = AtomicU64::new(1);
struct State {
    active: bool,
    phase: &'static str,
    route: &'static str,
    error: String,
    socket: Option<TcpStream>,
    config: Config,
    pending: Option<Config>,
    revision: u64,
    frames: VecDeque<Frame>,
    bytes: usize,
    sent: u64,
    sent_revision: u64,
    dropped: u64,
    keyframe: bool,
    last_timestamp: Option<u64>,
    stop: Arc<AtomicBool>,
}
impl State {
    fn request_keyframe(&mut self) {
        self.dropped += self.frames.len() as u64;
        self.frames.clear();
        self.bytes = 0;
        self.keyframe = true;
    }
}
pub struct Sender {
    id: String,
    peer: String,
    state: Arc<Mutex<State>>,
}
impl Sender {
    pub fn start(
        identity: Arc<Identity>,
        trust: TrustStore,
        peer: String,
        address: SocketAddr,
        config: Config,
    ) -> Result<Self> {
        Self::start_selected(identity, trust, peer, Some(address), None, config)
    }
    pub fn start_routed(
        identity: Arc<Identity>,
        trust: TrustStore,
        peer: String,
        address: String,
        config: Config,
    ) -> Result<Self> {
        let usb = crate::usb::selected(&peer, &address)?;
        let address = if usb.is_some() {
            None
        } else {
            Some(crate::endpoint(&address)?)
        };
        Self::start_selected(identity, trust, peer, address, usb, config)
    }
    fn start_selected(
        identity: Arc<Identity>,
        trust: TrustStore,
        peer: String,
        address: Option<SocketAddr>,
        usb: Option<crate::usb::Route>,
        config: Config,
    ) -> Result<Self> {
        config.validate()?;
        ensure!(
            !config.auto_size && !config.retina && !config.separate_cursor,
            "Unsupported capture configuration"
        );
        ensure!(trust.peer(&peer)?.is_some(), "Device is not paired");
        let id = IDS.fetch_add(1, Ordering::Relaxed).to_string();
        let state = Arc::new(Mutex::new(State {
            active: true,
            phase: "connecting",
            route: "connecting",
            error: String::new(),
            socket: None,
            config: config.clone(),
            pending: None,
            revision: 0,
            frames: VecDeque::new(),
            bytes: 0,
            sent: 0,
            sent_revision: 0,
            dropped: 0,
            keyframe: true,
            last_timestamp: None,
            stop: Arc::new(AtomicBool::new(false)),
        }));
        let shared = state.clone();
        let remote = peer.clone();
        thread::Builder::new()
            .name("extend-display-sender".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let stop = shared.lock().unwrap().stop.clone();
                    let socket = match &usb {
                        Some(route) => route.open(
                            extend_computer_agent::display::usb_stream::UsbService::Display,
                            &stop,
                            Instant::now() + Duration::from_secs(3),
                        )?,
                        None => TcpStream::connect_timeout(
                            &address
                                .ok_or_else(|| anyhow::anyhow!("Device address unavailable"))?,
                            Duration::from_secs(3),
                        )?,
                    };
                    {
                        let mut s = shared.lock().unwrap();
                        ensure!(s.active, "Capture canceled");
                        s.socket = Some(socket.try_clone()?);
                    }
                    let (mut transport, route) = match &usb {
                        Some(route) => (route.transport(config.codec)?, "usb"),
                        None => (
                            extend_computer_agent::display::Transport::with_codec(
                                socket.local_addr()?.ip(),
                                config.codec,
                            )?,
                            "lan",
                        ),
                    };
                    shared.lock().unwrap().route = route;
                    let offer = transport.offer()?;
                    let (mut signal, selected, answer) =
                        DisplaySignal::connect(socket, &identity, &remote, &config, &offer)?;
                    transport.accept_answer(&answer)?;
                    {
                        let mut s = shared.lock().unwrap();
                        ensure!(s.active, "Capture canceled");
                        s.config = selected;
                        s.phase = "negotiated";
                    }
                    let heartbeat_state = shared.clone();
                    let heartbeat = thread::spawn(move || {
                        let mut sequence = 0;
                        while heartbeat_state.lock().unwrap().active {
                            let pending = heartbeat_state.lock().unwrap().pending.take();
                            let exchange = (|| -> Result<()> {
                                if let Some(config) = pending {
                                    let applied = signal.configure(&config)?;
                                    let mut s = heartbeat_state.lock().unwrap();
                                    ensure!(s.active, "Capture canceled");
                                    s.config = applied;
                                    s.revision += 1;
                                    s.sent_revision = 0;
                                    s.request_keyframe();
                                    s.phase = "streaming";
                                }
                                signal.pulse(sequence)
                            })();
                            if let Err(error) = exchange {
                                let mut s = heartbeat_state.lock().unwrap();
                                if s.active {
                                    s.error = format!("{error:#}");
                                    s.phase = "failed";
                                    s.active = false;
                                }
                                break;
                            }
                            sequence += 1;
                            thread::sleep(Duration::from_millis(300));
                        }
                        signal.close();
                    });
                    let run = (|| -> Result<()> {
                        let deadline = Instant::now() + Duration::from_secs(10);
                        let mut connected = false;
                        while shared.lock().unwrap().active {
                            ensure!(trust.peer(&remote)?.is_some(), "Device was forgotten");
                            ensure!(
                                usb.as_ref().is_none_or(|route| route.current()),
                                "USB connection changed"
                            );
                            match transport.poll()? {
                                Some(DisplayEvent::Connected) => {
                                    connected = true;
                                    shared.lock().unwrap().phase = "streaming";
                                }
                                Some(DisplayEvent::Keyframe) => {
                                    shared.lock().unwrap().request_keyframe()
                                }
                                _ => {}
                            }
                            ensure!(
                                connected || Instant::now() < deadline,
                                "Media connection timed out"
                            );
                            if connected {
                                let mut s = shared.lock().unwrap();
                                if s.phase == "streaming" {
                                    let f = s.frames.pop_front();
                                    if let Some(f) = &f {
                                        s.bytes -= f.bytes.len();
                                    }
                                    if let Some(frame) = f {
                                        if transport.write(frame)? {
                                            s.sent += 1;
                                            s.sent_revision += 1;
                                        } else {
                                            s.dropped += 1;
                                            s.request_keyframe();
                                        }
                                    }
                                }
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                        Ok(())
                    })();
                    {
                        let mut s = shared.lock().unwrap();
                        s.active = false;
                        if let Some(socket) = &s.socket {
                            let _ = socket.shutdown(Shutdown::Both);
                        }
                    }
                    let _ = heartbeat.join();
                    run
                })();
                let mut s = shared.lock().unwrap();
                if s.phase != "closed" && s.phase != "failed" {
                    s.phase = if result.is_ok() { "closed" } else { "failed" };
                    s.error = result.err().map(|e| format!("{e:#}")).unwrap_or_default();
                }
                s.active = false;
                s.socket = None;
                s.frames.clear();
                s.bytes = 0;
            })?;
        Ok(Self { id, peer, state })
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let s = self.state.lock().unwrap();
        serde_json::json!({"id":self.id,"peer":self.peer,"state":s.phase,"route":s.route,"error":s.error,"config":s.config,"revision":s.revision,"sent":s.sent,"sent_revision":s.sent_revision,"dropped":s.dropped,"keyframe":s.keyframe})
    }
    pub fn configure(&self, id: &str, config: Config) -> Result<()> {
        ensure!(id == self.id, "Stale capture session");
        config.validate()?;
        let mut s = self.state.lock().unwrap();
        ensure!(
            s.active && s.phase == "streaming",
            "Capture is not ready for settings"
        );
        ensure!(
            config.mode == s.config.mode
                && config.codec == s.config.codec
                && !config.auto_size
                && !config.retina
                && !config.separate_cursor,
            "Unsupported capture settings change"
        );
        s.request_keyframe();
        s.pending = Some(config);
        s.phase = "configuring";
        Ok(())
    }
    pub fn enqueue(&self, id: &str, frame: Frame) -> Result<()> {
        ensure!(id == self.id, "Stale capture session");
        ensure!(
            !frame.bytes.is_empty() && frame.bytes.len() <= 8 * 1024 * 1024,
            "Invalid encoded frame"
        );
        let mut s = self.state.lock().unwrap();
        ensure!(s.active, "Capture stopped");
        ensure!(
            s.last_timestamp.is_none_or(|t| frame.timestamp > t),
            "Non-increasing capture timestamp"
        );
        s.last_timestamp = Some(frame.timestamp);
        if s.phase != "streaming" {
            s.dropped += 1;
            return Ok(());
        }
        if s.frames.len() >= 4 || s.bytes + frame.bytes.len() > 8 * 1024 * 1024 {
            s.request_keyframe();
        }
        if s.keyframe && !frame.keyframe {
            s.dropped += 1;
            return Ok(());
        }
        if frame.keyframe {
            s.dropped += s.frames.len() as u64;
            s.frames.clear();
            s.bytes = 0;
            s.keyframe = false;
        }
        s.bytes += frame.bytes.len();
        s.frames.push_back(frame);
        Ok(())
    }
    pub fn cancel(&self, id: &str) {
        if id != self.id {
            return;
        }
        let mut s = self.state.lock().unwrap();
        s.active = false;
        s.stop.store(true, Ordering::Release);
        s.phase = "closed";
        s.frames.clear();
        s.bytes = 0;
        s.pending = None;
        if let Some(socket) = &s.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.cancel(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::session::display::{Mode, VideoCodec};
    fn sender() -> Sender {
        Sender {
            id: "test".into(),
            peer: "peer".into(),
            state: Arc::new(Mutex::new(State {
                stop: Arc::new(AtomicBool::new(false)),
                active: true,
                phase: "streaming",
                route: "lan",
                error: String::new(),
                socket: None,
                frames: VecDeque::new(),
                bytes: 0,
                sent: 0,
                sent_revision: 0,
                dropped: 0,
                keyframe: true,
                last_timestamp: None,
                pending: None,
                revision: 0,
                config: Config {
                    mode: Mode::Mirror,
                    width: 640,
                    height: 480,
                    fps: 30,
                    bitrate: 1_000_000,
                    retina: false,
                    auto_size: false,
                    codec: VideoCodec::H264,
                    separate_cursor: false,
                },
            })),
        }
    }
    fn frame(timestamp: u64, keyframe: bool) -> Frame {
        Frame {
            timestamp,
            keyframe,
            bytes: vec![0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21],
        }
    }
    #[test]
    fn bounded_queue_recovers_at_keyframe_and_rejects_stale_timestamps() {
        let sender = sender();
        sender.enqueue("test", frame(1, false)).unwrap();
        assert_eq!(sender.state.lock().unwrap().frames.len(), 0);
        assert!(sender.enqueue("stale", frame(2, true)).is_err());
        sender.enqueue("test", frame(2, true)).unwrap();
        assert!(sender.enqueue("test", frame(2, false)).is_err());
        for timestamp in 3..=5 {
            sender.enqueue("test", frame(timestamp, false)).unwrap();
        }
        sender.enqueue("test", frame(6, false)).unwrap();
        {
            let s = sender.state.lock().unwrap();
            assert_eq!(s.frames.len(), 0);
            assert_eq!(s.bytes, 0);
            assert!(s.keyframe);
        }
        sender.enqueue("test", frame(7, true)).unwrap();
        sender.enqueue("test", frame(8, false)).unwrap();
        sender.state.lock().unwrap().request_keyframe();
        sender.enqueue("test", frame(9, false)).unwrap();
        assert!(sender.state.lock().unwrap().frames.is_empty());
        sender.enqueue("test", frame(10, true)).unwrap();
        sender.cancel("stale");
        assert_eq!(sender.snapshot()["state"], "streaming");
        sender.cancel("test");
        assert_eq!(sender.snapshot()["state"], "closed");
        assert!(sender.state.lock().unwrap().frames.is_empty());
        assert!(sender.enqueue("test", frame(11, true)).is_err());
    }
    #[test]
    fn configuration_clears_old_frames_blocks_stale_encoding_and_cancels() {
        let sender = sender();
        sender.enqueue("test", frame(1, true)).unwrap();
        let mut config = sender.state.lock().unwrap().config.clone();
        config.width = 800;
        config.height = 1280;
        let mut wrong_codec = config.clone();
        wrong_codec.codec = VideoCodec::Hevc;
        assert!(sender.configure("test", wrong_codec).is_err());
        assert!(sender.configure("old", config.clone()).is_err());
        assert_eq!(sender.state.lock().unwrap().frames.len(), 1);
        sender.configure("test", config.clone()).unwrap();
        assert_eq!(sender.snapshot()["state"], "configuring");
        assert!(sender.state.lock().unwrap().frames.is_empty());
        sender.enqueue("test", frame(2, true)).unwrap();
        assert!(sender.state.lock().unwrap().frames.is_empty());
        assert!(sender.configure("test", config).is_err());
        sender.cancel("test");
        assert!(sender.state.lock().unwrap().pending.is_none());
        assert_eq!(sender.snapshot()["state"], "closed");
    }
}
