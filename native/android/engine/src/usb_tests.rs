//! Isolated host verification of native adapters over the real accessory framing.
use super::*;
use extend_computer_agent::{
    display::{DisplayEvent, Frame},
    input::InputEvent,
    session::{
        display::{Config, DisplaySignal, Mode},
        CursorSink,
    },
};
use std::sync::{atomic::AtomicUsize, mpsc};

fn wait(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Native USB adapter did not progress"
        );
        thread::sleep(Duration::from_millis(2));
    }
}
struct Sink(Arc<AtomicUsize>);
impl CursorSink for Sink {
    fn supports_text_input(&self) -> bool {
        true
    }
    fn start_cursor(&mut self, _: &str) -> Result<bool> {
        Ok(true)
    }
    fn start_control(&mut self, _: &str) -> Result<bool> {
        Ok(true)
    }
    fn display_size(&self) -> Result<session::DisplaySize> {
        Ok(session::DisplaySize {
            width: 640.,
            height: 480.,
        })
    }
    fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
        Ok(())
    }
    fn input(&mut self, event: &InputEvent) -> Result<()> {
        if matches!(event, InputEvent::Text { .. }) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
    fn finish(&mut self) {}
}
struct Fixture {
    id: String,
    host: Arc<UsbHub>,
    stop: Arc<AtomicBool>,
    pump: Option<thread::JoinHandle<()>>,
    dispatch: Option<thread::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        super::stop(&self.id);
        self.host.close();
        self.stop.store(true, Ordering::Release);
        if let Some(mut incoming) = crate::RECEIVING.lock().unwrap().take() {
            incoming.stop();
        }
        if let Some(server) = crate::MEDIA.lock().unwrap().take() {
            server.stop();
        }
        if let Some(worker) = self.pump.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.dispatch.take() {
            let _ = worker.join();
        }
    }
}
#[test]
#[ignore = "Owns process-wide USB/RECEIVING/MEDIA; run alone with --ignored --test-threads=1"]
fn native_usb_no_lan_pins_both_directions_media_input_and_retires_epoch() {
    assert!(CONNECTION.lock().unwrap().is_none());
    assert!(crate::RECEIVING.lock().unwrap().is_none());
    assert!(crate::MEDIA.lock().unwrap().is_none());
    let files = tempfile::tempdir().unwrap();
    let tablet = Arc::new(Identity::generate());
    let host_identity = Arc::new(Identity::generate());
    let substituted = Identity::generate();
    let host_peer = host_identity.fingerprint();
    let tablet_peer = tablet.fingerprint();
    let trust = TrustStore::open(files.path().join("tablet")).unwrap();
    trust.remember(&host_peer).unwrap();
    trust.remember(&substituted.fingerprint()).unwrap();
    let host_trust = TrustStore::open(files.path().join("host")).unwrap();
    host_trust.remember(&tablet_peer).unwrap();
    let names = Arc::new(crate::names::Names::open(files.path()).unwrap());
    let receiver = Arc::new(crate::receiver::Receiver::default());
    receiver.configure(true, 640., 480.).unwrap();
    receiver.configure_text(true);
    receiver.drain();
    *crate::RECEIVING.lock().unwrap() = Some(
        crate::incoming::Incoming::receiving(
            tablet.clone(),
            trust.clone(),
            "Tablet".into(),
            names.clone(),
            receiver.clone(),
        )
        .unwrap(),
    );
    let server = Arc::new(
        crate::media::Server::start(
            tablet.clone(),
            trust.clone(),
            serde_json::json!({"h264":[{"width":640,"height":480,"fps":30}]}),
        )
        .unwrap(),
    );
    *crate::MEDIA.lock().unwrap() = Some(server.clone());
    let (host, mut driver) =
        UsbHub::new("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()).unwrap();
    host.enable_streams().unwrap();
    let id = attach(
        host_peer.clone(),
        tablet.clone(),
        trust.clone(),
        names.clone(),
    )
    .unwrap();
    let initial = route(&host_peer).unwrap().unwrap();
    assert_eq!(
        selected(&host_peer, "invalid LAN hint")
            .unwrap()
            .unwrap()
            .id,
        id
    );
    assert!(selected(&host_peer, "usb:99999999").is_err());
    assert!(initial
        .open(
            UsbService::Control,
            &AtomicBool::new(true),
            Instant::now() + Duration::from_secs(1)
        )
        .is_err());
    let stop = Arc::new(AtomicBool::new(false));
    let pump_stop = stop.clone();
    let pump_id = id.clone();
    let pump = thread::spawn(move || {
        while !pump_stop.load(Ordering::Acquire) {
            for _ in 0..64 {
                let Some(bytes) = (match outgoing(&pump_id) {
                    Ok(value) => value,
                    Err(_) => return,
                }) else {
                    break;
                };
                let mut offset = 0;
                let deadline = Instant::now() + Duration::from_secs(2);
                while offset < bytes.len() {
                    if pump_stop.load(Ordering::Acquire) {
                        return;
                    }
                    match driver.receive_bytes(&bytes[offset..]) {
                        Ok(0) => {
                            assert!(Instant::now() < deadline);
                            thread::sleep(Duration::from_millis(1));
                        }
                        Ok(n) => offset += n,
                        Err(_) => return,
                    }
                }
            }
            for _ in 0..64 {
                let Some(bytes) = (match driver.take_outgoing() {
                    Ok(value) => value,
                    Err(_) => return,
                }) else {
                    break;
                };
                let mut offset = 0;
                let deadline = Instant::now() + Duration::from_secs(2);
                while offset < bytes.len() {
                    if pump_stop.load(Ordering::Acquire) {
                        return;
                    }
                    match receive(&pump_id, &bytes[offset..]) {
                        Ok(0) => {
                            assert!(Instant::now() < deadline);
                            thread::sleep(Duration::from_millis(1));
                        }
                        Ok(n) => offset += n,
                        Err(_) => return,
                    }
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    });
    let host_for_dispatch = host.clone();
    let dispatch_stop = stop.clone();
    let host_for_identity = host_identity.clone();
    let trust_for_dispatch = host_trust.clone();
    let expected = tablet_peer.clone();
    let text = Arc::new(AtomicUsize::new(0));
    let text_for_dispatch = text.clone();
    let (frames, received_frames) = mpsc::channel();
    let dispatch = thread::spawn(move || {
        while !dispatch_stop.load(Ordering::Acquire) {
            match host_for_dispatch.accept_stream() {
                Ok(Some((service, socket))) => {
                    let identity = host_for_identity.clone();
                    let trust = trust_for_dispatch.clone();
                    let expected = expected.clone();
                    let hub = host_for_dispatch.clone();
                    let text = text_for_dispatch.clone();
                    let frames = frames.clone();
                    thread::spawn(move || match service {
                        UsbService::Presence => {
                            let _ = session::serve_presence_pinned(
                                socket,
                                &identity,
                                &trust,
                                &expected,
                                &|| Presence {
                                    receiving: true,
                                    port: 48177,
                                    name: "Host".into(),
                                },
                            );
                        }
                        UsbService::Control => {
                            let _ = session::serve_connection_with_cursor_pinned(
                                socket,
                                &identity,
                                &trust,
                                &expected,
                                &mut Sink(text),
                            );
                        }
                        UsbService::Display => {
                            let mut transport = Transport::with_packet_link(
                                VideoCodec::H264,
                                Box::new(hub.endpoint().unwrap()),
                            )
                            .unwrap();
                            let Ok((mut signal, _, _)) =
                                DisplaySignal::accept_with_settings_features_pinned(
                                    socket,
                                    &identity,
                                    &trust,
                                    &expected,
                                    false,
                                    |_, _, offer, features| {
                                        transport.answer_with_features(offer, features)
                                    },
                                )
                            else {
                                return;
                            };
                            let heartbeat =
                                thread::spawn(move || while signal.respond().unwrap_or(false) {});
                            let deadline = Instant::now() + Duration::from_secs(5);
                            loop {
                                if let Ok(Some(DisplayEvent::Frame(frame))) = transport.poll() {
                                    frames.send(frame.bytes).unwrap();
                                    break;
                                }
                                if Instant::now() >= deadline {
                                    return;
                                }
                                thread::sleep(Duration::from_millis(1));
                            }
                            drop(transport);
                            let _ = heartbeat.join();
                        }
                    });
                }
                Ok(None) => thread::sleep(Duration::from_millis(1)),
                Err(_) => return,
            }
        }
    });
    let fixture = Fixture {
        id: id.clone(),
        host: host.clone(),
        stop,
        pump: Some(pump),
        dispatch: Some(dispatch),
    };
    wait(|| host.streams_ready() && snapshot()["streams_ready"] == true);
    let presence = session::query_presence(
        host.open_stream(
            UsbService::Presence,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap(),
        &host_identity,
        &tablet_peer,
    )
    .unwrap()
    .unwrap();
    assert!(presence.receiving);
    let presence = session::query_presence(
        initial
            .open(
                UsbService::Presence,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(3),
            )
            .unwrap(),
        &tablet,
        &host_peer,
    )
    .unwrap()
    .unwrap();
    assert!(presence.receiving);
    let wrong = host
        .open_stream(
            UsbService::Control,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap();
    assert!(Client::connect(
        wrong,
        &substituted,
        &host_trust,
        None,
        Some(&tablet_peer),
        |_, _| Decision::Deny
    )
    .is_err());
    assert!(!receiver.active());
    assert_eq!(names.snapshot().unwrap(), "{}");
    assert_eq!(names.hosts().unwrap(), "{}");
    assert!(
        trust.peer(&host_peer).unwrap().is_some()
            && trust.peer(&substituted.fingerprint()).unwrap().is_some()
    );
    let socket = host
        .open_stream(
            UsbService::Control,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap();
    let mut control = Client::connect(
        socket,
        &host_identity,
        &host_trust,
        None,
        Some(&tablet_peer),
        |_, _| Decision::Deny,
    )
    .unwrap();
    control.request_control().unwrap();
    control
        .queue_input(InputEvent::Button {
            button: 0,
            down: true,
            clicks: 1,
        })
        .unwrap();
    control.probe().unwrap();
    assert!(receiver.active());
    let conflicting = host
        .open_stream(
            UsbService::Control,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap();
    assert!(Client::connect(
        conflicting,
        &host_identity,
        &host_trust,
        None,
        Some(&tablet_peer),
        |_, _| Decision::Deny
    )
    .is_err());
    assert!(receiver.active());
    let started = Instant::now();
    crate::RECEIVING.lock().unwrap().take().unwrap().stop();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!receiver.active());
    assert!(control.probe().is_err());
    let status = session::query_presence(
        host.open_stream(
            UsbService::Presence,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap(),
        &host_identity,
        &tablet_peer,
    )
    .unwrap()
    .unwrap();
    assert!(!status.receiving);
    let sender = crate::control::Sender::start_routed(
        tablet.clone(),
        trust.clone(),
        host_peer.clone(),
        format!("usb:{id}"),
        Arc::new(Default::default()),
    )
    .unwrap();
    wait(|| sender.snapshot()["state"] == "connected");
    assert_eq!(sender.snapshot()["route"], "usb");
    let control_id = sender.snapshot()["id"].as_str().unwrap().to_owned();
    sender
        .enqueue(
            &control_id,
            &serde_json::json!({"kind":"text","value":"漢字 👩‍💻 é"}).to_string(),
        )
        .unwrap();
    wait(|| text.load(Ordering::Relaxed) == 1);
    sender.cancel(&control_id);
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
    let capture = crate::sender::Sender::start_routed(
        tablet.clone(),
        trust.clone(),
        host_peer.clone(),
        format!("usb:{id}"),
        config.clone(),
    )
    .unwrap();
    wait(|| capture.snapshot()["state"] == "streaming");
    assert_eq!(capture.snapshot()["route"], "usb");
    let capture_id = capture.snapshot()["id"].as_str().unwrap().to_owned();
    let payload = vec![0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21];
    capture
        .enqueue(
            &capture_id,
            Frame {
                bytes: payload.clone(),
                timestamp: 1_000_000_000,
                keyframe: true,
            },
        )
        .unwrap();
    assert_eq!(
        received_frames
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        payload
    );
    capture.cancel(&capture_id);
    let source_identity = host_identity.clone();
    let expected = tablet_peer.clone();
    let source_hub = host.clone();
    let (finish, finished) = mpsc::channel();
    let source = thread::spawn(move || {
        let socket = source_hub
            .open_stream(
                UsbService::Display,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(3),
            )
            .unwrap();
        let mut transport =
            Transport::with_packet_link(VideoCodec::H264, Box::new(source_hub.endpoint().unwrap()))
                .unwrap();
        let offer = transport.offer().unwrap();
        let (mut signal, _, answer) =
            DisplaySignal::connect(socket, &source_identity, &expected, &config, &offer).unwrap();
        transport.accept_answer(&answer).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut written = false;
        while Instant::now() < deadline {
            if let Some(DisplayEvent::Connected) = transport.poll().unwrap() {
                written = true;
                assert!(transport
                    .write(Frame {
                        bytes: payload.clone(),
                        timestamp: 2_000_000_000,
                        keyframe: true
                    })
                    .unwrap());
            }
            if written && finished.try_recv().is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        signal.close();
    });
    wait(|| server.snapshot()["state"] == "offered");
    let display_id = server.snapshot()["id"].as_str().unwrap().to_owned();
    assert_eq!(server.snapshot()["route"], "usb");
    server.attach(&display_id).unwrap();
    wait(|| server.snapshot()["received"].as_u64().unwrap_or(0) > 0);
    let frame = server.take_frame(&display_id).unwrap().unwrap();
    assert_eq!(&frame[17..], &[0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21]);
    finish.send(()).unwrap();
    source.join().unwrap();
    initial.save_name(&names, "Host fresh").unwrap();
    let commit = initial.commit.lock().unwrap();
    let late_route = initial.clone();
    let late_names = names.clone();
    let late_save = thread::spawn(move || late_route.save_name(&late_names, "Stale host"));
    let retired_id = id.clone();
    let retired = thread::spawn(move || super::stop(&retired_id));
    wait(|| snapshot()["state"] == "disconnected");
    drop(commit);
    assert!(late_save.join().unwrap().is_err());
    retired.join().unwrap();
    names.record_removal(&host_peer).unwrap();
    assert!(initial.save_name(&names, "Stale host").is_err());
    assert_eq!(names.snapshot().unwrap(), "{}");
    assert_eq!(
        names.removals().unwrap().get(&host_peer).unwrap(),
        "Host fresh"
    );
    drop(fixture);
    assert!(!initial.current());
    assert!(selected(&host_peer, &format!("usb:{id}")).is_err());
    let replacement = attach(host_peer.clone(), tablet, trust.clone(), names).unwrap();
    let current = route(&host_peer).unwrap().unwrap();
    super::stop(&id);
    assert!(current.current());
    assert!(!initial.current());
    trust.complete_unpair(&host_peer).unwrap();
    wait(|| snapshot()["state"] == "disconnected");
    assert!(!current.current());
    super::stop(&replacement);
}
