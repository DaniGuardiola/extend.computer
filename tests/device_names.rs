use extend_computer_agent::{
    identity::Identity,
    session::{
        exchange_device_name, serve_connection_with_cursor, validate_device_name, CursorSink,
        Decision,
    },
    trust::TrustStore,
};
use std::{
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
};
struct NamedSink {
    name: String,
    received: Arc<Mutex<Vec<(String, String)>>>,
}
impl CursorSink for NamedSink {
    fn approve(&mut self, _: &str) -> anyhow::Result<bool> {
        panic!("metadata requested control")
    }
    fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
        panic!("metadata injected input")
    }
    fn device_name(&self) -> Option<String> {
        Some(self.name.clone())
    }
    fn peer_name(&mut self, peer: &str, name: &str) -> anyhow::Result<()> {
        self.received
            .lock()
            .unwrap()
            .push((peer.into(), name.into()));
        Ok(())
    }
}
fn exchange(known: bool, correct_pin: bool, remote_name: &str) {
    let host = Identity::generate();
    let caller = Identity::generate();
    let caller_id = caller.fingerprint();
    let expected = if correct_pin {
        host.fingerprint()
    } else {
        Identity::generate().fingerprint()
    };
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path()).unwrap();
    if known {
        store.remember(&caller_id, false).unwrap();
    }
    let before = std::fs::read(dir.path().join("trust.json")).ok();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut sink = NamedSink {
        name: remote_name.into(),
        received: received.clone(),
    };
    let thread = std::thread::spawn(move || {
        serve_connection_with_cursor(
            listener.accept().unwrap().0,
            &host,
            &store,
            &mut None,
            |_, _| -> Decision { panic!("metadata requested consent") },
            &mut sink,
        )
    });
    let result = exchange_device_name(
        TcpStream::connect(address).unwrap(),
        &caller,
        &expected,
        "Dani’s MacBook",
    );
    let _ = thread.join().unwrap();
    if known && correct_pin && validate_device_name(remote_name).is_ok() {
        assert_eq!(result.unwrap(), remote_name);
        assert_eq!(
            *received.lock().unwrap(),
            vec![(caller_id, "Dani’s MacBook".into())]
        );
    } else {
        assert!(result.is_err());
        assert!(received.lock().unwrap().is_empty());
    }
    assert_eq!(before, std::fs::read(dir.path().join("trust.json")).ok());
}
#[test]
fn names_exchange_without_input_or_trust_changes() {
    exchange(true, true, "Other Mac");
}
#[test]
fn unknown_peer_cannot_exchange_names() {
    exchange(false, true, "Other Mac");
}
#[test]
fn wrong_pin_never_sends_our_name() {
    exchange(true, false, "Other Mac");
}
#[test]
fn malformed_remote_name_rejected_before_metadata_changes() {
    exchange(true, true, "bad\nname");
}
#[test]
fn names_are_bounded_and_nonempty() {
    for name in ["", "  ", "bad\0name", &"a".repeat(101)] {
        assert!(validate_device_name(name).is_err());
    }
    assert!(validate_device_name("Dani’s MacBook").is_ok());
}
