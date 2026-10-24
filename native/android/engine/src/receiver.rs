//! Platform input mailbox. Android owns permissions and native injection.
use anyhow::{ensure, Result};
use extend_computer_agent::{
    input::{InputEvent, ShortcutPolicy},
    session::DisplaySize,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    available: bool,
    text_available: bool,
    keyboard_available: bool,
    width: f64,
    height: f64,
    generation: u64,
    active: Option<u64>,
    queue: VecDeque<Value>,
    polled: Option<Instant>,
}
#[derive(Default)]
pub struct Receiver(Mutex<State>);
static RECEIVER: OnceLock<Arc<Receiver>> = OnceLock::new();
pub fn shared() -> Arc<Receiver> {
    RECEIVER
        .get_or_init(|| Arc::new(Receiver::default()))
        .clone()
}
impl Receiver {
    /// Read-only admission hint; never drains or changes the input mailbox.
    pub fn active(&self) -> bool {
        self.0.lock().unwrap().active.is_some()
    }

    /// Native API availability is distinct from committed-text support.
    pub fn configure_keyboard(&self, enabled: bool) {
        self.0.lock().unwrap().keyboard_available = enabled;
    }
    pub fn shortcut_policy(&self, id: u64) -> Option<ShortcutPolicy> {
        let state = self.0.lock().unwrap();
        (state.available
            && state.keyboard_available
            && state.active == Some(id)
            && state
                .polled
                .is_some_and(|time| time.elapsed() < Duration::from_secs(1)))
        .then_some(ShortcutPolicy::Control)
    }
    pub fn configure_text(&self, enabled: bool) {
        let mut state = self.0.lock().unwrap();
        state.text_available = enabled;
        if !enabled {
            state.queue.retain(|event| event["kind"] != "text");
        }
    }
    pub fn supports_text_input(&self) -> bool {
        let s = self.0.lock().unwrap();
        s.available && s.text_available
    }
    pub fn configure(&self, enabled: bool, width: f64, height: f64) -> Result<()> {
        DisplaySize { width, height }.validate()?;
        let mut s = self.0.lock().unwrap();
        if !enabled || s.width != width || s.height != height {
            if let Some(id) = s.active.take() {
                s.queue.clear();
                s.queue
                    .push_back(json!({"kind":"release", "id":id,"end":true}));
            }
        }
        s.available = enabled;
        if !enabled {
            s.keyboard_available = false;
        }
        s.width = width;
        s.height = height;
        Ok(())
    }
    pub fn begin(&self, peer: &str) -> Result<Option<u64>> {
        let mut s = self.0.lock().unwrap();
        if !s.available
            || s.active.is_some()
            || !s
                .polled
                .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return Ok(None);
        }
        s.generation += 1;
        let id = s.generation;
        s.active = Some(id);
        s.queue
            .push_back(json!({"kind":"start","id":id,"peer":peer}));
        Ok(Some(id))
    }
    pub fn display(&self) -> Result<DisplaySize> {
        let s = self.0.lock().unwrap();
        DisplaySize {
            width: s.width,
            height: s.height,
        }
        .validate()
    }
    pub fn alive(&self, id: u64) -> Result<()> {
        let s = self.0.lock().unwrap();
        ensure!(
            s.available
                && s.active == Some(id)
                && s.polled
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(1)),
            "Android input permission or service unavailable"
        );
        Ok(())
    }
    pub fn enqueue(&self, id: u64, mut event: Value) -> Result<()> {
        self.alive(id)?;
        let mut s = self.0.lock().unwrap();
        ensure!(s.available && s.active == Some(id), "Input session ended");
        if event["kind"] == "text" {
            ensure!(s.text_available, "Android text adapter unavailable");
        }
        event["id"] = json!(id);
        if event["kind"] == "move" && s.queue.back().is_some_and(|v| v["kind"] == "move") {
            s.queue.pop_back();
        }
        if s.queue.len() >= 256 {
            s.active = None;
            s.queue.clear();
            s.queue
                .push_back(json!({"kind":"release","id":id,"end":true}));
            anyhow::bail!("Android input queue full");
        }
        s.queue.push_back(event);
        Ok(())
    }
    pub fn input(&self, id: u64, event: &InputEvent) -> Result<()> {
        event.validate()?;
        self.enqueue(id, serde_json::to_value(event)?)
    }
    pub fn finish(&self, id: u64) {
        let mut s = self.0.lock().unwrap();
        if s.active == Some(id) {
            s.active = None;
            s.keyboard_available = false;
            s.queue.clear();
            s.queue
                .push_back(json!({"kind":"release","id":id,"end":true}));
        }
    }
    pub fn drain(&self) -> Value {
        let mut s = self.0.lock().unwrap();
        s.polled = Some(Instant::now());
        let events: Vec<_> = s.queue.drain(..).collect();
        json!(events)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_policy_is_native_bound_current_and_separate_from_text() {
        let receiver = Receiver::default();
        receiver.configure(true, 1000., 800.).unwrap();
        receiver.drain();
        let id = receiver.begin("paired").unwrap().unwrap();
        receiver.configure_text(true);
        assert_eq!(receiver.shortcut_policy(id), None);
        receiver.configure_keyboard(true);
        assert_eq!(receiver.shortcut_policy(id), Some(ShortcutPolicy::Control));
        assert_eq!(receiver.shortcut_policy(id + 1), None);
        receiver.configure_text(false);
        assert_eq!(receiver.shortcut_policy(id), Some(ShortcutPolicy::Control));
        receiver.configure_keyboard(false);
        assert_eq!(receiver.shortcut_policy(id), None);
        receiver.configure_keyboard(true);
        receiver.finish(id);
        assert_eq!(receiver.shortcut_policy(id), None);
        let replacement = receiver.begin("paired").unwrap().unwrap();
        assert_eq!(receiver.shortcut_policy(replacement), None);
        receiver.configure_keyboard(true);
        receiver.finish(id); // A stale old finish must not revoke the replacement.
        assert_eq!(
            receiver.shortcut_policy(replacement),
            Some(ShortcutPolicy::Control)
        );
        receiver.configure(false, 16., 16.).unwrap();
        assert_eq!(receiver.shortcut_policy(replacement), None);
        receiver.configure(true, 1000., 800.).unwrap();
        receiver.drain();
        let current = receiver.begin("paired").unwrap().unwrap();
        assert_eq!(receiver.shortcut_policy(current), None);
        receiver.configure_keyboard(true);
        receiver.0.lock().unwrap().polled = Some(Instant::now() - Duration::from_secs(2));
        assert_eq!(receiver.shortcut_policy(current), None);
    }
    #[test]
    fn text_adapter_grant_requires_native_api_permission_flag() {
        let receiver = Receiver::default();
        receiver.configure(true, 1000., 800.).unwrap();
        receiver.drain();
        let id = receiver.begin("peer").unwrap().unwrap();
        let text = InputEvent::Text {
            value: "é🙂".into(),
        };
        assert!(!receiver.supports_text_input());
        assert!(receiver.input(id, &text).is_err());
        receiver.configure_text(true);
        assert!(receiver.supports_text_input());
        receiver.input(id, &text).unwrap();
        receiver.configure_text(false);
        assert!(receiver.input(id, &text).is_err());
        receiver.configure_text(true);
        receiver.configure(false, 1000., 800.).unwrap();
        assert!(!receiver.supports_text_input());
    }
    #[test]
    fn permission_polling_order_overflow_and_stale_session() {
        let r = Receiver::default();
        assert!(r.begin("peer").unwrap().is_none());
        r.configure(true, 1600., 2560.).unwrap();
        assert!(r.begin("peer").unwrap().is_none());
        r.drain();
        let id = r.begin("peer").unwrap().unwrap();
        r.enqueue(id, json!({"kind":"move","x":0.2,"y":0.3}))
            .unwrap();
        r.enqueue(id, json!({"kind":"move","x":0.4,"y":0.5}))
            .unwrap();
        r.input(
            id,
            &InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            },
        )
        .unwrap();
        let events = r.drain();
        assert_eq!(events.as_array().unwrap().len(), 3);
        assert_eq!(events[1]["x"], 0.4);
        r.configure(false, 1600., 2560.).unwrap();
        assert!(r.alive(id).is_err());
        assert_eq!(r.drain()[0]["kind"], "release");
        r.configure(true, 1600., 2560.).unwrap();
        let next = r.begin("peer").unwrap().unwrap();
        r.finish(id);
        assert!(r.alive(next).is_ok());
        r.drain();
        for _ in 0..256 {
            r.input(next, &InputEvent::Modifiers { mask: 0 }).unwrap();
        }
        assert!(r.input(next, &InputEvent::Release).is_err());
        assert_eq!(r.drain()[0]["kind"], "release");
        assert!(r.alive(next).is_err());
    }
    #[test]
    fn trusted_protocol_reaches_platform_and_permission_revocation_closes() {
        use extend_computer_agent::{
            identity::Identity,
            session::{Client, Decision},
            trust::TrustStore,
        };
        use std::{net::TcpStream, thread};
        let files = tempfile::tempdir().unwrap();
        let tablet = Arc::new(Identity::generate());
        let remote = Identity::generate();
        let trust = TrustStore::open(files.path().join("tablet")).unwrap();
        let remote_trust = TrustStore::open(files.path().join("remote")).unwrap();
        trust.remember(&remote.fingerprint()).unwrap();
        remote_trust.remember(&tablet.fingerprint()).unwrap();
        let receiver = Arc::new(Receiver::default());
        receiver.configure(true, 1600., 2560.).unwrap();
        receiver.configure_keyboard(true);
        receiver.drain();
        let mut server = crate::incoming::Incoming::receiving(
            tablet.clone(),
            trust,
            "Tablet".into(),
            Arc::new(crate::names::Names::open(files.path()).unwrap()),
            receiver.clone(),
        )
        .unwrap();
        let port = server.snapshot()["port"].as_u64().unwrap() as u16;
        let mut client = Client::connect(
            TcpStream::connect(("127.0.0.1", port)).unwrap(),
            &remote,
            &remote_trust,
            None,
            Some(&tablet.fingerprint()),
            |_, _| Decision::Deny,
        )
        .unwrap();
        assert_eq!(client.request_control().unwrap().width, 1600.);
        assert_eq!(client.shortcut_policy(), Some(ShortcutPolicy::Control));
        client.queue_cursor(0.3, 0.4).unwrap();
        client
            .queue_input(InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            })
            .unwrap();
        client.probe().unwrap();
        let events = receiver.drain();
        assert_eq!(events[0]["kind"], "start");
        assert_eq!(events[1]["kind"], "move");
        assert_eq!(events[2]["kind"], "button");
        receiver.configure(false, 1600., 2560.).unwrap();
        assert!(client.probe().is_err());
        thread::sleep(Duration::from_millis(20));
        let events = receiver.drain();
        assert_eq!(events[0]["kind"], "release");
        assert_eq!(events[0]["end"], true);
        server.stop();
    }
}
