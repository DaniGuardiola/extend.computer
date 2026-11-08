//! What the menu bar (or notification-area) icon and the session notices show:
//! the same session and receiving state as `snapshot`, without the device list.
use super::*;
use extend_computer_agent::session::display::Mode;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    /// This device accepts connections.
    pub receiving: bool,
    /// Only devices on a USB cable can connect (Wi-Fi optimization is off).
    pub usb_only: bool,
    pub session: Option<ActiveSession>,
    /// A pairing or verification request waiting for an answer in the window.
    pub approval: Option<u64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveSession {
    pub id: u64,
    /// The other device's name.
    pub peer: String,
    /// The other device started the session.
    pub incoming: bool,
    /// The display mode, or `None` for keyboard and mouse.
    pub display: Option<Mode>,
    pub stage: Stage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Connecting,
    Reconnecting,
    Approval,
    Connected,
    Disconnecting,
}
impl Desktop {
    pub fn activity(&self) -> Activity {
        // One lock at a time: other paths take these in other orders.
        let (receiving, usb_only, session) = {
            let inner = self.inner.lock().unwrap();
            let session = inner
                .visible_job()
                .filter(|job| {
                    matches!(job.view.kind, SessionKind::Incoming | SessionKind::Outgoing)
                })
                .map(|job| job.view.clone());
            let receiving = inner.receiving();
            (receiving, receiving && !inner.permissions.wifi, session)
        };
        let session = session.map(|view| ActiveSession {
            id: view.id,
            peer: self.peer_name(view.peer.as_deref()),
            incoming: view.kind == SessionKind::Incoming,
            display: view.display,
            stage: match view.phase {
                Phase::Connecting if view.reconnecting => Stage::Reconnecting,
                Phase::Connecting => Stage::Connecting,
                Phase::Approval => Stage::Approval,
                Phase::Connected => Stage::Connected,
                Phase::Disconnecting => Stage::Disconnecting,
            },
        });
        Activity {
            receiving,
            usb_only,
            session,
            approval: self.approvals.current().map(|request| request.id),
        }
    }
    /// Sessions another device started that connected since the last call,
    /// oldest first, each once, for the status icon to announce. A session
    /// that already ended is still here, so one shorter than the icon's poll
    /// is announced too. Sessions this device started never appear.
    pub fn take_arrivals(&self) -> Vec<ActiveSession> {
        let arrivals = std::mem::take(&mut self.inner.lock().unwrap().arrivals);
        arrivals
            .into_iter()
            .map(|arrival| ActiveSession {
                id: arrival.id,
                peer: self.peer_name(arrival.peer.as_deref()),
                incoming: true,
                display: arrival.display,
                stage: Stage::Connected,
            })
            .collect()
    }
    fn peer_name(&self, id: Option<&str>) -> String {
        let Some(id) = id else {
            return "Another device".into();
        };
        self.devices
            .lock()
            .unwrap()
            .get(id)
            .map(|device| device.name.clone())
            .unwrap_or_else(|| format!("Device {}", &id[..8.min(id.len())]))
    }
}
/// An incoming session as it connected, until the status icon announces it.
pub(super) struct Arrival {
    id: u64,
    peer: Option<String>,
    display: Option<Mode>,
}
impl Arrival {
    pub(super) fn of(view: &Session) -> Self {
        Self {
            id: view.id,
            peer: view.peer.clone(),
            display: view.display,
        }
    }
}
/// Arrivals kept while nothing reads them, as in the CLI and tests.
const ARRIVALS: usize = 8;
impl Inner {
    pub(super) fn arrived(&mut self, arrival: Arrival) {
        if self.arrivals.len() == ARRIVALS {
            self.arrivals.remove(0);
        }
        self.arrivals.push(arrival);
    }
}
/// How this device is named in copy: "this Mac" on macOS, as in the window.
const HERE: &str = if cfg!(target_os = "macos") {
    "this Mac"
} else {
    "this computer"
};
impl Activity {
    /// Whether this device accepts connections, as one line.
    pub fn status(&self) -> &'static str {
        match (self.receiving, self.usb_only) {
            (false, _) => "Not accepting connections",
            (true, true) => "Accepting connections over USB only",
            (true, false) => "Accepting connections",
        }
    }
    /// Another device controls this one or shows a display on it.
    pub fn incoming(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.incoming && session.stage == Stage::Connected)
    }
}
impl ActiveSession {
    /// The session as one line, such as "Studio is controlling this Mac".
    pub fn describe(&self) -> String {
        let peer = &self.peer;
        match (self.stage, self.incoming, self.display) {
            (Stage::Connected, true, None) => format!("{peer} is controlling {HERE}"),
            (Stage::Connected, true, Some(Mode::Extend)) => {
                format!("{peer} is extending its display to {HERE}")
            }
            (Stage::Connected, true, Some(Mode::Mirror)) => {
                format!("{peer} is mirroring its screen to {HERE}")
            }
            (Stage::Connected, false, None) => format!("Sharing keyboard and mouse with {peer}"),
            (Stage::Connected, false, Some(Mode::Extend)) => format!("Extending to {peer}"),
            (Stage::Connected, false, Some(Mode::Mirror)) => format!("Mirroring to {peer}"),
            (Stage::Approval, ..) => format!("{peer} is waiting for approval"),
            (Stage::Reconnecting, ..) => format!("Reconnecting to {peer}…"),
            (Stage::Disconnecting, ..) => format!("Disconnecting from {peer}…"),
            (Stage::Connecting, ..) => format!("Connecting to {peer}…"),
        }
    }
    /// The label of the action that ends the session, as on its device card.
    pub fn stop_label(&self) -> &'static str {
        match (self.stage, self.incoming, self.display) {
            (Stage::Connected, false, None) => "Stop sharing",
            (Stage::Connected, false, Some(Mode::Extend)) => "Stop extending",
            (Stage::Connected, false, Some(Mode::Mirror)) => "Stop mirroring",
            (Stage::Connected | Stage::Disconnecting, true, _) => "Disconnect",
            _ => "Cancel",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(incoming: bool, display: Option<Mode>, stage: Stage) -> ActiveSession {
        ActiveSession {
            id: 1,
            peer: "Studio".into(),
            incoming,
            display,
            stage,
        }
    }
    #[test]
    fn the_icon_marks_only_connected_incoming_sessions() {
        for display in [None, Some(Mode::Extend), Some(Mode::Mirror)] {
            let marked = |incoming, stage| {
                Activity {
                    session: Some(session(incoming, display, stage)),
                    ..Default::default()
                }
                .incoming()
            };
            assert!(marked(true, Stage::Connected));
            assert!(!marked(true, Stage::Approval));
            assert!(!marked(true, Stage::Disconnecting));
            assert!(!marked(false, Stage::Connected));
        }
        assert!(!Activity::default().incoming());
    }
    #[test]
    fn announces_each_incoming_session_once_even_after_it_ends() {
        let temp = tempfile::tempdir().unwrap();
        let app = Desktop::new(temp.path().join("app"), temp.path().join("helper")).unwrap();
        // Sessions this device starts never announce themselves.
        let (id, _) = app
            .reserve(SessionKind::Outgoing, Some("studio-fingerprint".into()))
            .unwrap();
        app.stage(id, Phase::Connected, None);
        app.finish(id, Ok(()));
        assert!(app.take_arrivals().is_empty());
        for display in [None, Some(Mode::Extend), Some(Mode::Mirror)] {
            let (id, _) = app.reserve(SessionKind::Incoming, None).unwrap();
            app.inner.lock().unwrap().job.as_mut().unwrap().view.display = display;
            app.stage(id, Phase::Approval, Some("studio-fingerprint"));
            assert!(
                app.take_arrivals().is_empty(),
                "announced before connecting"
            );
            app.stage(id, Phase::Connected, None);
            // Keyboard and mouse sessions can report Connected again.
            app.stage(id, Phase::Connected, None);
            // The session ends before the icon's next look, and is still
            // announced, once.
            app.finish(id, Ok(()));
            let arrivals = app.take_arrivals();
            assert_eq!(arrivals.len(), 1, "{arrivals:?}");
            assert_eq!(arrivals[0].id, id);
            assert_eq!(arrivals[0].display, display);
            assert!(
                arrivals[0].describe().starts_with("Device studio-f is "),
                "{}",
                arrivals[0].describe()
            );
            assert!(app.take_arrivals().is_empty());
        }
        // A cancelled connection never connected, so it isn't announced.
        let (id, _) = app.reserve(SessionKind::Incoming, None).unwrap();
        app.disconnect();
        app.stage(id, Phase::Connected, Some("studio-fingerprint"));
        app.finish(id, Ok(()));
        assert!(app.take_arrivals().is_empty());
        // With no icon reading them, only the latest few are kept.
        for _ in 0..ARRIVALS + 3 {
            let (id, _) = app.reserve(SessionKind::Incoming, None).unwrap();
            app.stage(id, Phase::Connected, Some("studio-fingerprint"));
            app.finish(id, Ok(()));
        }
        let kept = app.take_arrivals();
        assert_eq!(kept.len(), ARRIVALS);
        assert!(kept.windows(2).all(|pair| pair[0].id < pair[1].id));
    }
    #[test]
    fn describes_sessions_and_their_stop_action_like_the_device_card() {
        let cases = [
            (
                true,
                None,
                Stage::Connected,
                "Studio is controlling",
                "Disconnect",
            ),
            (
                true,
                Some(Mode::Extend),
                Stage::Connected,
                "Studio is extending its display to",
                "Disconnect",
            ),
            (
                true,
                Some(Mode::Mirror),
                Stage::Connected,
                "Studio is mirroring its screen to",
                "Disconnect",
            ),
            (
                false,
                None,
                Stage::Connected,
                "Sharing keyboard and mouse with Studio",
                "Stop sharing",
            ),
            (
                false,
                Some(Mode::Extend),
                Stage::Connected,
                "Extending to Studio",
                "Stop extending",
            ),
            (
                false,
                Some(Mode::Mirror),
                Stage::Connected,
                "Mirroring to Studio",
                "Stop mirroring",
            ),
            (
                false,
                None,
                Stage::Connecting,
                "Connecting to Studio…",
                "Cancel",
            ),
            (
                false,
                None,
                Stage::Reconnecting,
                "Reconnecting to Studio…",
                "Cancel",
            ),
            (
                true,
                None,
                Stage::Approval,
                "Studio is waiting for approval",
                "Cancel",
            ),
            (
                true,
                None,
                Stage::Disconnecting,
                "Disconnecting from Studio…",
                "Disconnect",
            ),
        ];
        for (incoming, display, stage, line, stop) in cases {
            let session = session(incoming, display, stage);
            assert!(
                session.describe().starts_with(line),
                "{}",
                session.describe()
            );
            assert_eq!(session.stop_label(), stop, "{line}");
        }
        assert!(session(true, None, Stage::Connected)
            .describe()
            .ends_with(HERE));
    }
    #[test]
    fn states_whether_this_device_accepts_connections() {
        let mut activity = Activity::default();
        assert_eq!(activity.status(), "Not accepting connections");
        activity.receiving = true;
        assert_eq!(activity.status(), "Accepting connections");
        activity.usb_only = true;
        assert_eq!(activity.status(), "Accepting connections over USB only");
    }
}
