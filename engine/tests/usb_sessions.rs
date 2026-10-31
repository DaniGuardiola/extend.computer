//! Exercise real pinned session protocols through USB framing, with no LAN
//! listener. The only TCP sockets are the carrier's private in-process pairs.
use extend_computer_agent::{
    display::{packet_link::PacketDriver, usb_link::UsbHub, usb_stream::UsbService},
    identity::Identity,
    input::InputEvent,
    session::{
        self,
        display::{Config, DisplaySignal, Mode, VideoCodec},
        Client, CursorSink, Decision, Presence,
    },
    trust::TrustStore,
};
use std::{
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

struct Cable {
    a: Arc<UsbHub>,
    b: Arc<UsbHub>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
fn transfer(source: &mut PacketDriver, target: &mut PacketDriver) {
    for _ in 0..64 {
        let Some(frame) = source.take_outgoing().unwrap() else {
            break;
        };
        for fragment in frame.chunks(37) {
            assert_eq!(target.receive_bytes(fragment).unwrap(), fragment.len());
        }
    }
}
impl Cable {
    fn new() -> Self {
        let (a, mut ad) =
            UsbHub::new("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()).unwrap();
        let (b, mut bd) =
            UsbHub::new("192.0.2.2".parse().unwrap(), "192.0.2.1".parse().unwrap()).unwrap();
        a.enable_streams().unwrap();
        b.enable_streams().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let halted = stop.clone();
        let worker = thread::spawn(move || {
            while !halted.load(Ordering::Acquire) {
                transfer(&mut ad, &mut bd);
                transfer(&mut bd, &mut ad);
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            a,
            b,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Cable {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        self.a.close();
        self.b.close();
    }
}
fn accept(hub: &UsbHub, expected: UsbService) -> TcpStream {
    let end = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some((service, socket)) = hub.accept_stream().unwrap() {
            assert_eq!(service, expected);
            return socket;
        }
        assert!(Instant::now() < end, "USB service did not arrive");
        thread::sleep(Duration::from_millis(1));
    }
}
#[derive(Default)]
struct Observed {
    granted: bool,
    events: Vec<InputEvent>,
    finished: bool,
}
struct Sink(Arc<Mutex<Observed>>);
impl CursorSink for Sink {
    fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn start_control(&mut self, _: &str) -> anyhow::Result<bool> {
        self.0.lock().unwrap().granted = true;
        Ok(true)
    }
    fn supports_text_input(&self) -> bool {
        true
    }
    fn input(&mut self, event: &InputEvent) -> anyhow::Result<()> {
        self.0.lock().unwrap().events.push(event.clone());
        Ok(())
    }
    fn finish(&mut self) {
        self.0.lock().unwrap().finished = true;
    }
}

#[test]
fn pinned_presence_unicode_control_and_large_display_work_in_both_usb_directions() {
    for reverse in [false, true] {
        let cable = Cable::new();
        let (out, incoming) = if reverse {
            (&cable.b, &cable.a)
        } else {
            (&cable.a, &cable.b)
        };
        let source = Identity::generate();
        let receiver = Identity::generate();
        let source_pin = source.fingerprint();
        let target_pin = receiver.fingerprint();
        let client_dir = tempfile::tempdir().unwrap();
        let server_dir = tempfile::tempdir().unwrap();
        let clients = TrustStore::open(client_dir.path()).unwrap();
        let servers = TrustStore::open(server_dir.path()).unwrap();
        clients.remember(&target_pin).unwrap();
        servers.remember(&source_pin).unwrap();
        let observed = Arc::new(Mutex::new(Observed::default()));
        let seen = observed.clone();
        let server_hub = incoming.clone();
        let server = thread::spawn(move || {
            session::serve_presence_pinned(
                accept(&server_hub, UsbService::Presence),
                &receiver,
                &servers,
                &source_pin,
                || Presence {
                    receiving: true,
                    port: 48177,
                    name: "Cable receiver".into(),
                },
            )
            .unwrap();
            session::serve_connection_with_cursor_pinned(
                accept(&server_hub, UsbService::Control),
                &receiver,
                &servers,
                &source_pin,
                &mut Sink(seen),
            )
            .unwrap();
            let (mut signal, _, _) = DisplaySignal::accept_with_settings_features_pinned(
                accept(&server_hub, UsbService::Display),
                &receiver,
                &servers,
                &source_pin,
                false,
                |_, _, sdp, _| {
                    assert_eq!(sdp, "o".repeat(16_384));
                    Ok("a".repeat(16_384))
                },
            )
            .unwrap();
            assert!(signal.respond().unwrap());
            assert!(!signal.respond().unwrap());
        });
        let open = |service| {
            out.open_stream(
                service,
                &cable.stop,
                Instant::now() + Duration::from_secs(4),
            )
            .unwrap()
        };
        let status = session::query_presence(open(UsbService::Presence), &source, &target_pin)
            .unwrap()
            .unwrap();
        assert!(status.receiving);
        assert_eq!(status.name, "Cable receiver");
        let mut client = Client::connect(
            open(UsbService::Control),
            &source,
            &clients,
            None,
            Some(&target_pin),
            |_, _| Decision::Deny,
        )
        .unwrap();
        client.request_control().unwrap();
        assert!(client.supports_text_input());
        let text = InputEvent::Text {
            value: "Café 中文 👩‍💻".into(),
        };
        client.queue_input(text.clone()).unwrap();
        client.probe().unwrap();
        client.close().unwrap();
        let config = Config {
            mode: Mode::Mirror,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 10_000_000,
            retina: false,
            auto_size: false,
            codec: VideoCodec::H264,
            separate_cursor: false,
        };
        let (mut signal, selected, answer) = DisplaySignal::connect(
            open(UsbService::Display),
            &source,
            &target_pin,
            &config,
            &"o".repeat(16_384),
        )
        .unwrap();
        assert_eq!(selected, config);
        assert_eq!(answer, "a".repeat(16_384));
        signal.pulse(0).unwrap();
        signal.close();
        server.join().unwrap();
        let seen = observed.lock().unwrap();
        assert!(seen.granted && seen.finished);
        assert!(seen.events.contains(&text));
    }
}
