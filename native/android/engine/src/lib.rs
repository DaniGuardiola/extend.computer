//! Shared mobile engine. Cryptography and trust use the shared desktop engine;
//! `api` is the platform-neutral surface, `jni_exports` the Android JNI boundary.
//! Java owns Keystore access; private seed bytes never cross the React bridge.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::session::{Client, Decision};
use extend_computer_agent::{identity::Identity, trust::TrustStore};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use zeroize::{Zeroize, Zeroizing};

struct Runtime {
    identity: Arc<Identity>,
    trust: TrustStore,
    root: PathBuf,
    names: Arc<names::Names>,
    account: accounts::Membership,
    relay: Arc<extend_computer_agent::relay::Relay>,
}
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
mod accounts;
pub mod api;
mod control;
mod cursor_compositor;
mod device_info;
mod incoming;
#[cfg(target_os = "android")]
mod jni_exports;
mod manualpairing;
mod media;
mod names;
mod presence;
mod receiver;
mod relay;
mod sender;
mod unpair;
mod usb;
mod visualpairing;
static MANUAL_PAIRING: Mutex<Option<manualpairing::ManualPairing>> = Mutex::new(None);
static VISUAL_PAIRING: Mutex<Option<visualpairing::VisualPairing>> = Mutex::new(None);
static SENDER: Mutex<Option<Arc<sender::Sender>>> = Mutex::new(None);
static SENDER_START: Mutex<()> = Mutex::new(());
static INCOMING: Mutex<Option<incoming::Incoming>> = Mutex::new(None);
static RECEIVING: Mutex<Option<incoming::Incoming>> = Mutex::new(None);
static MEDIA: Mutex<Option<Arc<media::Server>>> = Mutex::new(None);
static CONTROL: Mutex<Option<Arc<control::Sender>>> = Mutex::new(None);
static CONTROL_START: Mutex<()> = Mutex::new(());
static PRESENCE: Mutex<Option<presence::Server>> = Mutex::new(None);

// Availability checks do not reserve a control/display session. Pairing UI and
// approval also recheck these native states; a running display is never stopped.
fn visual_pairing_idle() -> bool {
    pairing_session_idle()
        && MANUAL_PAIRING
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| !s.active())
}
fn pairing_session_idle() -> bool {
    fn idle(value: serde_json::Value) -> bool {
        matches!(
            value["state"].as_str(),
            Some("idle" | "closed" | "failed" | "disconnected")
        )
    }
    !receiver::shared().active()
        && CONTROL
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| idle(s.snapshot()))
        && SENDER
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| idle(s.snapshot()))
        && MEDIA
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| idle(s.snapshot()))
}

fn endpoint(input: &str) -> Result<SocketAddr> {
    let address: SocketAddr = input.parse().context("Enter an IP address and port")?;
    ensure!(
        address.port() != 0 && !address.ip().is_unspecified() && !address.ip().is_multicast(),
        "Invalid device address"
    );
    Ok(address)
}

fn pair_code(runtime: &Runtime, address: &str, code: &str) -> Result<String> {
    let code = normalize_pair_code(code)?;
    let stream = TcpStream::connect_timeout(&endpoint(address)?, Duration::from_secs(5))
        .context("Could not reach this device")?;
    // Explicit code entry authorizes this PAKE-authenticated pairing, as on desktop.
    let client = Client::connect(
        stream,
        &runtime.identity,
        &runtime.trust,
        Some(&code),
        None,
        |_, _| Decision::Remember,
    )?;
    let peer = client.peer().to_owned();
    client.close()?;
    Ok(peer)
}
fn normalize_pair_code(code: &str) -> Result<Zeroizing<String>> {
    ensure!(code.len() <= 128, "Invalid pairing code");
    let digits = Zeroizing::new(
        code.chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect::<String>(),
    );
    ensure!(
        digits.len() == 16 && digits.bytes().all(|b| b.is_ascii_hexdigit()),
        "Enter the 16-character code shown on the other device"
    );
    Ok(Zeroizing::new(
        digits
            .as_bytes()
            .chunks(4)
            .map(|part| std::str::from_utf8(part).unwrap().to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("-"),
    ))
}

fn probe_peer(runtime: &Runtime, address: &str, peer: &str) -> Result<String> {
    ensure!(runtime.trust.peer(peer)?.is_some(), "Unknown device");
    let stream = TcpStream::connect_timeout(&endpoint(address)?, Duration::from_secs(5))
        .context("Could not reach this device")?;
    let mut client = Client::connect(
        stream,
        &runtime.identity,
        &runtime.trust,
        None,
        Some(peer),
        |_, _| Decision::Deny,
    )?;
    let latency = client.probe()?.as_secs_f64() * 1000.0;
    client.close()?;
    Ok(serde_json::json!({"peer":peer,"rtt_ms":latency}).to_string())
}

fn resolve_presence(runtime: &Runtime, address: &str, peer: &str) -> Result<String> {
    ensure!(runtime.trust.peer(peer)?.is_some(), "Unknown device");
    let mut address = endpoint(address)?;
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))
        .context("Could not reach this device's availability service")?;
    let status = extend_computer_agent::session::query_presence(stream, &runtime.identity, peer)?
        .context("This device no longer recognizes the pairing")?;
    ensure!(runtime.trust.peer(peer)?.is_some(), "Device was forgotten");
    // Only this pinned response establishes the current receiving endpoint.
    runtime.names.save(peer, &status.name)?;
    address.set_port(status.port);
    Ok(
        serde_json::json!({"peer":peer, "receiving":status.receiving,
        "name":status.name, "address":address.to_string()})
        .to_string(),
    )
}

fn public_identity(runtime: &Runtime) -> String {
    serde_json::json!({"fingerprint": runtime.identity.fingerprint(),
        "public_key": hex::encode(runtime.identity.public())})
    .to_string()
}

fn initialize(mut seed: Zeroizing<Vec<u8>>, root: PathBuf) -> Result<String> {
    ensure!(seed.len() == 32, "Invalid platform identity seed length");
    ensure!(
        root.is_absolute(),
        "Engine storage must be an absolute app-private path"
    );
    let mut bytes = [0; 32];
    bytes.copy_from_slice(&seed);
    let identity = Identity::from_secret(bytes);
    bytes.zeroize();
    seed.zeroize();
    let account = accounts::Membership::default();
    let trust = TrustStore::open(root.join("trust"))?.with_account_trust(account.trust.clone());
    let expected_root = root.clone();
    let expected_identity = identity.fingerprint();
    let runtime = Runtime {
        identity: Arc::new(identity),
        trust,
        names: Arc::new(names::Names::open(&root)?),
        account,
        relay: Arc::new(Default::default()),
        root,
    };
    let _ = RUNTIME.set(runtime);
    let current = RUNTIME.get().context("Engine initialization failed")?;
    ensure!(
        current.root == expected_root && current.identity.fingerprint() == expected_identity,
        "Engine identity or storage changed during process lifetime"
    );
    Ok(public_identity(current))
}

fn unpair_local(runtime: &Runtime, peer: &str) -> Result<()> {
    ensure!(
        peer.len() == 64 && hex::decode(peer)?.len() == 32,
        "Expected full device fingerprint"
    );
    // Commit revocation first. Account directory refresh must not restore access.
    runtime.trust.complete_unpair(peer)?;
    accounts::cancel_retired(
        runtime,
        std::collections::BTreeMap::from([(peer.to_owned(), String::new())]),
    )?;
    let accessory = usb::snapshot();
    if accessory["peer"].as_str() == Some(peer) {
        if let Some(id) = accessory["id"].as_str() {
            usb::stop(id);
        }
    }
    runtime.names.forget(peer)?;
    Ok(())
}

fn media_server() -> Result<Arc<media::Server>> {
    MEDIA.lock().unwrap().clone().context("Receiving disabled")
}

fn control_sender() -> Result<Arc<control::Sender>> {
    CONTROL
        .lock()
        .unwrap()
        .clone()
        .context("Control is not active")
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::{pairing::PairingWindow, session};
    use std::net::TcpListener;
    use std::thread;

    fn runtime(root: PathBuf) -> Runtime {
        Runtime {
            account: accounts::Membership::default(),
            relay: Arc::new(Default::default()),
            identity: Arc::new(Identity::generate()),
            trust: TrustStore::open(root.join("trust")).unwrap(),
            names: Arc::new(names::Names::open(&root).unwrap()),
            root,
        }
    }

    #[test]
    fn local_unpair_removes_manual_and_blocks_account_refresh_without_touching_other_peer() {
        let files = tempfile::tempdir().unwrap();
        let mut android = runtime(files.path().join("android"));
        android.trust = android
            .trust
            .clone()
            .with_account_trust(android.account.trust.clone());
        let manual = Identity::generate().fingerprint();
        let account = Identity::generate().fingerprint();
        let retained = Identity::generate().fingerprint();
        android.trust.remember(&manual).unwrap();
        android.trust.remember(&retained).unwrap();
        android.account.trust.replace(
            "server/account",
            &[account.clone()],
            Duration::from_secs(90),
        );
        for peer in [&manual, &account, &retained] {
            android.names.save(peer, "Paired device").unwrap();
            android
                .names
                .save_host(peer, "127.0.0.1".parse().unwrap())
                .unwrap();
        }
        assert!(unpair_local(&android, "invalid-peer").is_err());
        assert!(android.trust.peer(&manual).unwrap().is_some());
        unpair_local(&android, &manual).unwrap();
        assert!(android.trust.peer(&manual).unwrap().is_none());
        assert!(android.trust.peer(&account).unwrap().is_some());
        unpair_local(&android, &account).unwrap();
        android.account.trust.replace(
            "server/account",
            &[account.clone()],
            Duration::from_secs(90),
        );
        assert!(
            android.trust.peer(&account).unwrap().is_none(),
            "Fresh directory must not undo unpair"
        );
        assert!(android.trust.peer(&retained).unwrap().is_some());
        let names: serde_json::Value =
            serde_json::from_str(&android.names.snapshot().unwrap()).unwrap();
        let hosts: serde_json::Value =
            serde_json::from_str(&android.names.hosts().unwrap()).unwrap();
        assert!(names.get(&manual).is_none() && names.get(&account).is_none());
        assert!(hosts.get(&manual).is_none() && hosts.get(&account).is_none());
        assert_eq!(names[&retained], "Paired device");
        assert_eq!(hosts[&retained], "127.0.0.1");
        let restored = TrustStore::open(android.root.join("trust"))
            .unwrap()
            .with_account_trust(android.account.trust.clone());
        assert!(
            restored.peer(&account).unwrap().is_none(),
            "Account block survives reopen"
        );
        assert!(restored.peer(&retained).unwrap().is_some());
    }

    #[test]
    fn authenticated_unpair_hint_removes_remote_metadata_and_keeps_listener() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        let own = android.identity.fingerprint();
        let retained = Identity::generate().fingerprint();
        android.trust.remember(&peer).unwrap();
        remote.trust.remember(&own).unwrap();
        remote.trust.remember(&retained).unwrap();
        remote.names.record_removal(&retained).unwrap();
        assert!(
            remote.names.removals().unwrap().is_empty(),
            "Unknown metadata must not create unsolicited notices"
        );
        remote.names.save(&own, "Android tablet").unwrap();
        remote
            .names
            .save_host(&own, "127.0.0.1".parse().unwrap())
            .unwrap();
        let mut listener = incoming::Incoming::named(
            remote.identity.clone(),
            remote.trust.clone(),
            None,
            "Mac".into(),
            remote.names.clone(),
        )
        .unwrap();
        let address = format!(
            "127.0.0.1:{}",
            listener.snapshot()["port"].as_u64().unwrap()
        );
        unpair_local(&android, &peer).unwrap();
        let id = unpair::record(&peer);
        unpair::notify(&android, peer.clone(), address, id).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while (!remote.names.removals().unwrap().contains_key(&own)
            || remote.names.snapshot().unwrap().contains(&own)
            || remote.names.hosts().unwrap().contains(&own))
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(remote.trust.peer(&own).unwrap().is_none());
        assert_eq!(
            remote
                .names
                .removals()
                .unwrap()
                .get(&own)
                .map(String::as_str),
            Some("Android tablet")
        );
        assert!(!remote.names.snapshot().unwrap().contains(&own));
        assert!(!remote.names.hosts().unwrap().contains(&own));
        assert!(remote.trust.peer(&retained).unwrap().is_some());
        assert_ne!(listener.snapshot()["state"], "closed");
        let restored = names::Names::open(&remote.root).unwrap();
        assert!(restored.removals().unwrap().contains_key(&own));
        // A new authenticated name after pairing clears the old removal notice.
        remote.trust.remember(&own).unwrap();
        remote.names.save(&own, "Android tablet").unwrap();
        assert!(!remote.names.removals().unwrap().contains_key(&own));
        listener.stop();
    }

    #[test]
    fn unpair_hint_rejects_stale_and_repaired_operations() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let peer = Identity::generate().fingerprint();
        let old = unpair::record(&peer);
        let newer = unpair::record(&peer);
        assert!(unpair::notify(&android, peer.clone(), "127.0.0.1:9".into(), old).is_err());
        android.trust.remember(&peer).unwrap();
        assert!(unpair::notify(&android, peer.clone(), "127.0.0.1:9".into(), newer).is_err());
        assert!(android.trust.peer(&peer).unwrap().is_some());
    }

    #[test]
    fn unpair_hint_deadline_closes_stalled_peer() {
        use std::io::Read;
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let peer = Identity::generate().fingerprint();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0u8; 4096];
            loop {
                match socket.read(&mut bytes) {
                    Ok(0) => {
                        tx.send(()).unwrap();
                        break;
                    }
                    Ok(_) => {}
                    Err(error) => panic!("Unpair deadline did not close socket: {error}"),
                }
            }
        });
        let start = std::time::Instant::now();
        unpair::notify(&android, peer.clone(), address, unpair::record(&peer)).unwrap();
        rx.recv_timeout(Duration::from_secs(4)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(4));
        server.join().unwrap();
    }

    #[test]
    fn local_name_updates_live_authenticated_exchange_without_restarting_listener() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        android.trust.remember(&peer).unwrap();
        remote
            .trust
            .remember(&android.identity.fingerprint())
            .unwrap();
        let mut listener = incoming::Incoming::named(
            remote.identity.clone(),
            remote.trust.clone(),
            None,
            "Original".into(),
            remote.names.clone(),
        )
        .unwrap();
        let port = listener.snapshot()["port"].as_u64().unwrap();
        remote.names.set_local("Updated tablet").unwrap();
        assert!(remote.names.set_local("\n").is_err());
        let name = session::exchange_device_name(
            TcpStream::connect(("127.0.0.1", port as u16)).unwrap(),
            &android.identity,
            &peer,
            "Client",
        )
        .unwrap();
        assert_eq!(name, "Updated tablet");
        assert_eq!(listener.snapshot()["port"].as_u64().unwrap(), port);
        listener.stop();
    }

    #[test]
    fn presence_resolution_pins_identity_repairs_port_and_preserves_trust_on_failure() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        android.trust.remember(&peer).unwrap();
        remote
            .trust
            .remember(&android.identity.fingerprint())
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let _ = session::serve_presence(
                    listener.accept().unwrap().0,
                    &remote.identity,
                    &remote.trust,
                    || session::Presence {
                        receiving: true,
                        port: 54321,
                        name: "Authenticated Desktop".into(),
                    },
                );
            }
        });
        let status: serde_json::Value =
            serde_json::from_str(&resolve_presence(&android, &address, &peer).unwrap()).unwrap();
        assert_eq!(status["address"], "127.0.0.1:54321");
        assert_eq!(status["name"], "Authenticated Desktop");
        let wrong = Identity::generate().fingerprint();
        android.trust.remember(&wrong).unwrap();
        assert!(resolve_presence(&android, &address, &wrong).is_err());
        server.join().unwrap();
        let names: serde_json::Value =
            serde_json::from_str(&android.names.snapshot().unwrap()).unwrap();
        assert!(names.get(&wrong).is_none());
        assert!(android.trust.peer(&peer).unwrap().is_some());
        assert!(android.trust.peer(&wrong).unwrap().is_some());
        android
            .names
            .save_host(&peer, "127.0.0.1".parse().unwrap())
            .unwrap();
        android.names.forget(&peer).unwrap();
        assert_eq!(android.names.hosts().unwrap(), "{}");
        assert_eq!(
            names::Names::open(&android.root).unwrap().hosts().unwrap(),
            "{}"
        );
        android.trust.forget(&peer).unwrap();
        assert!(resolve_presence(&android, &address, &peer).is_err());
    }

    #[test]
    fn presence_resolution_preserves_ipv6_scope_when_repairing_port() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        android.trust.remember(&peer).unwrap();
        remote
            .trust
            .remember(&android.identity.fingerprint())
            .unwrap();
        let listener = TcpListener::bind("[::1]:0").unwrap();
        let mut address = listener.local_addr().unwrap();
        let SocketAddr::V6(ref mut v6) = address else {
            panic!("IPv6 listener expected")
        };
        v6.set_scope_id(7);
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            let socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "Presence client did not connect"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Presence accept failed: {error}"),
                }
            };
            session::serve_presence(socket, &remote.identity, &remote.trust, || {
                session::Presence {
                    receiving: true,
                    port: 54322,
                    name: "Scoped Desktop".into(),
                }
            })
            .unwrap();
        });
        let result = resolve_presence(&android, &address.to_string(), &peer);
        server.join().unwrap();
        let status: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(status["address"], "[::1%7]:54322");
        assert_eq!(status["name"], "Scoped Desktop");
        assert!(android.trust.peer(&peer).unwrap().is_some());
    }

    #[test]
    fn real_pair_reconnect_wrong_code_wrong_identity_and_forget() {
        let scratch = tempfile::tempdir().unwrap();
        let android = runtime(scratch.path().join("android"));
        let desktop = runtime(scratch.path().join("desktop"));
        let server_peer = desktop.identity.fingerprint();
        let tablet_peer = android.identity.fingerprint();
        let server_store = desktop.trust.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let window = PairingWindow::new(Duration::from_secs(120));
        let code = window.code().unwrap().to_string();
        let server = thread::spawn(move || {
            let mut window = Some(window);
            let mut accepted = vec![];
            for _ in 0..3 {
                let (socket, _) = listener.accept().unwrap();
                accepted.push(
                    session::serve_connection(
                        socket,
                        &desktop.identity,
                        &desktop.trust,
                        &mut window,
                        |_, _| Decision::Remember,
                    )
                    .is_ok(),
                );
            }
            accepted
        });
        let peer = pair_code(
            &android,
            &address,
            &code.replace('-', "").to_ascii_uppercase(),
        )
        .unwrap();
        assert_eq!(peer, server_peer);
        assert!(server_store.peer(&tablet_peer).unwrap().is_some());
        assert!(probe_peer(&android, &address, &peer).is_ok());
        let wrong_peer = Identity::generate().fingerprint();
        android.trust.remember(&wrong_peer).unwrap();
        assert!(probe_peer(&android, &address, &wrong_peer).is_err());
        android.trust.forget(&peer).unwrap();
        assert!(probe_peer(&android, &address, &peer).is_err());
        assert_eq!(server.join().unwrap(), [true, true, false]);

        let rejected = runtime(scratch.path().join("rejected"));
        let remote = runtime(scratch.path().join("wrong-code-remote"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mut window = Some(PairingWindow::new(Duration::from_secs(120)));
        let server = thread::spawn(move || {
            session::serve_connection(
                listener.accept().unwrap().0,
                &remote.identity,
                &remote.trust,
                &mut window,
                |_, _| Decision::Remember,
            )
            .is_err()
        });
        assert!(pair_code(&rejected, &address, "ffff-ffff-ffff-ffff").is_err());
        assert!(rejected.trust.load().unwrap().peers.is_empty());
        assert!(server.join().unwrap());
    }
}
