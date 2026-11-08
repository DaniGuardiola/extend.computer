//! What the menu bar (or notification-area) icon shows: the same session and
//! receiving state as `snapshot`, without the device list.
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
            peer: view.peer.as_deref().map_or_else(
                || "Another device".into(),
                |peer| peer_name(&self.devices.lock().unwrap(), peer),
            ),
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
fn peer_name(devices: &BTreeMap<String, Device>, id: &str) -> String {
    devices
        .get(id)
        .map(|device| device.name.clone())
        .unwrap_or_else(|| format!("Device {}", &id[..8.min(id.len())]))
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
