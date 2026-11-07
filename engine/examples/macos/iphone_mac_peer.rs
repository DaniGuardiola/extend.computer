// Mac-side peer for iPhone hardware checks when the desktop app can't run.
// Disposable identity in `<state>/seed` (never a desktop user's identity).
//
// cargo run -p extend-computer-agent --example iphone_mac_peer -- <state> <bind ip:port> [cursor-helper]
//
// Serves control sessions on `<bind>` with a pairing window (code printed;
// pair from the phone with "Pair manually"). With a cursor helper executable,
// control input is injected into this Mac; otherwise it is only logged.
// Every received control/display input event is appended to `<state>/events.jsonl`.
//
// stdin commands stream the REAL Mac screen to the paired phone through the
// native display helper (ScreenCaptureKit + VideoToolbox):
//   display <lan|usb:UDID> <mirror|extend> <h264|hevc> <secs>
// Display input requests are granted (full content, text + Command
// shortcuts) and each command is logged and acknowledged, never injected.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    display::{
        input::{Binding, Geometry, Message, Rect, Size},
        DisplayEvent, Transport,
    },
    identity::Identity,
    input::{InputEvent, ShortcutPolicy},
    pairing::PairingWindow,
    protocol::offer,
    session::{
        self,
        display::{Config, DisplaySignal, Mode, VideoCodec},
        CursorSink, Decision,
    },
    trust::TrustStore,
};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
#[path = "../../../desktop/src-tauri/src/runtime/display/helper.rs"]
#[allow(dead_code, unexpected_cfgs)]
mod helper;
#[path = "../../../desktop/src-tauri/src/runtime/display/receiver.rs"]
#[allow(dead_code)]
mod receiver;

const NAME: &str = "iPhone Test Mac";

#[derive(Clone)]
struct Log(Arc<Mutex<fs::File>>, Instant);
impl Log {
    fn record(&self, kind: &str, detail: Value) {
        let line = json!({"kind":kind,"ms":self.1.elapsed().as_millis() as u64,"detail":detail});
        println!("EVENT {line}");
        let mut file = self.0.lock().unwrap();
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

/// Control sink: logs every event and forwards it to the native cursor helper when present.
struct Sink {
    log: Log,
    native: Option<extend_computer_agent::cursor::NativeSink>,
    active: bool,
}
impl CursorSink for Sink {
    fn shortcut_policy(&self) -> Option<ShortcutPolicy> {
        self.native
            .as_ref()
            .and_then(|n| n.shortcut_policy())
            .or(Some(ShortcutPolicy::Command))
    }
    fn supports_text_input(&self) -> bool {
        self.active
    }
    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.log.record("control_move", json!({"x":x,"y":y}));
        if let Some(native) = &mut self.native {
            native.move_to(x, y)?;
        }
        Ok(())
    }
    fn start_control(&mut self, peer: &str) -> Result<bool> {
        if let Some(native) = &mut self.native {
            if let Err(error) = native.start_control(peer) {
                self.log
                    .record("control_helper_failed", json!(format!("{error:#}")));
                self.native = None;
            }
        }
        self.active = true;
        self.log.record(
            "control_start",
            json!({"peer":peer,"injecting":self.native.is_some()}),
        );
        Ok(true)
    }
    fn input(&mut self, event: &InputEvent) -> Result<()> {
        self.log.record("control_input", json!(event));
        if let Some(native) = &mut self.native {
            if matches!(event, InputEvent::Text { .. }) && !native.supports_text_input() {
                return Ok(());
            }
            native.input(event)?;
        }
        Ok(())
    }
    fn device_name(&self) -> Option<String> {
        Some(NAME.into())
    }
    fn finish(&mut self) {
        self.active = false;
        if let Some(native) = &mut self.native {
            native.finish();
        }
        self.log.record("control_finish", json!({}));
    }
}

fn identity(state: &Path) -> Result<Identity> {
    fs::create_dir_all(state)?;
    let path = state.join("seed");
    if let Ok(bytes) = fs::read(&path) {
        return Ok(Identity::from_secret(bytes.as_slice().try_into()?));
    }
    let identity = Identity::generate();
    fs::write(&path, identity.secret())?;
    Ok(identity)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let state = PathBuf::from(args.first().context("state dir required")?);
    let bind: SocketAddr = args.get(1).context("bind ip:port required")?.parse()?;
    let cursor_helper = args.get(2).map(PathBuf::from);
    let me = Arc::new(identity(&state)?);
    let trust = Arc::new(TrustStore::open(state.join("trust"))?);
    let log = Log(
        Arc::new(Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(state.join("events.jsonl"))?,
        )),
        Instant::now(),
    );
    let phone: Arc<Mutex<Option<(String, std::net::IpAddr)>>> = Arc::new(Mutex::new(
        fs::read(state.join("phone.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| {
                Some((
                    v["peer"].as_str()?.to_owned(),
                    v["ip"].as_str()?.parse().ok()?,
                ))
            }),
    ));
    let listener = reuse_listener(bind)?;
    // Presence on <bind ip>:48178 (SO_REUSEADDR beats a wildcard holder) points the phone at <bind>.
    if let Ok(presence) = reuse_listener(SocketAddr::new(bind.ip(), 48178)) {
        let (me, trust, log) = (me.clone(), trust.clone(), log.clone());
        thread::spawn(move || {
            for socket in presence.incoming().flatten() {
                let result = session::serve_presence(socket, &me, &trust, || session::Presence {
                    receiving: true,
                    port: bind.port(),
                    name: NAME.into(),
                });
                if let Err(error) = result {
                    log.record("presence_failed", json!(format!("{error:#}")));
                }
            }
        });
    }
    let window = Arc::new(Mutex::new(Some(PairingWindow::new(Duration::from_secs(
        600,
    )))));
    let code = window
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|w| w.code().map(str::to_owned));
    println!(
        "READY fingerprint={} listening={} code={:?}",
        me.fingerprint(),
        listener.local_addr()?,
        code
    );
    log.record(
        "ready",
        json!({"fingerprint":me.fingerprint(),"bind":bind.to_string()}),
    );
    {
        let (me, trust, log, phone, window) = (
            me.clone(),
            trust.clone(),
            log.clone(),
            phone.clone(),
            window.clone(),
        );
        let state = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (me, trust, log, phone, window) = (
                    me.clone(),
                    trust.clone(),
                    log.clone(),
                    phone.clone(),
                    window.clone(),
                );
                let helper = cursor_helper.clone();
                let state = state.clone();
                thread::spawn(move || {
                    let ip = stream.peer_addr().map(|a| a.ip()).ok();
                    let mut sink = Sink {
                        log: log.clone(),
                        native: helper
                            .map(|p| extend_computer_agent::cursor::NativeSink::new(Some(p))),
                        active: false,
                    };
                    let mut unpaired = None;
                    let paired = trust.load().map(|r| !r.peers.is_empty()).unwrap_or(false);
                    let mut guard = if paired {
                        None
                    } else {
                        Some(window.lock().unwrap())
                    };
                    let slot = guard.as_deref_mut().unwrap_or(&mut unpaired);
                    let result = session::serve_connection_with_cursor(
                        stream,
                        &me,
                        &trust,
                        slot,
                        |_, _| Decision::Remember,
                        &mut sink,
                    );
                    drop(guard);
                    if let (Ok(records), Some(ip)) = (trust.load(), ip) {
                        if let Some(peer) = records.peers.into_keys().next() {
                            let value = json!({"peer":peer,"ip":ip.to_string()});
                            let _ = fs::write(state.join("phone.json"), value.to_string());
                            *phone.lock().unwrap() = Some((peer, ip));
                        }
                    }
                    log.record(
                        "connection_closed",
                        json!({"ok":result.is_ok(),"error":result.err().map(|e| format!("{e:#}"))}),
                    );
                });
            }
        });
    }
    let display_helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/extend.computer Display.app/Contents/MacOS/ExtendComputerDisplay");
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() != Some(&"display") {
            println!("UNKNOWN {line}");
            continue;
        }
        let Some((peer, ip)) = phone.lock().unwrap().clone() else {
            println!("NO_PHONE pair from the phone first");
            continue;
        };
        let result = display(
            &me,
            &peer,
            ip,
            words.get(1).copied().unwrap_or("lan"),
            words.get(2) == Some(&"extend"),
            words.get(3) == Some(&"hevc"),
            words.get(4).and_then(|s| s.parse().ok()).unwrap_or(30),
            &display_helper,
            &log,
        );
        println!("DISPLAY_DONE {result:?}");
        log.record("display_done", json!(format!("{result:?}")));
    }
    Ok(())
}

fn reuse_listener(address: SocketAddr) -> Result<TcpListener> {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::for_address(address), Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    socket.listen(16)?;
    Ok(socket.into())
}

#[allow(clippy::too_many_arguments)]
fn display(
    me: &Identity,
    peer: &str,
    ip: std::net::IpAddr,
    route: &str,
    extend: bool,
    hevc: bool,
    secs: u64,
    helper_path: &Path,
    log: &Log,
) -> Result<Value> {
    let requested = Config {
        mode: if extend { Mode::Extend } else { Mode::Mirror },
        width: 1280,
        height: 800,
        fps: 60,
        bitrate: 12_000_000,
        retina: false,
        auto_size: false,
        codec: if hevc {
            VideoCodec::Hevc
        } else {
            VideoCodec::H264
        },
        separate_cursor: true,
    };
    let stop = AtomicBool::new(false);
    #[cfg(target_os = "macos")]
    let usb = if let Some(udid) = route.strip_prefix("usb:") {
        let host = extend_computer_agent::display::usb_host::NativeUsbHost::connect_usbmux(
            udid,
            &me.fingerprint(),
        )?;
        host.hub().enable_streams()?;
        Some(host)
    } else {
        None
    };
    let socket = if let Some(host) = &usb {
        host.hub().open_stream(
            extend_computer_agent::display::usb_stream::UsbService::Display,
            &stop,
            Instant::now() + Duration::from_secs(5),
        )?
    } else {
        TcpStream::connect_timeout(&SocketAddr::new(ip, 48179), Duration::from_secs(3))?
    };
    let local_ip = socket.local_addr()?.ip();
    let shutdown = socket.try_clone()?;
    let mut media = None;
    let (mut signal, config, answer, features) = DisplaySignal::connect_for_peer(
        socket,
        me,
        peer,
        requested,
        false,
        |config, features, offers| {
            if config.codec == VideoCodec::Hevc && !offers.allows(offer::DISPLAY_DECODE_HEVC) {
                config.codec = VideoCodec::H264;
            }
            config.separate_cursor &= offers.allows(offer::DISPLAY_CURSOR_OVERLAY);
            let mut transport = match &usb {
                Some(host) => host.transport(config.codec)?,
                None => Transport::with_udp_peer(local_ip, ip, config.codec)?,
            };
            let offer = transport.offer_with_features(features)?;
            media = Some(transport);
            Ok(offer)
        },
    )?;
    let mut media = media.context("Missing display transport")?;
    media.accept_answer(&answer)?;
    media.set_target_bitrate(config.bitrate)?;
    log.record(
        "display_config",
        json!({"config":config,"route":route,"features":format!("{features:?}")}),
    );
    let stopped = Arc::new(AtomicBool::new(false));
    let monitor = stopped.clone();
    let heartbeat = thread::spawn(move || {
        let mut sequence = 0;
        while !monitor.load(Ordering::Acquire) {
            if signal.pulse(sequence).is_err() {
                break;
            }
            sequence += 1;
            thread::sleep(Duration::from_millis(500));
        }
        signal.close();
        monitor.store(true, Ordering::Release);
    });
    let scale = if config.retina { 2 } else { 1 };
    let raster = Size {
        width: config.width * scale,
        height: config.height * scale,
    };
    let binding = Binding::new(1, 0);
    let result = (|| -> Result<Value> {
        let until = Instant::now() + Duration::from_secs(secs);
        let mut capture: Option<helper::Helper> = None;
        let (mut encoded, mut inputs) = (0u64, 0u64);
        while Instant::now() < until && !stopped.load(Ordering::Acquire) {
            match media.poll()? {
                Some(DisplayEvent::Connected) if capture.is_none() => {
                    capture = Some(helper::Helper::start(
                        helper_path,
                        &config,
                        true,
                        false,
                        &[],
                        Some(peer),
                    )?);
                    log.record("display_connected", json!({}));
                }
                Some(DisplayEvent::Keyframe) => {
                    if let Some(capture) = &capture {
                        capture.request_keyframe()?;
                    }
                }
                Some(DisplayEvent::Bitrate(estimate)) => {
                    if let Some(capture) = &capture {
                        capture.set_bitrate(
                            (estimate * 85 / 100).clamp(1_000_000, config.bitrate as u64) as u32,
                        )?;
                    }
                }
                Some(DisplayEvent::Input(Message::Request { request })) => {
                    let granted = Message::Granted {
                        request,
                        binding: binding.clone(),
                        geometry: Geometry {
                            content: Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 1.0,
                                height: 1.0,
                            },
                            canvas: Size {
                                width: config.width,
                                height: config.height,
                            },
                            raster,
                            source_size: raster,
                        },
                        text_supported: media.text_input_negotiated(),
                        shortcut_policy: Some(ShortcutPolicy::Command),
                    };
                    log.record("display_input_request", json!({"request":request}));
                    media.write_input(&granted)?;
                }
                Some(DisplayEvent::Input(Message::Command {
                    binding: active,
                    sequence,
                    command,
                })) => {
                    inputs += 1;
                    log.record(
                        "display_input",
                        json!({"sequence":sequence,"command":command}),
                    );
                    if active == binding {
                        media.write_input(&Message::Applied {
                            binding: active,
                            sequence,
                        })?;
                    }
                }
                Some(DisplayEvent::Input(other)) => log.record("display_input_other", json!(other)),
                _ => {}
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
                        if !media.write_cursor(&bytes)? {
                            slot.lock().unwrap().get_or_insert(bytes);
                        }
                    }
                }
                while let Ok(frame) = capture.frames.try_recv() {
                    encoded += 1;
                    media.write(frame)?;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(
            json!({"encoded":encoded,"display_inputs":inputs,"codec":config.codec.name(),"mode":format!("{:?}",config.mode),
            "width":config.width,"height":config.height,"separate_cursor":config.separate_cursor}),
        )
    })();
    stopped.store(true, Ordering::Release);
    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    let _ = heartbeat.join();
    stop.store(true, Ordering::Release);
    result
}
