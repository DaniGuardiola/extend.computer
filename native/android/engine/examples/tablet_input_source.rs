//! Ephemeral pinned input source for a disposable Android native target.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    error::EngineError,
    identity::Identity,
    input::InputEvent,
    session::{self, Client, Decision},
    trust::TrustStore,
};
use serde_json::{json, Value};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

// Public fixed text: precomposed/decomposed accents, CJK, surrogate pairs, ZWJ
// sequences and a skin-tone modifier. No arbitrary user text input.
const UNICODE_TEXT: &str = "Café é — 中文测试 👩‍💻 🧑🏽‍🚀";

fn save_receipt(path: &Path, value: &Value) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Receipt parent required")?)?;
    #[cfg(unix)]
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
struct Diagnostics<'a> {
    path: &'a Path,
    started: Instant,
}
impl Diagnostics<'_> {
    fn stage(&self, receipt: &mut Value, stage: &'static str) -> Result<()> {
        let elapsed = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        receipt["stage"] = json!(stage);
        receipt["elapsed_ms"] = json!(elapsed);
        receipt["clock"] = json!("fixture-process-monotonic-ms");
        if !receipt["stage_history"].is_array() {
            receipt["stage_history"] = json!([]);
        }
        let history = receipt["stage_history"].as_array_mut().unwrap();
        if history.len() < 32 {
            history.push(json!({"stage":stage,"elapsed_ms":elapsed}));
        }
        save_receipt(self.path, receipt)
    }
    fn failed(&self, receipt: &mut Value, error: &anyhow::Error) -> Result<()> {
        receipt["failure_stage"] = receipt["stage"].clone();
        receipt["state"] = json!("failed");
        // Preserve only typed classifications, never arbitrary error text or the code.
        receipt["error_kind"] = error
            .chain()
            .find_map(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .map(|io| format!("{:?}", io.kind()))
            })
            .map_or(Value::Null, |kind| json!(kind));
        receipt["error_category"] = json!(match error.downcast_ref::<EngineError>() {
            Some(EngineError::RemoteConsentDenied) => "remote-consent-denied",
            Some(EngineError::LocalConsentDenied) => "local-consent-denied",
            Some(EngineError::PeerIdentityChanged) => "peer-identity-changed",
            Some(EngineError::PeerUnpaired) => "peer-unpaired",
            Some(EngineError::PairingFailed) => "pairing-authentication-failed",
            Some(EngineError::AuthenticationFailed) => "encrypted-authentication-failed",
            Some(EngineError::ProtocolIncompatible) => "protocol-incompatible",
            Some(_) => "protocol-or-policy-rejected",
            None if !receipt["error_kind"].is_null() => "io-failure",
            None => "unclassified-fixture-failure",
        });
        self.stage(receipt, "failed")
    }
}
fn pairing_connect(
    address: SocketAddr,
    code: &str,
    identity: &Identity,
    trust: &TrustStore,
    peer: &str,
    diagnostics: &Diagnostics<'_>,
    receipt: &mut Value,
) -> Result<Client> {
    receipt["selected_pairing_address"] = json!(address.to_string());
    diagnostics.stage(receipt, "pairing-tcp-connect")?;
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(3))
        .context("Pairing TCP connection failed")?;
    receipt["pairing_tcp_connected"] = json!(true);
    diagnostics.stage(receipt, "pairing-handshake-ready")?;
    let mut callback_error = None;
    let connected = Client::connect(
        socket,
        identity,
        trust,
        Some(code),
        Some(peer),
        |authenticated, _| {
            // This callback is reached only after PAKE, Noise, Hello and authenticated Ready.
            // It precedes RequestProbe send, so the next boundary is request/grant combined.
            receipt["authenticated_ready_observed"] = json!(true);
            receipt["authenticated_peer"] = json!(authenticated);
            if let Err(error) = diagnostics.stage(receipt, "pairing-request-grant") {
                callback_error = Some(error);
                Decision::Deny
            } else {
                Decision::Remember
            }
        },
    );
    if let Some(error) = callback_error {
        return Err(error.context("Authenticated pairing receipt failed"));
    }
    let client = connected.context(if receipt["authenticated_ready_observed"] == true {
        "Authenticated pairing request/grant failed"
    } else {
        "Pairing PAKE/Noise/Hello/Ready failed"
    })?;
    receipt["pairing_remote_grant_observed"] = json!(true);
    diagnostics.stage(receipt, "pairing-granted")?;
    Ok(client)
}
fn command_sequence(command: &Value, last: u64) -> Result<u64> {
    let next = if let Some(value) = command.get("sequence") {
        value.as_u64().context("Invalid command sequence")?
    } else {
        last.checked_add(1).context("Command sequence exhausted")?
    };
    ensure!(next > last, "Stale command sequence");
    Ok(next)
}
fn recovery_grant(mut candidate: Client) -> Result<Option<(Client, session::DisplaySize)>> {
    match candidate.request_control() {
        Ok(display) => Ok(Some((candidate, display))),
        Err(error)
            if matches!(
                error.downcast_ref::<EngineError>(),
                Some(EngineError::RemoteConsentDenied)
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
fn input_command(client: &mut Client, command: &Value) -> Result<String> {
    let stage = command["kind"].as_str().context("Command required")?;
    match stage {
        "click" | "hold" => {
            client.queue_cursor(
                command["x"].as_f64().context("x required")?,
                command["y"].as_f64().context("y required")?,
            )?;
            client.queue_input(InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            })?;
            if stage == "click" {
                client.queue_input(InputEvent::Button {
                    button: 0,
                    down: false,
                    clicks: 1,
                })?;
            }
        }
        "move" => client.queue_cursor(
            command["x"].as_f64().context("x required")?,
            command["y"].as_f64().context("y required")?,
        )?,
        "scroll" => client.queue_input(InputEvent::Scroll { dx: 0, dy: 320 })?,
        "type" => {
            // Physical keys remain a separate acceptance path from text.
            for code in [14, 7, 17, 14, 45, 2] {
                client.queue_input(InputEvent::MacKey {
                    code,
                    down: true,
                    repeat: false,
                })?;
                client.queue_input(InputEvent::MacKey {
                    code,
                    down: false,
                    repeat: false,
                })?;
                client.probe()?;
            }
        }
        "unicode" => {
            ensure!(
                client.supports_text_input(),
                "Receiver did not grant committed text support"
            );
            client.queue_input(InputEvent::Text {
                value: UNICODE_TEXT.into(),
            })?;
        }
        "release" => client.queue_input(InputEvent::Release)?,
        _ => anyhow::bail!("Unknown test command"),
    }
    client.probe()?;
    Ok(stage.into())
}
fn account_identity(path: &Path) -> Result<Identity> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 65536,
        "Invalid fixture credential file"
    );
    #[cfg(unix)]
    ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "Fixture credentials require mode 0600"
    );
    let bytes = Zeroizing::new(fs::read(path)?);
    let mut credentials: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        credentials["peer_seed_kind"] == "disposable-account-fixture-v1",
        "Disposable fixture seed required"
    );
    ensure!(
        credentials["email"]
            .as_str()
            .is_some_and(|email| email.ends_with("@example.invalid")),
        "Disposable fixture account required"
    );
    let seed = Zeroizing::new(
        credentials["peer_seed"]
            .take()
            .as_str()
            .context("Fixture seed missing")?
            .to_owned(),
    );
    ensure!(seed.len() == 64, "Invalid fixture seed length");
    let mut secret = Zeroizing::new([0u8; 32]);
    hex::decode_to_slice(seed.as_str(), &mut *secret).context("Invalid fixture seed encoding")?;
    let identity = Identity::from_secret(*secret);
    secret.zeroize();
    ensure!(
        credentials["peer_fingerprint"].as_str() == Some(identity.fingerprint().as_str()),
        "Disposable seed/public identity mismatch"
    );
    Ok(identity)
}
// Revocation acceptance must observe the typed handshake denial. A fresh grant,
// a permission denial, or a transport timeout cannot substitute for PeerUnpaired.
fn expect_account_revoked(candidate: Result<Client>) -> Result<()> {
    match candidate {
        Err(error)
            if matches!(
                error.downcast_ref::<EngineError>(),
                Some(EngineError::PeerUnpaired)
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.context("Expected account peer-unpaired rejection")),
        Ok(candidate) => {
            let _ = candidate.close();
            anyhow::bail!("Revoked account peer was still admitted")
        }
    }
}
fn run(fixture_path: &Path, receipt_path: &Path) -> Result<()> {
    let fixture: Value = serde_json::from_slice(&fs::read(fixture_path)?)?;
    let address: SocketAddr = fixture["address"]
        .as_str()
        .context("Address required")?
        .parse()?;
    let peer = fixture["peer"].as_str().context("Tablet pin required")?;
    ensure!(
        peer.len() == 64 && peer.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid tablet pin"
    );
    let account_only = fixture["accountOnly"] == true;
    ensure!(
        !account_only || fixture["lifecycle"] == true,
        "Account-only fixture requires lifecycle mode"
    );
    let identity = if account_only {
        account_identity(Path::new(
            fixture["accountCredentials"]
                .as_str()
                .context("Account fixture credentials required")?,
        ))?
    } else {
        Identity::generate()
    };
    let name = format!("Input test {}", &identity.fingerprint()[..8]);
    let temp = tempfile::tempdir()?;
    let trust = TrustStore::open(temp.path())?;
    if account_only {
        // Trust only the tablet in this ephemeral SOURCE store. Never write
        // Android manual trust, send PAKE, or remember the fixture on Android.
        trust.remember(peer)?;
    }
    let mut receipt = json!({"state":"waiting", "peer":identity.fingerprint(), "name":name, "tablet_peer":peer,
        "text_supported":false, "unicode_fixture":UNICODE_TEXT, "unicode_utf16_units":UNICODE_TEXT.encode_utf16().count(), "command_sequence":0,
        "selected_pairing_address":address.to_string(), "pairing_tcp_connected":false,
        "authenticated_ready_observed":false, "pairing_remote_grant_observed":false, "diagnostic_version":1});
    receipt["mode"] = json!(if account_only {
        "account-only"
    } else {
        "manual"
    });
    receipt["manual_pairing_attempted"] = json!(!account_only);
    let diagnostics = Diagnostics {
        path: receipt_path,
        started: Instant::now(),
    };
    diagnostics.stage(&mut receipt, "initialized")?;
    let outcome = (|| -> Result<()> {
        let connect = |address: SocketAddr, code: Option<&str>| {
            Client::connect(
                TcpStream::connect_timeout(&address, Duration::from_secs(3))?,
                &identity,
                &trust,
                code,
                Some(peer),
                |_, _| {
                    if account_only {
                        Decision::Deny
                    } else {
                        Decision::Remember
                    }
                },
            )
        };
        if account_only {
            receipt["state"] = json!("account-ready");
            diagnostics.stage(&mut receipt, "account-ready-waiting-target")?;
        } else {
            let mut client = pairing_connect(
                address,
                fixture["code"].as_str().context("Code required")?,
                &identity,
                &trust,
                peer,
                &diagnostics,
                &mut receipt,
            )?;
            diagnostics.stage(&mut receipt, "pairing-probe")?;
            client.probe()?;
            diagnostics.stage(&mut receipt, "pairing-close")?;
            client.close()?;
            diagnostics.stage(&mut receipt, "pairing-name-tcp-connect")?;
            let name_socket = TcpStream::connect_timeout(&address, Duration::from_secs(3))
                .context("Paired name-exchange TCP connection failed")?;
            diagnostics.stage(&mut receipt, "pairing-name-exchange")?;
            session::exchange_device_name(name_socket, &identity, peer, &name)?;
            receipt["state"] = json!("paired");
            diagnostics.stage(&mut receipt, "paired-waiting-target")?;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(std::io::stdin()).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let line = rx.recv_timeout(Duration::from_secs(120))??;
        let command: Value = serde_json::from_str(&line)?;
        ensure!(
            command["kind"] == "connect",
            "Wait for the disposable Android target before control"
        );
        let mut sequence = command_sequence(&command, 0)?;
        let presence_address = if let Some(value) = fixture["presenceAddress"].as_str() {
            value.parse()?
        } else {
            SocketAddr::new(address.ip(), 48178)
        };
        receipt["presence_address"] = json!(presence_address.to_string());
        diagnostics.stage(&mut receipt, "presence-tcp-connect")?;
        let presence_socket = TcpStream::connect_timeout(&presence_address, Duration::from_secs(3))
            .context("Pinned presence TCP connection failed")?;
        diagnostics.stage(&mut receipt, "presence-query")?;
        let presence = session::query_presence(presence_socket, &identity, peer)?
            .context("Tablet presence missing")?;
        ensure!(presence.receiving, "Tablet not available");
        receipt["control_address"] =
            json!(SocketAddr::new(address.ip(), presence.port).to_string());
        diagnostics.stage(&mut receipt, "control-connect")?;
        let mut client = connect(SocketAddr::new(address.ip(), presence.port), None)?;
        diagnostics.stage(&mut receipt, "control-request-grant")?;
        let display = client.request_control()?;
        receipt["state"] = json!("connected");
        receipt["command_sequence"] = json!(sequence);
        receipt["width"] = json!(display.width);
        receipt["height"] = json!(display.height);
        // Actual authenticated adapter grant, not a receiver-version guess.
        receipt["text_supported"] = json!(client.supports_text_input());
        diagnostics.stage(&mut receipt, "control-active")?;
        // Permission-recovery checks retain this disposable identity after a
        // revoked session. Ordinary input acceptance still treats disconnects
        // as terminal failures and never retries a command.
        let lifecycle = fixture["lifecycle"] == true;
        let mut client = Some(client);
        loop {
            let line = match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => line?,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(active) = client.as_mut() {
                        if let Err(error) = active.probe() {
                            if !lifecycle {
                                return Err(error);
                            }
                            client = None;
                            receipt["state"] = json!("disconnected");
                            receipt["text_supported"] = json!(false);
                            diagnostics.stage(&mut receipt, "control-disconnected")?;
                        }
                    }
                    continue;
                }
                Err(_) => break,
            };
            let command: Value = serde_json::from_str(&line)?;
            sequence = command_sequence(&command, sequence)?;
            let stage = command["kind"].as_str().context("Command required")?;
            if stage == "connect" {
                ensure!(
                    lifecycle && client.is_none(),
                    "Unexpected control reconnect"
                );
                diagnostics.stage(&mut receipt, "control-reconnect")?;
                let connection = connect(SocketAddr::new(address.ip(), presence.port), None);
                if account_only {
                    expect_account_revoked(connection)?;
                    receipt["state"] = json!("denied");
                    receipt["denial"] = json!("peer-unpaired");
                    receipt["text_supported"] = json!(false);
                    receipt["command_sequence"] = json!(sequence);
                    diagnostics.stage(&mut receipt, "account-revoked-rejected")?;
                    continue;
                }
                let candidate = connection?;
                match recovery_grant(candidate)? {
                    Some((candidate, display)) => {
                        receipt["state"] = json!("connected");
                        receipt["width"] = json!(display.width);
                        receipt["height"] = json!(display.height);
                        receipt["text_supported"] = json!(candidate.supports_text_input());
                        receipt["denial"] = Value::Null;
                        client = Some(candidate);
                    }
                    None => {
                        receipt["state"] = json!("denied");
                        receipt["text_supported"] = json!(false);
                        receipt["denial"] = json!("remote-consent-denied");
                    }
                }
                receipt["command_sequence"] = json!(sequence);
                save_receipt(receipt_path, &receipt)?;
                continue;
            }
            if stage == "close" {
                if let Some(active) = client.take() {
                    active.close()?;
                }
                receipt["state"] = json!("closed");
                receipt["command_sequence"] = json!(sequence);
                save_receipt(receipt_path, &receipt)?;
                return Ok(());
            }
            input_command(
                client.as_mut().context("Input source disconnected")?,
                &command,
            )?;
            receipt["state"] = json!(stage);
            receipt["command_sequence"] = json!(sequence);
            save_receipt(receipt_path, &receipt)?;
        }
        if let Some(active) = client.take() {
            active.close()?;
        }
        Ok(())
    })();
    if let Err(error) = &outcome {
        diagnostics.failed(&mut receipt, error)?;
    }
    outcome
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 3,
        "Private fixture and receipt paths required"
    );
    run(Path::new(&args[1]), Path::new(&args[2]))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };
    #[test]
    fn disposable_seed_matches_identity_and_rejects_wrong_public_or_file_permissions() {
        let files = tempfile::tempdir().unwrap();
        let path = files.path().join("credentials.json");
        // RFC 7748 X25519 vector also defines the Node JWK d handoff.
        let mut seed = [0u8; 32];
        hex::decode_to_slice(
            "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
            &mut seed,
        )
        .unwrap();
        let identity = Identity::from_secret(seed);
        assert_eq!(
            hex::encode(identity.public()),
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
        );
        let mut value = json!({"email":"fixture@example.invalid", "peer_seed_kind":"disposable-account-fixture-v1", "peer_seed":hex::encode(seed), "peer_fingerprint":identity.fingerprint()});
        save_receipt(&path, &value).unwrap();
        assert_eq!(
            account_identity(&path).unwrap().fingerprint(),
            identity.fingerprint()
        );
        value["peer_fingerprint"] = json!(Identity::generate().fingerprint());
        save_receipt(&path, &value).unwrap();
        assert!(account_identity(&path).is_err());
        value["peer_fingerprint"] = json!(identity.fingerprint());
        save_receipt(&path, &value).unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(account_identity(&path).is_err());
        }
    }
    #[test]
    fn account_revocation_requires_actual_typed_encrypted_handshake_denial() {
        let files = tempfile::tempdir().unwrap();
        let source = Identity::generate();
        let target = Identity::generate();
        let pin = target.fingerprint();
        let client_trust = TrustStore::open(files.path().join("client")).unwrap();
        client_trust.remember(&pin).unwrap();
        // No target trust remains after account revocation. Neither side is
        // permitted to invoke manual Remember or create a new PAKE pairing.
        let account = extend_computer_agent::account_trust::AccountTrust::default();
        account.replace(
            "disposable/account",
            &[source.fingerprint()],
            Duration::from_secs(60),
        );
        let target_trust = TrustStore::open(files.path().join("target"))
            .unwrap()
            .with_account_trust(account.clone());
        let manual_peer = Identity::generate().fingerprint();
        target_trust.remember(&manual_peer).unwrap();
        assert!(target_trust.is_account_peer(&source.fingerprint()).unwrap());
        assert!(TrustStore::open(files.path().join("target"))
            .unwrap()
            .peer(&source.fingerprint())
            .unwrap()
            .is_none());
        account.clear();
        assert!(target_trust.peer(&source.fingerprint()).unwrap().is_none());
        assert!(target_trust.peer(&manual_peer).unwrap().is_some());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            session::serve_connection_with_cursor(
                listener.accept().unwrap().0,
                &target,
                &target_trust,
                &mut None,
                |_, _| Decision::Deny,
                &mut Sink {
                    control: Some(true),
                    text: true,
                    events: Arc::new(Mutex::new(Vec::new())),
                },
            )
        });
        let candidate = Client::connect(
            TcpStream::connect(address).unwrap(),
            &source,
            &client_trust,
            None,
            Some(&pin),
            |_, _| Decision::Deny,
        );
        expect_account_revoked(candidate).unwrap();
        let _ = worker.join().unwrap();
        assert!(expect_account_revoked(Err(EngineError::RemoteConsentDenied.into())).is_err());
    }
    fn pending_receipt() -> Value {
        json!({"state":"waiting", "command_sequence":0, "pairing_tcp_connected":false,
            "authenticated_ready_observed":false, "pairing_remote_grant_observed":false})
    }
    #[test]
    fn tcp_failure_and_connected_handshake_failure_have_distinct_private_receipts() {
        for connected in [false, true] {
            let files = tempfile::tempdir().unwrap();
            let identity = Identity::generate();
            let trust = TrustStore::open(files.path().join("trust")).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let worker = if connected {
                Some(thread::spawn(move || {
                    use std::io::Read;
                    let mut socket = listener.accept().unwrap().0;
                    let mut magic = [0; 9];
                    socket.read_exact(&mut magic).unwrap();
                    // A listener answering TCP is not evidence that it serves PAKE.
                }))
            } else {
                drop(listener);
                None
            };
            let path = files.path().join("receipt.json");
            let diagnostics = Diagnostics {
                path: &path,
                started: Instant::now(),
            };
            let mut receipt = pending_receipt();
            let error = pairing_connect(
                address,
                "1234-abcd-5678-ef90",
                &identity,
                &trust,
                &Identity::generate().fingerprint(),
                &diagnostics,
                &mut receipt,
            )
            .err()
            .unwrap();
            diagnostics.failed(&mut receipt, &error).unwrap();
            let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved["selected_pairing_address"], address.to_string());
            assert_eq!(saved["pairing_tcp_connected"], connected);
            assert_eq!(saved["authenticated_ready_observed"], false);
            assert_eq!(saved["pairing_remote_grant_observed"], false);
            assert_eq!(
                saved["failure_stage"],
                if connected {
                    "pairing-handshake-ready"
                } else {
                    "pairing-tcp-connect"
                }
            );
            assert_eq!(saved["state"], "failed");
            assert!(!saved["error_kind"].is_null());
            assert!(!fs::read_to_string(&path)
                .unwrap()
                .contains("1234-abcd-5678-ef90"));
            #[cfg(unix)]
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            if let Some(worker) = worker {
                worker.join().unwrap();
            }
        }
    }
    #[test]
    fn authenticated_ready_is_recorded_before_remote_grant_or_denial() {
        use extend_computer_agent::pairing::PairingWindow;
        for allow in [false, true] {
            let files = tempfile::tempdir().unwrap();
            let identity = Identity::generate();
            let receiver = Identity::generate();
            let trust = TrustStore::open(files.path().join("client")).unwrap();
            let server_trust = TrustStore::open(files.path().join("server")).unwrap();
            let peer = receiver.fingerprint();
            let mut window = Some(PairingWindow::new(Duration::from_secs(5)));
            let code = window.as_ref().unwrap().code().unwrap().to_owned();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (pending_tx, pending_rx) = mpsc::channel();
            let (decide_tx, decide_rx) = mpsc::channel();
            let server = thread::spawn(move || {
                session::serve_connection(
                    listener.accept().unwrap().0,
                    &receiver,
                    &server_trust,
                    &mut window,
                    |_, _| {
                        pending_tx.send(()).unwrap();
                        if decide_rx.recv().unwrap() {
                            Decision::Remember
                        } else {
                            Decision::Deny
                        }
                    },
                )
            });
            let path = files.path().join("receipt.json");
            let client_path = path.clone();
            let worker = thread::spawn(move || {
                let diagnostics = Diagnostics {
                    path: &client_path,
                    started: Instant::now(),
                };
                let mut receipt = pending_receipt();
                match pairing_connect(
                    address,
                    &code,
                    &identity,
                    &trust,
                    &peer,
                    &diagnostics,
                    &mut receipt,
                ) {
                    Ok(client) => client.close(),
                    Err(error) => {
                        diagnostics.failed(&mut receipt, &error).unwrap();
                        Err(error)
                    }
                }
            });
            pending_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let pending: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(pending["stage"], "pairing-request-grant");
            assert_eq!(pending["authenticated_ready_observed"], true);
            assert_eq!(pending["pairing_remote_grant_observed"], false);
            decide_tx.send(allow).unwrap();
            assert_eq!(worker.join().unwrap().is_ok(), allow);
            assert_eq!(server.join().unwrap().is_ok(), allow);
            let completed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(completed["pairing_remote_grant_observed"], allow);
            assert_eq!(completed["clock"], "fixture-process-monotonic-ms");
            if allow {
                assert_eq!(completed["stage"], "pairing-granted");
            } else {
                assert_eq!(completed["failure_stage"], "pairing-request-grant");
                assert_eq!(completed["error_category"], "remote-consent-denied");
            }
        }
    }
    struct Sink {
        control: Option<bool>,
        text: bool,
        events: Arc<Mutex<Vec<InputEvent>>>,
    }
    impl session::CursorSink for Sink {
        fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
            Ok(())
        }
        fn start_control(&mut self, _: &str) -> Result<bool> {
            self.control.context("Disposable control adapter failed")
        }
        fn supports_text_input(&self) -> bool {
            self.text
        }
        fn input(&mut self, event: &InputEvent) -> Result<()> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }
    fn text_case(supported: bool) {
        let files = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        let receiver = Identity::generate();
        let client_trust = TrustStore::open(files.path().join("client")).unwrap();
        let server_trust = TrustStore::open(files.path().join("server")).unwrap();
        client_trust.remember(&receiver.fingerprint()).unwrap();
        server_trust.remember(&identity.fingerprint()).unwrap();
        let peer = receiver.fingerprint();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let received = events.clone();
        let server = thread::spawn(move || {
            session::serve_connection_with_cursor(
                listener.accept().unwrap().0,
                &receiver,
                &server_trust,
                &mut None,
                |_, _| Decision::Deny,
                &mut Sink {
                    control: Some(true),
                    text: supported,
                    events: received,
                },
            )
        });
        let mut client = Client::connect(
            TcpStream::connect(address).unwrap(),
            &identity,
            &client_trust,
            None,
            Some(&peer),
            |_, _| Decision::Deny,
        )
        .unwrap();
        client.request_control().unwrap();
        assert_eq!(client.supports_text_input(), supported);
        input_command(&mut client, &json!({"kind":"type"})).unwrap();
        assert_eq!(
            input_command(&mut client, &json!({"kind":"unicode"})).is_ok(),
            supported
        );
        input_command(&mut client, &json!({"kind":"release"})).unwrap();
        client.close().unwrap();
        server.join().unwrap().unwrap();
        let events = events.lock().unwrap();
        let keys: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let InputEvent::MacKey { code, down, .. } = event {
                    Some((*code, *down))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            keys,
            [14, 7, 17, 14, 45, 2]
                .into_iter()
                .flat_map(|code| [(code, true), (code, false)])
                .collect::<Vec<_>>()
        );
        let texts: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let InputEvent::Text { value } = event {
                    Some(value.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            texts,
            if supported {
                vec![UNICODE_TEXT]
            } else {
                vec![]
            }
        );
        assert_eq!(events.last(), Some(&InputEvent::Release));
    }
    #[test]
    fn authenticated_receiver_grant_delivers_fixed_unicode_and_keeps_physical_keys() {
        text_case(true);
    }
    #[test]
    fn unavailable_receiver_rejects_text_without_sending_and_keeps_physical_keys() {
        text_case(false);
    }
    #[test]
    fn recovery_distinguishes_denial_grant_and_broken_control_adapter() {
        for control in [Some(false), Some(true), None] {
            let files = tempfile::tempdir().unwrap();
            let identity = Identity::generate();
            let receiver = Identity::generate();
            let client_trust = TrustStore::open(files.path().join("client")).unwrap();
            let server_trust = TrustStore::open(files.path().join("server")).unwrap();
            client_trust.remember(&receiver.fingerprint()).unwrap();
            server_trust.remember(&identity.fingerprint()).unwrap();
            let peer = receiver.fingerprint();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let events = Arc::new(Mutex::new(Vec::new()));
            let received = events.clone();
            let server = thread::spawn(move || {
                session::serve_connection_with_cursor(
                    listener.accept().unwrap().0,
                    &receiver,
                    &server_trust,
                    &mut None,
                    |_, _| Decision::Deny,
                    &mut Sink {
                        control,
                        text: true,
                        events: received,
                    },
                )
            });
            let candidate = Client::connect(
                TcpStream::connect(address).unwrap(),
                &identity,
                &client_trust,
                None,
                Some(&peer),
                |_, _| Decision::Deny,
            )
            .unwrap();
            let recovered = recovery_grant(candidate);
            assert_eq!(recovered.is_ok(), control.is_some());
            let recovered = recovered.unwrap_or(None);
            assert_eq!(recovered.is_some(), control == Some(true));
            if let Some((mut client, _)) = recovered {
                assert!(client.supports_text_input());
                input_command(&mut client, &json!({"kind":"unicode"})).unwrap();
                client.close().unwrap();
            }
            let served = server.join().unwrap();
            if control == Some(true) {
                served.unwrap();
                assert!(events.lock().unwrap().contains(&InputEvent::Text {
                    value: UNICODE_TEXT.into(),
                }));
            } else {
                assert!(events.lock().unwrap().is_empty());
            }
        }
    }
    #[test]
    fn receipt_and_command_ids_preserve_support_and_private_atomic_files() {
        let files = tempfile::tempdir().unwrap();
        let path = files.path().join("receipt.json");
        assert!(UNICODE_TEXT.encode_utf16().count() > 20);
        InputEvent::Text {
            value: UNICODE_TEXT.into(),
        }
        .validate()
        .unwrap();
        let mut receipt = json!({"state":"connected","text_supported":true,"command_sequence":1});
        save_receipt(&path, &receipt).unwrap();
        receipt["state"] = json!("unicode");
        receipt["command_sequence"] = json!(2);
        save_receipt(&path, &receipt).unwrap();
        let loaded: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded["text_supported"], true);
        assert_eq!(loaded["command_sequence"], 2);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(command_sequence(&json!({"sequence":4}), 3).unwrap(), 4);
        assert_eq!(command_sequence(&json!({}), 3).unwrap(), 4);
        for command in [
            json!({"sequence":3}),
            json!({"sequence":2}),
            json!({"sequence":"4"}),
        ] {
            assert!(command_sequence(&command, 3).is_err());
        }
    }
}
