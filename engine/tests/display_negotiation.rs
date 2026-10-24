use extend_computer_agent::{
    display::{input::Message, DisplayEvent, Frame, Transport},
    identity::Identity,
    session::display::{Config, DisplaySignal, Mode, VideoCodec},
    trust::TrustStore,
};
use std::{
    net::{IpAddr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

#[test]
fn authenticated_features_open_input_channel_without_interrupting_video() {
    let source = Identity::generate();
    let receiver = Identity::generate();
    let peer = receiver.fingerprint();
    let files = tempfile::tempdir().unwrap();
    let store = TrustStore::open(files.path()).unwrap();
    store.remember(&source.fingerprint()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();
    let config = Config {
        mode: Mode::Mirror,
        width: 1280,
        height: 800,
        fps: 60,
        bitrate: 8_000_000,
        retina: false,
        auto_size: false,
        codec: VideoCodec::H264,
        separate_cursor: true,
    };
    let server = std::thread::spawn(move || {
        let mut media = Transport::new(ip).unwrap();
        let (signal, _, features) = DisplaySignal::accept_with_features(
            listener.accept().unwrap().0,
            &receiver,
            &store,
            |_, _, offer, features| media.answer_with_features(offer, features),
        )
        .unwrap();
        assert!(features.input_enabled());
        (media, signal)
    });
    let mut sender = Transport::new(ip).unwrap();
    let (_source_signal, _, answer, features) = DisplaySignal::connect_with_offer(
        TcpStream::connect(address).unwrap(),
        &source,
        &peer,
        &config,
        |features| sender.offer_with_features(features),
    )
    .unwrap();
    assert!(features.input_enabled());
    let (mut receiver, _receiver_signal) = server.join().unwrap();
    sender.accept_answer(&answer).unwrap();
    let request = Message::Request { request: 7 };
    let mut payload = vec![0, 0, 0, 1, 0x65];
    payload.extend(vec![0x88; 8_000]);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request_sent = false;
    let mut request_received = false;
    let mut frame_sent = false;
    let mut frame_received = false;
    while !(request_received && frame_received) {
        match sender.poll().unwrap() {
            Some(DisplayEvent::Connected) if !frame_sent => {
                assert!(sender
                    .write(Frame {
                        timestamp: 1_000_000_000,
                        keyframe: true,
                        bytes: payload.clone(),
                    })
                    .unwrap());
                frame_sent = true;
            }
            Some(DisplayEvent::Input(message)) => {
                assert_eq!(message, request);
                request_received = true;
            }
            _ => {}
        }
        if let Some(DisplayEvent::Frame(frame)) = receiver.poll().unwrap() {
            assert!(frame.keyframe);
            assert_eq!(frame.bytes, payload);
            frame_received = true;
        }
        if !request_sent {
            request_sent = receiver.write_input(&request).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "Negotiated input or video did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
