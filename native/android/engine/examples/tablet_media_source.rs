//! Temporary identity + generated pattern. Exercises real encryption and Android decoding.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    display::{DisplayEvent, Frame, Transport},
    identity::Identity,
    session::{
        self,
        display::{Config, DisplaySignal, Mode, VideoCodec},
        Client, Decision,
    },
    trust::TrustStore,
};
use std::{
    fs,
    io::{BufRead, Cursor, Read},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
fn pattern(path: &str, offset: u64) -> Result<Vec<Frame>> {
    let bytes = fs::read(path)?;
    ensure!(bytes.len() < 32 * 1024 * 1024, "Pattern file too large");
    let mut input = Cursor::new(bytes);
    let mut frames = Vec::new();
    while (input.position() as usize) < input.get_ref().len() {
        let mut size = [0; 4];
        input.read_exact(&mut size)?;
        let count = u32::from_be_bytes(size) as usize;
        ensure!(
            (10..=8 * 1024 * 1024).contains(&count),
            "Invalid frame size"
        );
        let mut bytes = vec![0; count];
        input.read_exact(&mut bytes)?;
        frames.push(Frame {
            timestamp: u64::from_be_bytes(bytes[..8].try_into()?) + offset,
            keyframe: bytes[8] == 1,
            bytes: bytes[9..].to_vec(),
        });
    }
    ensure!(
        !frames.is_empty() && frames[0].keyframe,
        "Missing startup keyframe"
    );
    Ok(frames)
}

fn stream(
    identity: &Identity,
    peer: &str,
    address: SocketAddr,
    path: &str,
    codec: VideoCodec,
    resized_path: Option<&str>,
    native_transport: &dyn Fn(std::net::IpAddr, VideoCodec) -> Result<Transport>,
) -> Result<u64> {
    let config = Config {
        mode: Mode::Extend,
        width: 1280,
        height: 800,
        fps: 30,
        bitrate: 8_000_000,
        retina: false,
        auto_size: false,
        codec,
        separate_cursor: false,
    };
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    let shutdown = socket.try_clone()?;
    let mut transport = native_transport(socket.local_addr()?.ip(), codec)?;
    let offer = transport.offer()?;
    let (mut signal, _, answer) = DisplaySignal::connect(socket, identity, peer, &config, &offer)?;
    transport.accept_answer(&answer)?;
    let frames = pattern(path, 0)?;
    let (updates, requests) = std::sync::mpsc::channel::<Config>();
    let (applied, replies) = std::sync::mpsc::channel::<Result<Config>>();
    let stopped = Arc::new(AtomicBool::new(false));
    let monitor = stopped.clone();
    let heartbeat = thread::spawn(move || {
        while !monitor.load(Ordering::Acquire) {
            if let Ok(config) = requests.try_recv() {
                if applied.send(signal.configure(&config)).is_err() {
                    break;
                }
            }
            if signal.pulse(0).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(300));
        }
        signal.close();
    });
    let result = (|| -> Result<u64> {
        let deadline =
            Instant::now() + Duration::from_secs(if resized_path.is_some() { 18 } else { 8 });
        let mut stage_start = None;
        let mut sent: u64 = 0;
        let mut frames = frames.into_iter().peekable();
        let mut stage_sent = 0u64;
        let mut resized = false;
        while Instant::now() < deadline {
            if let Some(DisplayEvent::Connected) = transport.poll()? {
                stage_start = Some(Instant::now());
            }
            if let Some(start) = stage_start {
                if frames.peek().is_some()
                    && start.elapsed().as_millis() >= stage_sent as u128 * 1000 / 30
                {
                    ensure!(
                        transport.write(frames.next().unwrap())?,
                        "Transport refused generated frame"
                    );
                    sent += 1;
                    stage_sent += 1;
                }
                if frames.peek().is_none() && start.elapsed() > Duration::from_secs(6) {
                    if let Some(path) = resized_path.filter(|_| !resized) {
                        let mut next = config.clone();
                        next.width = 960;
                        next.height = 600;
                        next.bitrate = 4_000_000;
                        updates.send(next.clone())?;
                        ensure!(
                            replies.recv_timeout(Duration::from_secs(9))?? == next,
                            "Resize answer mismatch"
                        );
                        frames = pattern(path, 6_000_000_000)?.into_iter().peekable();
                        stage_sent = 0;
                        resized = true;
                        stage_start = Some(Instant::now());
                    } else {
                        break;
                    }
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        ensure!(sent >= 80, "Too few generated frames delivered");
        Ok(sent)
    })();
    stopped.store(true, Ordering::Release);
    let _ = shutdown.shutdown(Shutdown::Both);
    let _ = heartbeat.join();
    result
}

fn cursor_stream(
    identity: &Identity,
    peer: &str,
    address: SocketAddr,
    path: &str,
    codec: VideoCodec,
    native_transport: &dyn Fn(std::net::IpAddr, VideoCodec) -> Result<Transport>,
    png: &str,
    receipt: &std::path::Path,
    commands: &Receiver<String>,
) -> Result<u64> {
    ensure!(png.len() <= 120 * 1024, "Cursor fixture is too large");
    let config = Config {
        mode: Mode::Extend,
        width: 1280,
        height: 800,
        fps: 30,
        bitrate: 8_000_000,
        retina: false,
        auto_size: false,
        codec,
        separate_cursor: true,
    };
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    let shutdown = socket.try_clone()?;
    let mut transport = native_transport(socket.local_addr()?.ip(), codec)?;
    let offer = transport.offer()?;
    let (mut signal, selected, answer) =
        DisplaySignal::connect(socket, identity, peer, &config, &offer)?;
    ensure!(
        selected == config,
        "Receiver did not accept cursor fixture configuration"
    );
    transport.accept_answer(&answer)?;
    let frames = pattern(path, 0)?;
    let stopped = Arc::new(AtomicBool::new(false));
    let monitor = stopped.clone();
    let heartbeat = thread::spawn(move || {
        while !monitor.load(Ordering::Acquire) {
            if signal.pulse(0).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(300));
        }
        signal.close();
    });
    let result = (|| -> Result<u64> {
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut connected = false;
        let mut stage = 0;
        let mut announced = false;
        let mut video_start = Instant::now();
        let mut frames = frames.into_iter();
        let mut sent = 0u64;
        let mut revision = 1u64;
        let mut sequence = 0u64;
        let mut shape_sent = false;
        let mut position_sent = false;
        let mut last_position = Instant::now() - Duration::from_secs(1);
        while Instant::now() < deadline {
            if let Some(DisplayEvent::Connected) = transport.poll()? {
                connected = true;
            }
            if !connected {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            if !shape_sent {
                let shape = serde_json::to_vec(&serde_json::json!({"revision":revision,"png":png,
                    "width":32,"height":24,"hotspot_x":5,"hotspot_y":7}))?;
                shape_sent = transport.write_cursor(&shape)?;
            }
            let (x, y, visible) = if stage < 2 {
                (0.2, 0.25, true)
            } else {
                (0.72, 0.38, stage == 2)
            };
            if !position_sent || last_position.elapsed() >= Duration::from_millis(500) {
                let position = serde_json::to_vec(&serde_json::json!({"revision":revision,
                    "sequence":sequence+1,"x":x,"y":y,"visible":visible}))?;
                if transport.write_cursor(&position)? {
                    sequence += 1;
                    position_sent = true;
                    last_position = Instant::now();
                }
            }
            if shape_sent && position_sent && !announced {
                let name = ["initial", "video", "paused", "hidden"][stage];
                let value = serde_json::json!({"status":format!("{}-cursor-{name}",codec.name()),
                    "peer":identity.fingerprint(),"sent":sent,"video_paused":stage!=1,
                    "config":config,"cursor":{"revision":revision,"sequence":sequence,
                        "x":x,"y":y,"visible":visible,"width":32,"height":24,"hotspot_x":5,"hotspot_y":7}});
                let temporary = receipt.with_extension("cursor.tmp");
                fs::write(&temporary, value.to_string())?;
                fs::rename(temporary, receipt)?;
                announced = true;
            }
            if announced {
                match commands.try_recv() {
                    Ok(command) => match (stage, command.as_str()) {
                        (0, "cursor-video") => {
                            stage = 1;
                            video_start = Instant::now();
                            announced = false;
                        }
                        (1, "cursor-move") => {
                            ensure!(sent >= 120, "Pause requires at least 120 generated frames");
                            stage = 2;
                            revision = 2;
                            shape_sent = false;
                            position_sent = false;
                            announced = false;
                        }
                        (2, "cursor-hide") => {
                            stage = 3;
                            position_sent = false;
                            announced = false;
                        }
                        (3, "cursor-finish") => return Ok(sent),
                        _ => anyhow::bail!("Unexpected cursor fixture command"),
                    },
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        anyhow::bail!("Cursor fixture UI closed")
                    }
                }
            }
            if stage == 1 && video_start.elapsed().as_millis() >= sent as u128 * 1000 / 30 {
                if let Some(frame) = frames.next() {
                    ensure!(
                        transport.write(frame)?,
                        "Transport refused cursor fixture video"
                    );
                    sent += 1;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        anyhow::bail!("Cursor fixture timed out")
    })();
    stopped.store(true, Ordering::Release);
    let _ = shutdown.shutdown(Shutdown::Both);
    let _ = heartbeat.join();
    result
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let fixture: serde_json::Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let tablet = fixture["peer"].as_str().context("Tablet pin required")?;
    let addr: SocketAddr = fixture["address"].as_str().unwrap().parse()?;
    let cursor_png = fixture.get("cursor_png").and_then(|value| value.as_str());
    ensure!(
        cursor_png.is_none() || args.len() == 5,
        "Cursor fixture does not combine with resize"
    );
    let files = tempfile::tempdir()?;
    let identity = Identity::generate();
    let trust = TrustStore::open(files.path())?;
    fs::write(
        &args[2],
        serde_json::json!({"status":"waiting","peer":identity.fingerprint()}).to_string(),
    )?;
    let mut client = Client::connect(
        TcpStream::connect(addr)?,
        &identity,
        &trust,
        Some(fixture["code"].as_str().unwrap()),
        Some(tablet),
        |_, _| Decision::Remember,
    )?;
    client.probe()?;
    client.close()?;
    session::exchange_device_name(
        TcpStream::connect(addr)?,
        &identity,
        tablet,
        "Media Test Mac",
    )?;
    fs::write(
        &args[2],
        serde_json::json!({"status":"paired","peer":identity.fingerprint()}).to_string(),
    )?;
    // One reader owns stdin throughout both codecs; cursor stage acknowledgements
    // never block native RTC polling or strand a reader from the previous stream.
    let (command_tx, commands) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if command_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut signal = commands.recv().context("UI-ready signal missing")?;
    #[cfg(target_os = "macos")]
    let usb = if let Ok(helper) = std::env::var("EXTEND_NATIVE_USB_HOST") {
        ensure!(
            signal.trim() == "usb",
            "Expected explicit USB-ready request"
        );
        fs::write(
            &args[2],
            serde_json::json!({"status":"usb-connecting","peer":identity.fingerprint()})
                .to_string(),
        )?;
        let host = extend_computer_agent::display::usb_host::NativeUsbHost::connect(
            std::path::Path::new(&helper),
            &std::env::var("EXTEND_NATIVE_USB_VENDOR")?,
            &std::env::var("EXTEND_NATIVE_USB_SERIAL")?,
            &identity.fingerprint(),
        )?;
        fs::write(&args[2], serde_json::json!({"status":"usb-connected","peer":identity.fingerprint(),"usb":host.snapshot()}).to_string())?;
        signal = commands.recv().context("USB-ready signal missing")?;
        ensure!(
            signal.trim() == "usb-ready",
            "Expected Android accessory-ready signal"
        );
        signal = commands.recv().context("UI-ready signal missing")?;
        Some(host)
    } else {
        None
    };
    #[cfg(not(target_os = "macos"))]
    ensure!(
        std::env::var_os("EXTEND_NATIVE_USB_HOST").is_none(),
        "Native USB host currently requires macOS"
    );
    let native_transport = |ip, codec| {
        #[cfg(target_os = "macos")]
        if let Some(host) = usb.as_ref() {
            return host.transport(codec);
        }
        Transport::with_codec(ip, codec)
    };
    ensure!(signal.trim() == "h264", "Expected explicit UI-ready signal");
    let display = SocketAddr::new(addr.ip(), 48179);
    let avc = if let Some(png) = cursor_png {
        cursor_stream(
            &identity,
            tablet,
            display,
            &args[3],
            VideoCodec::H264,
            &native_transport,
            png,
            std::path::Path::new(&args[2]),
            &commands,
        )?
    } else {
        stream(
            &identity,
            tablet,
            display,
            &args[3],
            VideoCodec::H264,
            args.get(5).map(String::as_str),
            &native_transport,
        )?
    };
    fs::write(
        &args[2],
        serde_json::json!({"status":"h264-sent","peer":identity.fingerprint(),"sent":avc})
            .to_string(),
    )?;
    signal = commands.recv().context("Second UI-ready signal missing")?;
    ensure!(signal.trim() == "hevc", "Expected second UI-ready signal");
    let hevc = if let Some(png) = cursor_png {
        cursor_stream(
            &identity,
            tablet,
            display,
            &args[4],
            VideoCodec::Hevc,
            &native_transport,
            png,
            std::path::Path::new(&args[2]),
            &commands,
        )?
    } else {
        stream(
            &identity,
            tablet,
            display,
            &args[4],
            VideoCodec::Hevc,
            args.get(6).map(String::as_str),
            &native_transport,
        )?
    };
    let mut receipt = serde_json::json!({"status":"passed","peer":identity.fingerprint(),"h264Sent":avc,"hevcSent":hevc,"signaling_route":"lan","cursor_checked":cursor_png.is_some()});
    #[cfg(target_os = "macos")]
    if let Some(host) = usb.as_ref() {
        receipt["usb"] = host.snapshot();
    }
    fs::write(&args[2], receipt.to_string())?;
    Ok(())
}
