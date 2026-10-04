use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{pair_visually, serve_connection_with_verification, CursorSink, Decision},
    trust::TrustStore,
};
use std::{
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};
struct NoInput;
impl CursorSink for NoInput {
    fn approve(&mut self, _: &str) -> anyhow::Result<bool> {
        panic!("pairing requested control")
    }
    fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
        panic!("pairing injected input")
    }
}
fn scenario(local: bool, remote: bool, window: Option<Duration>) {
    let a = Identity::generate();
    let b = Identity::generate();
    let aid = a.fingerprint();
    let bid = b.fingerprint();
    let ad = tempfile::tempdir().unwrap();
    let bd = tempfile::tempdir().unwrap();
    let at = TrustStore::open(ad.path()).unwrap();
    let bt = TrustStore::open(bd.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server_store = bt.clone();
    let observed = Arc::new(Mutex::new(None));
    let server_observed = observed.clone();
    let thread = std::thread::spawn(move || {
        let mut window = window.map(PairingWindow::new);
        let result = serve_connection_with_verification(
            listener.accept().unwrap().0,
            &b,
            &server_store,
            &mut window,
            |_, _| -> Decision { panic!("visual pairing used manual approval") },
            &mut NoInput,
            |peer, symbols| {
                assert_eq!(peer, aid);
                assert!(server_store.peer(peer).unwrap().is_none());
                *server_observed.lock().unwrap() = Some(*symbols);
                remote
            },
        );
        if let Some(window) = window {
            assert!(window.code().is_none());
        }
        result
    });
    let mut own = None;
    let result = pair_visually(
        TcpStream::connect(address).unwrap(),
        &a,
        &at,
        |peer, symbols| {
            assert_eq!(peer, bid);
            assert!(at.peer(peer).unwrap().is_none());
            own = Some(*symbols);
            local
        },
    );
    let server_result = thread.join().unwrap();
    let succeeds = local && remote && window.is_some_and(|d| !d.is_zero());
    assert_eq!(result.is_ok(), succeeds);
    assert_eq!(server_result.is_ok(), succeeds);
    assert_eq!(own, *observed.lock().unwrap());
    if succeeds {
        let peer = at.peer(&bid).unwrap().unwrap();
        assert!(!peer.automatic_input && !peer.automatic_probe);
        let peer = bt.peer(&a.fingerprint()).unwrap().unwrap();
        assert!(!peer.automatic_input && !peer.automatic_probe);
    } else {
        assert!(at.peer(&bid).unwrap().is_none());
        assert!(bt.peer(&a.fingerprint()).unwrap().is_none());
    }
}
#[test]
fn both_match_before_trust_and_no_control() {
    scenario(true, true, Some(Duration::from_secs(30)));
}
#[test]
fn sender_mismatch_neither_trusts() {
    scenario(false, true, Some(Duration::from_secs(30)));
}
#[test]
fn receiver_mismatch_neither_trusts() {
    scenario(true, false, Some(Duration::from_secs(30)));
}
#[test]
fn closed_window_rejects_visual_pairing() {
    scenario(true, true, None);
}
#[test]
fn expired_window_rejects_visual_pairing() {
    scenario(true, true, Some(Duration::ZERO));
}

#[test]
fn forgetting_clears_grants_allows_pairing_and_preserves_explicit_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path()).unwrap();
    let peer = Identity::generate().fingerprint();
    store.remember(&peer, true).unwrap();
    store.allow_control(&peer).unwrap();
    store.forget(&peer).unwrap();
    assert!(store.peer(&peer).unwrap().is_none());
    store.remember(&peer, false).unwrap();
    let permissions = store.peer(&peer).unwrap().unwrap();
    assert!(!permissions.automatic_input && !permissions.automatic_probe);
    store.revoke(&peer).unwrap();
    store.forget(&peer).unwrap();
    assert!(store.is_revoked(&peer).unwrap());
    assert!(store.remember(&peer, false).is_err());
}
