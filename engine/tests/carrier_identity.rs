//! Private USB/relay streams must authenticate the selected peer before any
//! platform or peer-metadata operation, even if another identity is trusted.
use extend_computer_agent::{
    error::EngineError,
    identity::Identity,
    session::{
        self,
        display::{Config, DisplaySignal, Mode, VideoCodec},
        Client, CursorSink, Decision, Presence,
    },
    trust::TrustStore,
};
use std::{
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

struct Sink(Arc<AtomicBool>);
impl CursorSink for Sink {
    fn start_control(&mut self, _: &str) -> anyhow::Result<bool> {
        self.0.store(true, Ordering::Release);
        Ok(true)
    }
    fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn unpaired(&mut self, _: &str) -> anyhow::Result<()> {
        self.0.store(true, Ordering::Release);
        Ok(())
    }
}
fn assert_changed(result: anyhow::Result<()>) {
    assert_eq!(
        result.unwrap_err().downcast_ref::<EngineError>(),
        Some(&EngineError::PeerIdentityChanged)
    );
}

#[test]
fn pinned_private_control_rejects_other_trusted_peer_before_grant_or_unpair() {
    for unpair in [false, true] {
        let selected = Identity::generate();
        let other = Identity::generate();
        let receiver = Identity::generate();
        let pin = receiver.fingerprint();
        let root = tempfile::tempdir().unwrap();
        let trust = TrustStore::open(root.path()).unwrap();
        let client_root = tempfile::tempdir().unwrap();
        let client_trust = TrustStore::open(client_root.path()).unwrap();
        client_trust.remember(&pin).unwrap();
        trust.remember(&selected.fingerprint()).unwrap();
        trust.remember(&other.fingerprint()).unwrap();
        let store = trust.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let seen = called.clone();
        let server = thread::spawn(move || {
            session::serve_connection_with_cursor_pinned(
                listener.accept().unwrap().0,
                &receiver,
                &store,
                &selected.fingerprint(),
                &mut Sink(seen),
            )
        });
        if unpair {
            assert!(
                session::notify_unpair(TcpStream::connect(address).unwrap(), &other, &pin).is_err()
            );
        } else {
            assert!(Client::connect(
                TcpStream::connect(address).unwrap(),
                &other,
                &client_trust,
                None,
                Some(&pin),
                |_, _| Decision::Once
            )
            .is_err());
        }
        assert_changed(server.join().unwrap());
        assert!(!called.load(Ordering::Acquire));
        assert!(trust.peer(&other.fingerprint()).unwrap().is_some());
    }
}

#[test]
fn pinned_private_presence_rejects_other_trusted_peer_before_status() {
    let selected = Identity::generate();
    let other = Identity::generate();
    let receiver = Identity::generate();
    let pin = receiver.fingerprint();
    let root = tempfile::tempdir().unwrap();
    let trust = TrustStore::open(root.path()).unwrap();
    trust.remember(&other.fingerprint()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let called = Arc::new(AtomicBool::new(false));
    let seen = called.clone();
    let server = thread::spawn(move || {
        session::serve_presence_pinned(
            listener.accept().unwrap().0,
            &receiver,
            &trust,
            &selected.fingerprint(),
            || {
                seen.store(true, Ordering::Release);
                Presence {
                    receiving: true,
                    port: 48177,
                    name: "Receiver".into(),
                }
            },
        )
    });
    assert!(session::query_presence(TcpStream::connect(address).unwrap(), &other, &pin).is_err());
    assert_changed(server.join().unwrap());
    assert!(!called.load(Ordering::Acquire));
}

#[test]
fn pinned_private_display_accepts_selected_peer_and_rejects_trusted_substitution() {
    for correct_peer in [false, true] {
        let source = Identity::generate();
        let receiver = Identity::generate();
        let selected = if correct_peer {
            source.fingerprint()
        } else {
            Identity::generate().fingerprint()
        };
        let pin = receiver.fingerprint();
        let root = tempfile::tempdir().unwrap();
        let trust = TrustStore::open(root.path()).unwrap();
        trust.remember(&source.fingerprint()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let seen = called.clone();
        let server = thread::spawn(move || {
            DisplaySignal::accept_with_settings_features_pinned(
                listener.accept().unwrap().0,
                &receiver,
                &trust,
                &selected,
                true,
                |_, _, offer, _| {
                    seen.store(true, Ordering::Release);
                    assert_eq!(offer.len(), 16_384);
                    Ok("answer".repeat(2048))
                },
            )
            .map(|_| ())
        });
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
        let result = DisplaySignal::connect(
            TcpStream::connect(address).unwrap(),
            &source,
            &pin,
            &config,
            &"o".repeat(16_384),
        );
        let accepted = server.join().unwrap();
        assert_eq!(called.load(Ordering::Acquire), correct_peer);
        assert_eq!(result.is_ok(), correct_peer);
        if correct_peer {
            accepted.unwrap();
        } else {
            assert_changed(accepted);
        }
    }
}
