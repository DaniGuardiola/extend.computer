use extend_computer_agent::{
    identity::Identity,
    session::{notify_unpair, serve_connection, Decision},
    trust::TrustStore,
};
use std::{
    net::{TcpListener, TcpStream},
    sync::Arc,
};

fn request(
    server: Arc<Identity>,
    store: TrustStore,
    caller: &Identity,
    expected: &str,
) -> (bool, bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        serve_connection(
            listener.accept().unwrap().0,
            &server,
            &store,
            &mut None,
            |_, _| -> Decision { panic!("unpair must not request probe or input consent") },
        )
        .is_ok()
    });
    let client = notify_unpair(TcpStream::connect(address).unwrap(), caller, expected).is_ok();
    (client, thread.join().unwrap())
}
#[test]
fn authenticated_unpair_is_scoped_to_sender_and_idempotent() {
    let server = Arc::new(Identity::generate());
    let caller = Identity::generate();
    let other = Identity::generate();
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path()).unwrap();
    store.remember(&caller.fingerprint()).unwrap();
    store.remember(&other.fingerprint()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            request(
                server.clone(),
                store.clone(),
                &caller,
                &server.fingerprint()
            ),
            (true, true)
        );
        assert!(store.peer(&caller.fingerprint()).unwrap().is_none());
        assert!(store.peer(&other.fingerprint()).unwrap().is_some());
    }
}
#[test]
fn unknown_identity_cannot_remove_another_device() {
    let server = Arc::new(Identity::generate());
    let stranger = Identity::generate();
    let known = Identity::generate();
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path()).unwrap();
    store.remember(&known.fingerprint()).unwrap();
    assert_eq!(
        request(
            server.clone(),
            store.clone(),
            &stranger,
            &server.fingerprint()
        ),
        (true, true)
    );
    assert!(store.peer(&known.fingerprint()).unwrap().is_some());
}
#[test]
fn changed_address_identity_never_receives_unpair_request() {
    let server = Arc::new(Identity::generate());
    let caller = Identity::generate();
    let expected = Identity::generate();
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path()).unwrap();
    store.remember(&caller.fingerprint()).unwrap();
    assert_eq!(
        request(server, store.clone(), &caller, &expected.fingerprint()),
        (false, false)
    );
    assert!(store.peer(&caller.fingerprint()).unwrap().is_some());
}
#[test]
fn new_pairing_clears_pending_notification_and_old_grants() {
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path()).unwrap();
    let id = Identity::generate().fingerprint();
    store.remember(&id).unwrap();
    store.begin_unpair(&id).unwrap();
    assert!(store.peer(&id).unwrap().is_none());
    assert!(store.load().unwrap().pending_unpairs.contains(&id));
    store.remember(&id).unwrap();
    assert!(store.load().unwrap().pending_unpairs.is_empty());
    assert!(store.peer(&id).unwrap().is_some());
}
