use extend_computer_agent::{
    identity::Identity,
    input::InputEvent,
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
#[derive(Default)]
struct Observed {
    events: Vec<InputEvent>,
    finished: bool,
}
struct Sink {
    allow: bool,
    text: bool,
    state: Arc<Mutex<Observed>>,
}
impl CursorSink for Sink {
    fn supports_text_input(&self) -> bool {
        self.text
    }
    fn start_control(&mut self, _: &str) -> anyhow::Result<bool> {
        Ok(self.allow)
    }
    fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn input(&mut self, e: &InputEvent) -> anyhow::Result<()> {
        self.state.lock().unwrap().events.push(e.clone());
        Ok(())
    }
    fn finish(&mut self) {
        self.state.lock().unwrap().finished = true;
    }
}
fn scenario(
    allow: bool,
    action: impl FnOnce(&mut Client, &TrustStore, &str),
    success: bool,
) -> Vec<InputEvent> {
    scenario_with_adapter(allow, true, action, success)
}
fn scenario_with_adapter(
    allow: bool,
    text: bool,
    action: impl FnOnce(&mut Client, &TrustStore, &str),
    success: bool,
) -> Vec<InputEvent> {
    let a = Identity::generate();
    let b = Identity::generate();
    let ad = tempfile::tempdir().unwrap();
    let bd = tempfile::tempdir().unwrap();
    let at = TrustStore::open(ad.path()).unwrap();
    let bt = TrustStore::open(bd.path()).unwrap();
    let store = bt.clone();
    let mut window = Some(PairingWindow::new(Duration::from_secs(30)));
    let code = window.as_ref().unwrap().code().unwrap().to_owned();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let state = Arc::new(Mutex::new(Observed::default()));
    let seen = state.clone();
    let t = thread::spawn(move || {
        serve_connection_with_cursor(
            l.accept().unwrap().0,
            &b,
            &store,
            &mut window,
            |_, _| Decision::Remember,
            &mut Sink { allow, text, state },
        )
        .is_ok()
    });
    let mut c = Client::connect(
        TcpStream::connect(addr).unwrap(),
        &a,
        &at,
        Some(&code),
        None,
        |_, _| Decision::Remember,
    )
    .unwrap();
    action(&mut c, &bt, &a.fingerprint());
    let _ = c.close();
    assert_eq!(t.join().unwrap(), success);
    let seen = seen.lock().unwrap();
    assert!(seen.finished);
    seen.events.clone()
}
#[test]
fn unavailable_input_adapter_blocks_input() {
    assert!(scenario(
        false,
        |c, _, _| {
            assert!(c.request_control().is_err());
        },
        false
    )
    .is_empty());
}
#[test]
fn transitions_remain_ordered_before_barrier() {
    let events = scenario(
        true,
        |c, _, _| {
            c.request_control().unwrap();
            for event in [
                InputEvent::Modifiers { mask: 1 },
                InputEvent::MacKey {
                    code: 0,
                    down: true,
                    repeat: false,
                },
                InputEvent::MacKey {
                    code: 0,
                    down: false,
                    repeat: false,
                },
                InputEvent::Release,
            ] {
                c.queue_input(event).unwrap();
            }
            c.probe().unwrap();
        },
        true,
    );
    assert_eq!(events.len(), 4);
    assert_eq!(events.last(), Some(&InputEvent::Release));
}
#[test]
fn revocation_stops_keys_and_finishes_sink() {
    let events = scenario(
        true,
        |c, t, id| {
            c.request_control().unwrap();
            c.queue_input(InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            })
            .unwrap();
            c.probe().unwrap();
            t.revoke(id).unwrap();
            let _ = c.queue_input(InputEvent::Scroll { dx: 0, dy: 20 });
            assert!(c.probe().is_err());
        },
        false,
    );
    assert_eq!(events.len(), 1);
}
#[test]
fn input_window_is_bounded() {
    let events = scenario(
        true,
        |c, _, _| {
            c.request_control().unwrap();
            for _ in 0..5 {
                let _ = c.queue_input(InputEvent::Release);
            }
            assert!(c.probe().is_err());
        },
        false,
    );
    assert_eq!(events.len(), 4);
}
#[test]
fn duplicate_grant_cannot_extend_input() {
    scenario(
        true,
        |c, _, _| {
            c.request_control().unwrap();
            assert!(c.request_control().is_err());
        },
        false,
    );
}
#[test]
fn input_validation_rejects_invalid_fields() {
    for event in [
        InputEvent::Button {
            button: 3,
            down: true,
            clicks: 1,
        },
        InputEvent::Scroll {
            dx: i32::MIN,
            dy: 0,
        },
        InputEvent::MacKey {
            code: 999,
            down: true,
            repeat: false,
        },
        InputEvent::MacKey {
            code: 0,
            down: false,
            repeat: true,
        },
        InputEvent::Modifiers { mask: 16 },
    ] {
        assert!(event.validate().is_err());
    }
}

#[test]
fn committed_unicode_is_bounded_and_crosses_authenticated_input() {
    for value in ["Aaé", "e\u{301}", "🙂", "👩‍💻", "中文"] {
        let text = InputEvent::Text {
            value: value.into(),
        };
        text.validate().unwrap();
        let encoded = serde_json::to_vec(&text).unwrap();
        assert_eq!(
            serde_json::from_slice::<InputEvent>(&encoded).unwrap(),
            text
        );
    }
    for value in [
        "".to_string(),
        "x\0".into(),
        "x".repeat(65),
        "🙂".repeat(65),
    ] {
        assert!(InputEvent::Text { value }.validate().is_err());
    }
    assert!(serde_json::from_str::<InputEvent>(r#"{"kind":"text","value":"\ud800"}"#).is_err());
    let text = InputEvent::Text {
        value: "é🙂中文".into(),
    };
    let expected = text.clone();
    let events = scenario(
        true,
        |c, _, _| {
            c.request_control().unwrap();
            assert!(c.supports_text_input());
            c.queue_input(text).unwrap();
            c.probe().unwrap();
        },
        true,
    );
    assert_eq!(events, vec![expected]);
}

#[test]
fn text_encoding_without_receiving_adapter_keeps_physical_input_usable() {
    let events = scenario_with_adapter(
        true,
        false,
        |client, _, _| {
            client.request_control().unwrap();
            assert!(!client.supports_text_input());
            assert!(client
                .queue_input(InputEvent::Text {
                    value: "é🙂".into()
                })
                .is_err());
            client.queue_input(InputEvent::Release).unwrap();
            client.probe().unwrap();
        },
        true,
    );
    assert_eq!(events, vec![InputEvent::Release]);
}
