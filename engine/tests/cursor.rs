use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{serve_connection_with_cursor, Client, CursorSink, Decision},
    trust::TrustStore,
};
use std::{
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
struct Sink {
    allow: bool,
    finished: Arc<std::sync::atomic::AtomicUsize>,
    moves: Arc<Mutex<Vec<(f64, f64)>>>,
}
impl CursorSink for Sink {
    fn display_size(&self) -> anyhow::Result<extend_computer_agent::session::DisplaySize> {
        Ok(extend_computer_agent::session::DisplaySize {
            width: 1280.0,
            height: 800.0,
        })
    }
    fn finish(&mut self) {
        self.finished
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn start_cursor(&mut self, _: &str) -> anyhow::Result<bool> {
        Ok(self.allow)
    }
    fn move_to(&mut self, x: f64, y: f64) -> anyhow::Result<()> {
        self.moves.lock().unwrap().push((x, y));
        Ok(())
    }
}
fn scenario(
    allow: bool,
    action: impl FnOnce(&mut Client, &TrustStore, &str),
    expected_moves: usize,
    success: bool,
) {
    let a = Identity::generate();
    let b = Identity::generate();
    let ad = tempfile::tempdir().unwrap();
    let bd = tempfile::tempdir().unwrap();
    let at = TrustStore::open(ad.path()).unwrap();
    let bt = TrustStore::open(bd.path()).unwrap();
    let mut window = Some(PairingWindow::new(Duration::from_secs(30)));
    let code = window.as_ref().unwrap().code().unwrap().to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let moves = Arc::new(Mutex::new(Vec::new()));
    let observed = moves.clone();
    let store = bt.clone();
    let task = thread::spawn(move || {
        let finished = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut sink = Sink {
            allow,
            moves,
            finished: finished.clone(),
        };
        let success = serve_connection_with_cursor(
            listener.accept().unwrap().0,
            &b,
            &store,
            &mut window,
            |_, _| Decision::Remember,
            &mut sink,
        )
        .is_ok();
        assert_eq!(
            finished.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "session resources must release before caller drops sink"
        );
        success
    });
    let mut client = Client::connect(
        TcpStream::connect(address).unwrap(),
        &a,
        &at,
        Some(&code),
        None,
        |_, _| Decision::Remember,
    )
    .unwrap();
    action(&mut client, &bt, &a.fingerprint());
    let _ = client.close();
    assert_eq!(task.join().unwrap(), success);
    assert_eq!(observed.lock().unwrap().len(), expected_moves);
}
#[test]
fn unavailable_cursor_adapter_blocks_movement() {
    scenario(
        false,
        |c, _, _| {
            assert!(c.request_cursor().is_err());
        },
        0,
        false,
    );
}
#[test]
fn cursor_without_request_rejected() {
    scenario(
        true,
        |c, _, _| {
            assert!(c.move_cursor(0.5, 0.5).is_err());
        },
        0,
        false,
    );
}
#[test]
fn cursor_consent_moves_and_revocation_stops_injection() {
    scenario(
        true,
        |c, t, id| {
            c.request_cursor().unwrap();
            c.move_cursor(0.25, 0.75).unwrap();
            t.revoke(id).unwrap();
            assert!(c.move_cursor(0.4, 0.4).is_err());
        },
        1,
        false,
    );
}
#[test]
fn invalid_coordinates_never_reach_sink() {
    scenario(
        true,
        |c, _, _| {
            c.request_cursor().unwrap();
            for (x, y) in [
                (f64::NAN, 0.0),
                (f64::INFINITY, 0.0),
                (-0.1, 0.0),
                (0.0, 1.1),
            ] {
                assert!(c.move_cursor(x, y).is_err());
            }
            c.move_cursor(1.0, 0.0).unwrap();
        },
        1,
        true,
    );
}
#[test]
fn duplicate_request_cannot_extend_grant() {
    scenario(
        true,
        |c, _, _| {
            c.request_cursor().unwrap();
            assert!(c.request_cursor().is_err());
        },
        0,
        false,
    );
}

#[test]
fn bounded_stream_is_applied_before_probe_ack() {
    scenario(
        true,
        |c, _, _| {
            c.request_cursor().unwrap();
            for _ in 0..3 {
                for i in 0..4 {
                    c.queue_cursor(i as f64 / 4.0, 0.5).unwrap();
                }
                c.probe().unwrap();
            }
        },
        12,
        true,
    );
}
#[test]
fn stream_without_consent_never_injects() {
    scenario(
        true,
        |c, _, _| {
            let _ = c.queue_cursor(0.5, 0.5);
            assert!(c.probe().is_err());
        },
        0,
        false,
    );
}
#[test]
fn stream_window_overflow_is_rejected() {
    scenario(
        true,
        |c, _, _| {
            c.request_cursor().unwrap();
            for _ in 0..5 {
                let _ = c.queue_cursor(0.5, 0.5);
            }
            assert!(c.probe().is_err());
        },
        4,
        false,
    );
}
#[test]
fn stream_revocation_stops_next_injection() {
    scenario(
        true,
        |c, t, id| {
            c.request_cursor().unwrap();
            c.queue_cursor(0.5, 0.5).unwrap();
            c.probe().unwrap();
            t.revoke(id).unwrap();
            let _ = c.queue_cursor(0.7, 0.7);
            assert!(c.probe().is_err());
        },
        1,
        false,
    );
}

#[test]
fn approved_cursor_reports_receiver_display_geometry() {
    scenario(
        true,
        |client, _, _| {
            let display = client.request_cursor().unwrap();
            assert_eq!(display.width, 1280.0);
            assert_eq!(display.height, 800.0);
        },
        0,
        true,
    );
}

#[test]
fn invalid_display_dimensions_are_rejected() {
    for (width, height) in [
        (f64::NAN, 800.0),
        (1280.0, f64::INFINITY),
        (0.0, 800.0),
        (1280.0, 32769.0),
    ] {
        assert!(
            extend_computer_agent::session::DisplaySize { width, height }
                .validate()
                .is_err()
        );
    }
}
