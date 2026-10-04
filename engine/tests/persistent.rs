use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{serve_connection_with_cursor, Client, ControlApproval, CursorSink, Decision},
    trust::TrustStore,
};
use std::{
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
struct Sink {
    approvals: Arc<AtomicUsize>,
}
impl CursorSink for Sink {
    fn approve(&mut self, _: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn approve_control(&mut self, _: &str, remembered: bool) -> anyhow::Result<ControlApproval> {
        self.approvals.fetch_add(1, Ordering::SeqCst);
        Ok(if remembered {
            ControlApproval::Once
        } else {
            ControlApproval::Remember
        })
    }
    fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
}
#[test]
fn remembered_control_survives_test_deadline_reconnects_and_permission_removal_stops_it() {
    let a = Identity::generate();
    let b = Identity::generate();
    let a_id = a.fingerprint();
    let b_id = b.fingerprint();
    let ad = tempfile::tempdir().unwrap();
    let bd = tempfile::tempdir().unwrap();
    let at = TrustStore::open(ad.path()).unwrap();
    let bt = TrustStore::open(bd.path()).unwrap();
    let bs = bt.clone();
    let mut window = Some(PairingWindow::new(Duration::from_secs(90)));
    let code = window.as_ref().unwrap().code().unwrap().to_owned();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let approvals = Arc::new(AtomicUsize::new(0));
    let observed = approvals.clone();
    let server = thread::spawn(move || {
        (0..3)
            .map(|_| {
                serve_connection_with_cursor(
                    l.accept().unwrap().0,
                    &b,
                    &bs,
                    &mut window,
                    |_, _| Decision::Once,
                    &mut Sink {
                        approvals: approvals.clone(),
                    },
                )
                .is_ok()
            })
            .collect::<Vec<_>>()
    });
    let mut first = Client::connect(
        TcpStream::connect(addr).unwrap(),
        &a,
        &at,
        Some(&code),
        None,
        |_, _| Decision::Remember,
    )
    .unwrap();
    first.request_control(false).unwrap();
    assert!(bt.peer(&a_id).unwrap().unwrap().automatic_input);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(32) {
        first.probe().unwrap();
        thread::sleep(Duration::from_millis(100));
    }
    first.move_cursor(0.4, 0.5).unwrap();
    first.close().unwrap();
    let mut second = Client::connect(
        TcpStream::connect(addr).unwrap(),
        &a,
        &at,
        None,
        Some(&b_id),
        |_, _| Decision::Once,
    )
    .unwrap();
    second.request_control(true).unwrap();
    second.move_cursor(0.5, 0.4).unwrap();
    bt.require_control_consent(&a_id).unwrap();
    assert!(second.probe().is_err());
    drop(second);
    let mut third = Client::connect(
        TcpStream::connect(addr).unwrap(),
        &a,
        &at,
        None,
        Some(&b_id),
        |_, _| Decision::Once,
    )
    .unwrap();
    assert!(third.request_control(true).is_err());
    drop(third);
    assert_eq!(server.join().unwrap(), vec![true, false, false]);
    assert_eq!(
        observed.load(Ordering::SeqCst),
        2,
        "reconnect without permission must not prompt/start native control"
    );
}
#[test]
fn legacy_trust_migrates_to_no_input_and_probe_updates_preserve_explicit_control() {
    let d = tempfile::tempdir().unwrap();
    let id = "a".repeat(64);
    std::fs::write(
        d.path().join("trust.json"),
        format!("{{\"peers\":{{\"{id}\":{{\"automatic_probe\":true}}}},\"revoked\":[]}}"),
    )
    .unwrap();
    let t = TrustStore::open(d.path()).unwrap();
    assert!(!t.peer(&id).unwrap().unwrap().automatic_input);
    t.allow_control(&id).unwrap();
    t.remember(&id, false).unwrap();
    assert!(t.peer(&id).unwrap().unwrap().automatic_input);
    t.revoke(&id).unwrap();
    assert!(t.allow_control(&id).is_err());
}

#[test]
fn actual_transport_drop_reconnects_with_pinned_identity_and_saved_control() {
    use std::net::Shutdown;
    let a = Identity::generate();
    let b = Identity::generate();
    let b_id = b.fingerprint();
    let ad = tempfile::tempdir().unwrap();
    let bd = tempfile::tempdir().unwrap();
    let at = TrustStore::open(ad.path()).unwrap();
    let bt = TrustStore::open(bd.path()).unwrap();
    let mut window = Some(PairingWindow::new(Duration::from_secs(30)));
    let code = window.as_ref().unwrap().code().unwrap().to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let approvals = Arc::new(AtomicUsize::new(0));
    let server = thread::spawn(move || {
        (0..3)
            .map(|index| {
                let stream = listener.accept().unwrap().0;
                let breaker = if index == 1 {
                    let copy = stream.try_clone().unwrap();
                    Some(thread::spawn(move || {
                        thread::sleep(Duration::from_millis(200));
                        let _ = copy.shutdown(Shutdown::Both);
                    }))
                } else {
                    None
                };
                let result = serve_connection_with_cursor(
                    stream,
                    &b,
                    &bt,
                    &mut window,
                    |_, _| Decision::Once,
                    &mut Sink {
                        approvals: approvals.clone(),
                    },
                )
                .is_ok();
                if let Some(breaker) = breaker {
                    breaker.join().unwrap();
                }
                result
            })
            .collect::<Vec<_>>()
    });
    let mut paired = Client::connect(
        TcpStream::connect(address).unwrap(),
        &a,
        &at,
        Some(&code),
        None,
        |_, _| Decision::Remember,
    )
    .unwrap();
    paired.request_control(false).unwrap();
    paired.close().unwrap();
    let mut attempts = 0;
    extend_computer_agent::reconnect::run(true, || {
        attempts += 1;
        let mut client = Client::connect(
            TcpStream::connect(address)?,
            &a,
            &at,
            None,
            Some(&b_id),
            |_, _| Decision::Once,
        )?;
        client.request_control(true)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(500) {
            client.probe()?;
            thread::sleep(Duration::from_millis(25));
        }
        client.close()
    })
    .unwrap();
    assert_eq!(attempts, 2);
    assert_eq!(server.join().unwrap(), vec![true, false, true]);
}

#[cfg(unix)]
#[test]
fn unexpected_native_capture_stop_reports_failure_instead_of_idle_success() {
    use std::os::unix::fs::PermissionsExt;
    for (ending, expected) in [
        (
            "printf 'STOP heartbeat-timeout\\n'\ncat >/dev/null",
            "stopped responding",
        ),
        ("exit 0", "stopped unexpectedly"),
    ] {
        let a = Identity::generate();
        let b = Identity::generate();
        let ad = tempfile::tempdir().unwrap();
        let bd = tempfile::tempdir().unwrap();
        let at = TrustStore::open(ad.path()).unwrap();
        let bt = TrustStore::open(bd.path()).unwrap();
        let helper = ad.path().join("capture");
        std::fs::write(
            &helper,
            format!("#!/bin/sh\nprintf 'READY 1512 982\\n'\nsleep 0.05\n{ending}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut window = Some(PairingWindow::new(Duration::from_secs(30)));
        let code = window.as_ref().unwrap().code().unwrap().to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let _ = serve_connection_with_cursor(
                listener.accept().unwrap().0,
                &b,
                &bt,
                &mut window,
                |_, _| Decision::Once,
                &mut Sink {
                    approvals: Arc::new(AtomicUsize::new(0)),
                },
            );
        });
        let client = Client::connect(
            TcpStream::connect(address).unwrap(),
            &a,
            &at,
            Some(&code),
            None,
            |_, _| Decision::Once,
        )
        .unwrap();
        let error = extend_computer_agent::control::send_session(
            client, &helper, false, "left", 0.0, None, false,
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
        assert!(!extend_computer_agent::error::is_connection_failure(&error));
        server.join().unwrap();
    }
}

#[cfg(unix)]
#[test]
fn emergency_stop_wins_over_simultaneous_connection_failure() {
    use std::os::unix::fs::PermissionsExt;
    struct FailingHeartbeat;
    impl CursorSink for FailingHeartbeat {
        fn approve(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        fn approve_control(&mut self, _: &str, _: bool) -> anyhow::Result<ControlApproval> {
            Ok(ControlApproval::Once)
        }
        fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
            Ok(())
        }
        fn heartbeat(&mut self) -> anyhow::Result<()> {
            anyhow::bail!("test transport shutdown")
        }
    }
    let a = Identity::generate();
    let b = Identity::generate();
    let ad = tempfile::tempdir().unwrap();
    let bd = tempfile::tempdir().unwrap();
    let at = TrustStore::open(ad.path()).unwrap();
    let bt = TrustStore::open(bd.path()).unwrap();
    let helper = ad.path().join("capture");
    std::fs::write(&helper, "#!/bin/sh\nprintf 'READY 1512 982\\n'\nsleep 0.05\nprintf 'STOP user\\n'\ncat >/dev/null\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut window = Some(PairingWindow::new(Duration::from_secs(30)));
    let code = window.as_ref().unwrap().code().unwrap().to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        serve_connection_with_cursor(
            listener.accept().unwrap().0,
            &b,
            &bt,
            &mut window,
            |_, _| Decision::Once,
            &mut FailingHeartbeat,
        )
        .is_err()
    });
    let mut attempts = 0;
    extend_computer_agent::reconnect::run(true, || {
        attempts += 1;
        anyhow::ensure!(attempts == 1, "emergency stop must not reconnect");
        let client = Client::connect(
            TcpStream::connect(address)?,
            &a,
            &at,
            Some(&code),
            None,
            |_, _| Decision::Once,
        )?;
        extend_computer_agent::control::send_session(
            client, &helper, false, "left", 0.0, None, false,
        )
    })
    .unwrap();
    assert_eq!(attempts, 1);
    assert!(server.join().unwrap());
}
