use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{serve_connection, Client, Decision},
    trust::TrustStore,
    wire,
};
use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

struct Device {
    identity: Arc<Identity>,
    trust: TrustStore,
    _dir: tempfile::TempDir,
}
impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self {
            identity: Arc::new(Identity::generate()),
            trust: TrustStore::open(dir.path()).unwrap(),
            _dir: dir,
        }
    }
    fn server(
        &self,
        mut window: Option<PairingWindow>,
        decision: Decision,
        count: usize,
    ) -> (SocketAddr, JoinHandle<Vec<bool>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let identity = self.identity.clone();
        let trust = self.trust.clone();
        let task = thread::spawn(move || {
            (0..count)
                .map(|_| {
                    let stream = listener.accept().unwrap().0;
                    serve_connection(stream, &identity, &trust, &mut window, |_, _| decision)
                        .is_ok()
                })
                .collect()
        });
        (address, task)
    }
    fn connect(
        &self,
        address: SocketAddr,
        code: Option<&str>,
        peer: Option<&str>,
        decision: Decision,
    ) -> anyhow::Result<Client> {
        Client::connect(
            TcpStream::connect(address)?,
            &self.identity,
            &self.trust,
            code,
            peer,
            |_, _| decision,
        )
    }
}

fn window() -> (PairingWindow, String) {
    let window = PairingWindow::new(Duration::from_secs(30));
    let code = window.code().unwrap().to_owned();
    (window, code)
}

#[test]
fn pair_probe_and_reconnect_with_pinned_identities() {
    let server = Device::new();
    let client = Device::new();
    let (window, code) = window();
    let (address, task) = server.server(Some(window), Decision::Remember, 2);
    let mut session = client
        .connect(address, Some(&code), None, Decision::Remember)
        .unwrap();
    let peer = session.peer().to_owned();
    session.probe().unwrap();
    session.close().unwrap();
    // Deny callback demonstrates that automatic permission was explicitly persisted.
    let mut session = client
        .connect(address, None, Some(&peer), Decision::Deny)
        .unwrap();
    session.probe().unwrap();
    session.close().unwrap();
    assert_eq!(task.join().unwrap(), vec![true, true]);
}

#[test]
fn wrong_code_burns_window_and_never_prompts() {
    let server = Device::new();
    let client = Device::new();
    let (window, code) = window();
    let (address, task) = server.server(Some(window), Decision::Remember, 2);
    assert!(client
        .connect(address, Some("wrong"), None, Decision::Remember)
        .is_err());
    assert!(client
        .connect(address, Some(&code), None, Decision::Remember)
        .is_err());
    assert!(server.trust.load().unwrap().peers.is_empty());
    assert_eq!(task.join().unwrap(), vec![false, false]);
}

#[test]
fn expired_window_cannot_pair() {
    let server = Device::new();
    let client = Device::new();
    let window = PairingWindow::new(Duration::ZERO);
    let code = window.code().unwrap().to_owned();
    let (address, task) = server.server(Some(window), Decision::Remember, 1);
    assert!(client
        .connect(address, Some(&code), None, Decision::Remember)
        .is_err());
    assert_eq!(task.join().unwrap(), vec![false]);
}

#[test]
fn known_code_does_not_bypass_host_consent() {
    let server = Device::new();
    let client = Device::new();
    let (window, code) = window();
    let (address, task) = server.server(Some(window), Decision::Deny, 1);
    let error = client
        .connect(address, Some(&code), None, Decision::Remember)
        .err()
        .expect("consent must be denied");
    assert_eq!(
        error.downcast_ref::<extend_computer_agent::error::EngineError>(),
        Some(&extend_computer_agent::error::EngineError::RemoteConsentDenied)
    );
    assert!(client.trust.load().unwrap().peers.is_empty());
    assert!(server.trust.load().unwrap().peers.is_empty());
    assert_eq!(task.join().unwrap(), vec![false]);
}

#[test]
fn client_can_decline_authenticated_pairing() {
    let server = Device::new();
    let client = Device::new();
    let (window, code) = window();
    let (address, task) = server.server(Some(window), Decision::Remember, 1);
    let error = client
        .connect(address, Some(&code), None, Decision::Deny)
        .err()
        .expect("consent must be denied");
    assert_eq!(
        error.downcast_ref::<extend_computer_agent::error::EngineError>(),
        Some(&extend_computer_agent::error::EngineError::LocalConsentDenied)
    );
    assert!(server.trust.load().unwrap().peers.is_empty());
    assert_eq!(task.join().unwrap(), vec![false]);
}

#[test]
fn unknown_client_cannot_reconnect_even_if_it_trusts_server() {
    let server = Device::new();
    let client = Device::new();
    let peer = server.identity.fingerprint();
    client.trust.remember(&peer).unwrap();
    let (address, task) = server.server(None, Decision::Remember, 1);
    assert!(client
        .connect(address, None, Some(&peer), Decision::Remember)
        .is_err());
    // Server successfully reports NotPaired; the client still rejects control.
    assert_eq!(task.join().unwrap(), vec![true]);
}

#[test]
fn one_off_grant_disappears_after_disconnect() {
    let server = Device::new();
    let client = Device::new();
    let (window, code) = window();
    let (address, task) = server.server(Some(window), Decision::Once, 2);
    let session = client
        .connect(address, Some(&code), None, Decision::Remember)
        .unwrap();
    let peer = session.peer().to_owned();
    session.close().unwrap();
    assert!(server.trust.load().unwrap().peers.is_empty());
    assert!(client
        .connect(address, None, Some(&peer), Decision::Remember)
        .is_err());
    // Server successfully reports NotPaired; the client still rejects control.
    assert_eq!(task.join().unwrap(), vec![true, true]);
}

#[test]
fn paired_devices_connect_without_another_approval() {
    let server = Device::new();
    let client = Device::new();
    let peer = server.identity.fingerprint();
    client.trust.remember(&peer).unwrap();
    server
        .trust
        .remember(&client.identity.fingerprint())
        .unwrap();
    let (address, task) = server.server(None, Decision::Deny, 1);
    let mut connection = client
        .connect(address, None, Some(&peer), Decision::Deny)
        .unwrap();
    connection.probe().unwrap();
    connection.close().unwrap();
    assert_eq!(task.join().unwrap(), vec![true]);
}

#[test]
fn changed_server_identity_fails_pin_check() {
    let server = Device::new();
    let client = Device::new();
    let old_peer = Identity::generate().fingerprint();
    client.trust.remember(&old_peer).unwrap();
    server
        .trust
        .remember(&client.identity.fingerprint())
        .unwrap();
    let (address, task) = server.server(None, Decision::Remember, 1);
    let error = client
        .connect(address, None, Some(&old_peer), Decision::Remember)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<extend_computer_agent::error::EngineError>(),
        Some(&extend_computer_agent::error::EngineError::PeerIdentityChanged)
    );
    assert_eq!(task.join().unwrap(), vec![false]);
}

#[test]
fn revoke_terminates_active_session_and_prevents_reconnect() {
    let server = Device::new();
    let client = Device::new();
    let (window, code) = window();
    let (address, task) = server.server(Some(window), Decision::Remember, 2);
    let mut session = client
        .connect(address, Some(&code), None, Decision::Remember)
        .unwrap();
    let peer = session.peer().to_owned();
    session.probe().unwrap();
    server.trust.revoke(&client.identity.fingerprint()).unwrap();
    assert!(session.probe().is_err());
    drop(session);
    assert!(client
        .connect(address, None, Some(&peer), Decision::Remember)
        .is_err());
    assert_eq!(task.join().unwrap(), vec![false, false]);
}

#[test]
fn invalid_or_truncated_frames_fail_closed() {
    for bytes in [vec![0, 0], vec![255, 255], vec![0, 4, 1, 2]] {
        assert!(wire::read_frame(&mut &bytes[..]).is_err());
    }
}

#[test]
fn corrupt_trust_store_does_not_reset_permissions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("trust.json"), b"{garbage").unwrap();
    assert!(TrustStore::open(dir.path()).is_err());
}
