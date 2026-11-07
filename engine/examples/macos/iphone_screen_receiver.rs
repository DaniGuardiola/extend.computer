// Mac-side receiver for iPhone screen sharing checks when the desktop app
// holds the wildcard display port. Binding a specific address (the Mac's LAN
// IP) on 48179 takes precedence over the app's `*:48179` for connections to
// that address. Uses the identity and trust of an `iphone_mac_peer` state dir.
//
// cargo run -p extend-computer-agent --example iphone_screen_receiver -- <state> <lan-ip> <secs>
//
// Shows the stream in the native display helper (target/extend.computer
// Display.app) and prints one `EVENT {json}` line per session milestone.
use anyhow::{Context, Result};
use extend_computer_agent::{
    display::{DisplayEvent, Transport},
    identity::Identity,
    session::display::{Config, DisplaySignal},
    trust::TrustStore,
};
use serde_json::{json, Value};
use std::{
    fs,
    net::{IpAddr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
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

fn event(kind: &str, detail: Value) {
    println!("EVENT {}", json!({"kind":kind,"detail":detail}));
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let state = PathBuf::from(args.first().context("state dir required")?);
    let ip: IpAddr = args.get(1).context("LAN ip required")?.parse()?;
    let secs: u64 = args.get(2).map_or(Ok(300), |s| s.parse())?;
    let seed = fs::read(state.join("seed")).context("state has no seed")?;
    let me = Identity::from_secret(seed.as_slice().try_into()?);
    let trust = TrustStore::open(state.join("trust"))?;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/extend.computer Display.app/Contents/MacOS/ExtendComputerDisplay");
    let listener = TcpListener::bind(SocketAddr::new(ip, 48179))?;
    event(
        "ready",
        json!({"fingerprint":me.fingerprint(),"bind":listener.local_addr()?.to_string()}),
    );
    let until = Instant::now() + Duration::from_secs(secs);
    listener.set_nonblocking(true)?;
    while Instant::now() < until {
        let Ok((socket, remote)) = listener.accept() else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        socket.set_nonblocking(false)?;
        event("accepted", json!({"remote":remote.to_string()}));
        if let Err(error) = session(socket, &me, &trust, &path, until) {
            event("session_error", json!(format!("{error:#}")));
        }
    }
    Ok(())
}

fn session(
    socket: std::net::TcpStream,
    me: &Identity,
    trust: &TrustStore,
    path: &Path,
    until: Instant,
) -> Result<()> {
    let local = socket.local_addr()?.ip();
    let remote = socket.peer_addr()?.ip();
    let shutdown = socket.try_clone()?;
    let mut media = None;
    let mut selected = None;
    let (mut signal, peer, _) =
        DisplaySignal::accept_with_features(socket, me, trust, |peer, config, offer, features| {
            event("offer", json!({"peer":peer,"config":config}));
            let mut transport = Transport::with_udp_peer(local, remote, config.codec)?;
            let answer = transport.answer_with_features(offer, features)?;
            media = Some(transport);
            selected = Some(config.clone());
            Ok(answer)
        })?;
    let mut media = media.context("Missing display transport")?;
    let mut config: Config = selected.context("Missing display configuration")?;
    // Signaling: heartbeats and size renegotiation (rotation, settings changes).
    let (changes, changed) = mpsc::channel::<Config>();
    let stopped = Arc::new(AtomicBool::new(false));
    let ended = stopped.clone();
    let signaling = thread::spawn(move || {
        loop {
            let answered = signal.respond_with_config(|next| {
                event("configure", json!(next));
                changes.send(next.clone())?;
                Ok(next)
            });
            match answered {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    event("signal_error", json!(format!("{error:#}")));
                    break;
                }
            }
        }
        ended.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    let mut viewer: Option<helper::Helper> = None;
    let (mut frames, mut keyframes, mut bytes, mut waiting) = (0u64, 0u64, 0u64, true);
    let mut last_report = Instant::now();
    let result = (|| -> Result<()> {
        while Instant::now() < until && !stopped.load(Ordering::SeqCst) {
            while let Ok(next) = changed.try_recv() {
                config = next;
                if let Some(viewer) = &viewer {
                    viewer.configure(&config)?;
                }
                waiting = true;
            }
            match media.poll()? {
                Some(DisplayEvent::Connected) if viewer.is_none() => {
                    event(
                        "connected",
                        json!({"peer":peer,"ms":started.elapsed().as_millis() as u64}),
                    );
                    viewer = Some(helper::Helper::start(
                        path,
                        &config,
                        false,
                        false,
                        &[],
                        Some(&peer),
                    )?);
                }
                Some(DisplayEvent::Frame(frame)) => {
                    frames += 1;
                    bytes += frame.bytes.len() as u64;
                    if frame.keyframe {
                        keyframes += 1;
                    }
                    if !waiting || frame.keyframe {
                        if let Some(viewer) = &viewer {
                            if viewer.present(frame) {
                                if waiting {
                                    event(
                                        "first_presented",
                                        json!({"frames":frames,"ms":started.elapsed().as_millis() as u64,"width":config.width,"height":config.height}),
                                    );
                                }
                                waiting = false;
                            } else {
                                waiting = true;
                                media.request_keyframe()?;
                            }
                        }
                    }
                }
                _ => {}
            }
            if let Some(viewer) = &viewer {
                if viewer.closed.load(Ordering::SeqCst) {
                    anyhow::bail!("viewer closed: {}", viewer.failure());
                }
            }
            if last_report.elapsed() >= Duration::from_secs(5) {
                last_report = Instant::now();
                event(
                    "progress",
                    json!({"frames":frames,"keyframes":keyframes,"bytes":bytes,"width":config.width,"height":config.height,"codec":config.codec.name()}),
                );
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    })();
    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    let _ = signaling.join();
    event(
        "ended",
        json!({"frames":frames,"keyframes":keyframes,"bytes":bytes,"secs":started.elapsed().as_secs_f64(),"error":result.as_ref().err().map(|e| format!("{e:#}"))}),
    );
    result
}
