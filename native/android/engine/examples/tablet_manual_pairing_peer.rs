//! Disposable, loopback-only peer for the real outgoing manual pairing UI.
//! Never loads a desktop identity or touches a user's trust. Codes stay in 0600
//! private.json; status.json contains only public identities and test outcomes.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{self, Decision},
    trust::TrustStore,
};
use serde_json::{json, Value};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::{ErrorKind, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

struct Receipt {
    path: PathBuf,
    started: Instant,
    value: Value,
}
impl Receipt {
    fn publish(&mut self) -> Result<()> {
        self.value["elapsed_ms"] = json!(self.started.elapsed().as_millis());
        // Same-directory rename keeps readers from observing partial JSON. Both
        // the temporary inode and final inode are private, including on failure.
        let mut file = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())?;
        #[cfg(unix)]
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(&mut file, &self.value)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        Ok(())
    }
}

fn prepare(
    directory: &Path,
) -> Result<(
    Identity,
    TrustStore,
    TcpListener,
    PairingWindow,
    Receipt,
    tempfile::TempDir,
)> {
    fs::create_dir(directory).context("Use a new fixture directory")?;
    #[cfg(unix)]
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let scratch = tempfile::tempdir()?;
    let identity = Identity::generate();
    let trust = TrustStore::open(scratch.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let window = PairingWindow::new(Duration::from_secs(120));
    let mut private = tempfile::NamedTempFile::new_in(directory)?;
    #[cfg(unix)]
    private
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(
        &mut private,
        &json!({
            "address":listener.local_addr()?.to_string(),
            "code":window.code().unwrap(), "peer":identity.fingerprint(),
            "name":format!("Manual test {}", &identity.fingerprint()[..8]), "expires_after_ms":120_000,
        }),
    )?;
    private.write_all(b"\n")?;
    private.as_file().sync_all()?;
    private.persist(directory.join("private.json"))?;
    let mut receipt = Receipt {
        path: directory.join("status.json"),
        started: Instant::now(),
        value: json!({"state":"ready", "peer":identity.fingerprint(),
            "client_peer":null, "pending_approval":false, "decision":null,
            "eof_before_decision":false, "unexpected_pending_data":false,
            "remote_trusted":false, "clean_close":false, "outcome":null,
            "clock":"fixture-process-monotonic-ms"}),
    };
    receipt.publish()?;
    Ok((identity, trust, listener, window, receipt, scratch))
}

fn approve(directory: &Path, monitor: &TcpStream, receipt: &mut Receipt, peer: &str) -> Decision {
    receipt.value["state"] = json!("waiting");
    receipt.value["client_peer"] = json!(peer);
    receipt.value["pending_approval"] = json!(true);
    if receipt.publish().is_err()
        || monitor
            .set_read_timeout(Some(Duration::from_millis(20)))
            .is_err()
    {
        return Decision::Deny;
    }
    let expires = receipt.started + Duration::from_secs(115);
    loop {
        let decision = if directory.join("stop").exists() || directory.join("deny").exists() {
            Some(Decision::Deny)
        } else if directory.join("grant").exists() {
            Some(Decision::Remember)
        } else if Instant::now() >= expires {
            Some(Decision::Deny)
        } else {
            None
        };
        if let Some(decision) = decision {
            receipt.value["pending_approval"] = json!(false);
            receipt.value["decision"] = json!(if matches!(decision, Decision::Remember) {
                "grant"
            } else {
                "deny"
            });
            receipt.value["decision_elapsed_ms"] = json!(receipt.started.elapsed().as_millis());
            if receipt.publish().is_err() {
                return Decision::Deny;
            }
            // This peek monitor runs only inside the blocking consent callback.
            // Restore the socket timeout before shared-engine I/O resumes; no
            // other thread changes the socket options or consumes protocol data.
            if monitor
                .set_read_timeout(Some(Duration::from_secs(120)))
                .is_err()
            {
                return Decision::Deny;
            }
            return decision;
        }
        let mut byte = [0];
        match monitor.peek(&mut byte) {
            Ok(0) => {
                if receipt.value["eof_before_decision"] != true {
                    receipt.value["eof_before_decision"] = json!(true);
                    receipt.value["eof_elapsed_ms"] = json!(receipt.started.elapsed().as_millis());
                    if receipt.publish().is_err() {
                        return Decision::Deny;
                    }
                }
                // Retain the approval point after EOF so a deliberately late
                // grant still tests the remote-vs-local trust commit boundary.
                thread::sleep(Duration::from_millis(20));
            }
            Ok(_) => {
                receipt.value["unexpected_pending_data"] = json!(true);
                let _ = receipt.publish();
                return Decision::Deny;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                ) => {}
            Err(_) => {
                receipt.value["outcome"] = json!("pending-socket-error");
                let _ = receipt.publish();
                return Decision::Deny;
            }
        }
    }
}

fn run(directory: &Path) -> Result<()> {
    let (identity, trust, listener, window, mut receipt, _scratch) = prepare(directory)?;
    println!("Ready: disposable loopback manual pairing peer; private code not printed.");
    let socket = loop {
        if directory.join("stop").exists() || receipt.started.elapsed() >= Duration::from_secs(120)
        {
            receipt.value["state"] = json!("finished");
            receipt.value["outcome"] = json!("stopped-before-connection");
            receipt.publish()?;
            return Ok(());
        }
        match listener.accept() {
            Ok((socket, address)) => {
                ensure!(
                    address.ip().is_loopback(),
                    "Loopback fixture rejected a remote address"
                );
                break socket;
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(error) => return Err(error.into()),
        }
    };
    socket.set_nonblocking(false)?;
    let monitor = socket.try_clone()?;
    receipt.value["state"] = json!("handshaking");
    receipt.publish()?;
    let result =
        session::serve_connection(socket, &identity, &trust, &mut Some(window), |peer, _| {
            approve(directory, &monitor, &mut receipt, peer)
        });
    let trusted = receipt.value["client_peer"]
        .as_str()
        .map(|peer| trust.peer(peer))
        .transpose()?
        .flatten()
        .is_some();
    receipt.value["state"] = json!("finished");
    receipt.value["pending_approval"] = json!(false);
    receipt.value["remote_trusted"] = json!(trusted);
    receipt.value["clean_close"] = json!(result.is_ok());
    if receipt.value["outcome"].is_null() {
        receipt.value["outcome"] = json!(match (
            trusted,
            receipt.value["eof_before_decision"] == true,
            result.is_ok()
        ) {
            (true, true, _) => "granted-after-peer-eof",
            (true, false, true) => "granted-and-closed",
            (true, false, false) => "granted-connection-failed",
            (false, _, _) => "not-granted",
        });
    }
    let _ = monitor.shutdown(Shutdown::Both);
    receipt.publish()?;
    println!("Finished: sanitized receipt saved; disposable trust removed on exit.");
    Ok(())
}

fn main() -> Result<()> {
    let directory = std::env::args()
        .nth(1)
        .context("New private fixture directory required")?;
    run(Path::new(&directory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::session::Client;
    use std::sync::Arc;

    fn wait(directory: &Path, check: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(bytes) = fs::read(directory.join("status.json")) {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                if check(&value) {
                    return value;
                }
            }
            assert!(Instant::now() < deadline, "Fixture receipt timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn marker(directory: &Path, name: &str) {
        fs::write(directory.join(name), b"").unwrap();
    }
    #[test]
    fn pending_cancel_eof_then_late_grant_records_independent_remote_commit() {
        let files = tempfile::tempdir().unwrap();
        let directory = files.path().join("fixture");
        let server_directory = directory.clone();
        let server = thread::spawn(move || run(&server_directory).unwrap());
        wait(&directory, |state| state["state"] == "ready");
        let private: Value =
            serde_json::from_slice(&fs::read(directory.join("private.json")).unwrap()).unwrap();
        let identity = Arc::new(Identity::generate());
        let local_trust = TrustStore::open(files.path().join("client")).unwrap();
        let socket = TcpStream::connect(private["address"].as_str().unwrap()).unwrap();
        let cancel = socket.try_clone().unwrap();
        let client_identity = identity.clone();
        let client_trust = local_trust.clone();
        let code = private["code"].as_str().unwrap().to_owned();
        let peer = private["peer"].as_str().unwrap().to_owned();
        let expected = peer.clone();
        let client = thread::spawn(move || {
            Client::connect(
                socket,
                &client_identity,
                &client_trust,
                Some(&code),
                Some(&expected),
                |_, _| Decision::Once,
            )
        });
        let pending = wait(&directory, |state| state["pending_approval"] == true);
        assert_eq!(pending["client_peer"], identity.fingerprint());
        assert!(!local_trust.load().unwrap().peers.contains_key(&peer));
        cancel.shutdown(Shutdown::Both).unwrap();
        assert!(client.join().unwrap().is_err());
        wait(&directory, |state| state["eof_before_decision"] == true);
        marker(&directory, "grant");
        server.join().unwrap();
        let receipt = wait(&directory, |state| state["state"] == "finished");
        assert_eq!(receipt["outcome"], "granted-after-peer-eof");
        assert_eq!(receipt["remote_trusted"], true);
        assert!(
            receipt["eof_elapsed_ms"].as_u64().unwrap()
                <= receipt["decision_elapsed_ms"].as_u64().unwrap()
        );
        assert!(!local_trust.load().unwrap().peers.contains_key(&peer));
        assert!(!receipt
            .to_string()
            .contains(private["code"].as_str().unwrap()));
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for path in ["private.json", "status.json"] {
                assert_eq!(
                    fs::metadata(directory.join(path))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
    }
    #[test]
    fn approved_success_closes_cleanly_and_new_directory_uses_new_identity() {
        let files = tempfile::tempdir().unwrap();
        let directory = files.path().join("fixture");
        let server_directory = directory.clone();
        let server = thread::spawn(move || run(&server_directory).unwrap());
        wait(&directory, |state| state["state"] == "ready");
        let private: Value =
            serde_json::from_slice(&fs::read(directory.join("private.json")).unwrap()).unwrap();
        let identity = Identity::generate();
        let local_trust = TrustStore::open(files.path().join("client")).unwrap();
        let server_peer = private["peer"].as_str().unwrap().to_owned();
        let approval = directory.clone();
        let grant = thread::spawn(move || {
            wait(&approval, |state| state["pending_approval"] == true);
            marker(&approval, "grant");
        });
        let client = Client::connect(
            TcpStream::connect(private["address"].as_str().unwrap()).unwrap(),
            &identity,
            &local_trust,
            Some(private["code"].as_str().unwrap()),
            Some(&server_peer),
            |_, _| Decision::Remember,
        )
        .unwrap();
        client.close().unwrap();
        grant.join().unwrap();
        server.join().unwrap();
        let receipt = wait(&directory, |state| state["state"] == "finished");
        assert_eq!(receipt["outcome"], "granted-and-closed");
        assert_eq!(receipt["clean_close"], true);
        assert!(local_trust.load().unwrap().peers.contains_key(&server_peer));
        let next = files.path().join("next");
        let (_, _, _, _, next_receipt, _) = prepare(&next).unwrap();
        assert_ne!(receipt["peer"], next_receipt.value["peer"]);
        assert!(
            prepare(&directory).is_err(),
            "Existing run must not be overwritten"
        );
    }
    #[test]
    fn stop_before_connection_finishes_without_peer_or_approval() {
        let files = tempfile::tempdir().unwrap();
        let directory = files.path().join("fixture");
        let server_directory = directory.clone();
        let server = thread::spawn(move || run(&server_directory).unwrap());
        wait(&directory, |state| state["state"] == "ready");
        marker(&directory, "stop");
        server.join().unwrap();
        let receipt = wait(&directory, |state| state["state"] == "finished");
        assert_eq!(receipt["outcome"], "stopped-before-connection");
        assert_eq!(receipt["client_peer"], Value::Null);
        assert_eq!(receipt["remote_trusted"], false);
    }
}
