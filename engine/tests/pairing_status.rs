use extend_computer_agent::{
    identity::Identity,
    session::{pairing_status, serve_connection, Decision},
    trust::TrustStore,
};
use std::{
    net::{TcpListener, TcpStream},
    sync::Arc,
};
fn check(known: bool, correct_pin: bool) {
    let server = Arc::new(Identity::generate());
    let caller = Identity::generate();
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path()).unwrap();
    if known {
        store.remember(&caller.fingerprint()).unwrap();
    }
    let before = std::fs::read(dir.path().join("trust.json")).ok();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let expected = if correct_pin {
        server.fingerprint()
    } else {
        Identity::generate().fingerprint()
    };
    let server_store = store.clone();
    let thread = std::thread::spawn(move || {
        serve_connection(
            listener.accept().unwrap().0,
            &server,
            &server_store,
            &mut None,
            |_, _| -> Decision { panic!("read-only status requested consent") },
        )
    });
    let result = pairing_status(TcpStream::connect(address).unwrap(), &caller, &expected);
    let _ = thread.join().unwrap();
    if correct_pin {
        assert_eq!(result.unwrap(), known);
    } else {
        assert!(result.is_err());
    }
    assert_eq!(before, std::fs::read(dir.path().join("trust.json")).ok());
}
#[test]
fn paired_status_does_not_change_trust_or_request_control() {
    check(true, true);
}
#[test]
fn unpaired_status_does_not_change_trust_or_request_control() {
    check(false, true);
}
#[test]
fn wrong_identity_cannot_report_unpaired() {
    check(false, false);
}
#[test]
fn socket_failure_is_not_unpaired() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || drop(listener.accept().unwrap()));
    let result = pairing_status(
        TcpStream::connect(address).unwrap(),
        &Identity::generate(),
        &Identity::generate().fingerprint(),
    );
    thread.join().unwrap();
    assert!(result.is_err());
}
