//! Session states and valid forward transitions. IPC spelling remains stable.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum SessionKind {
    Pair,
    Unpair,
    Incoming,
    Outgoing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Phase {
    Connecting,
    Approval,
    Connected,
    Disconnecting,
}
#[derive(Clone, Serialize)]
pub struct Session {
    pub(super) id: u64,
    pub(super) kind: SessionKind,
    pub(super) phase: Phase,
    pub(super) peer: Option<String>,
}
impl Session {
    pub(super) fn advance(&mut self, next: Phase) -> bool {
        let valid = self.phase == next
            || (self.kind == SessionKind::Outgoing
                && self.phase == Phase::Connected
                && next == Phase::Connecting)
            || matches!(
                (self.phase, next),
                (Phase::Connecting, Phase::Approval | Phase::Connected)
                    | (Phase::Approval, Phase::Connected)
                    | (
                        Phase::Connecting | Phase::Approval | Phase::Connected,
                        Phase::Disconnecting
                    )
            );
        if !valid
            || (matches!(self.kind, SessionKind::Pair | SessionKind::Unpair)
                && next == Phase::Connected)
        {
            return false;
        }
        self.phase = next;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_outgoing_control_can_return_to_connecting() {
        for kind in [
            SessionKind::Outgoing,
            SessionKind::Incoming,
            SessionKind::Pair,
        ] {
            let mut session = Session {
                id: 1,
                kind,
                phase: Phase::Connected,
                peer: None,
            };
            assert_eq!(
                session.advance(Phase::Connecting),
                kind == SessionKind::Outgoing
            );
        }
    }
    #[test]
    fn cancelled_sessions_cannot_be_revived_by_late_callbacks() {
        let mut s = Session {
            id: 1,
            kind: SessionKind::Outgoing,
            phase: Phase::Connecting,
            peer: None,
        };
        assert!(s.advance(Phase::Connected));
        assert!(s.advance(Phase::Disconnecting));
        assert!(!s.advance(Phase::Connected));
        assert!(!s.advance(Phase::Approval));
        assert_eq!(serde_json::to_value(&s).unwrap()["phase"], "disconnecting");
    }
    #[test]
    fn pairing_cannot_enter_input_connected_state() {
        let mut s = Session {
            id: 1,
            kind: SessionKind::Pair,
            phase: Phase::Connecting,
            peer: None,
        };
        assert!(!s.advance(Phase::Connected));
        assert!(s.advance(Phase::Approval));
        assert!(!s.advance(Phase::Connected));
        assert_eq!(serde_json::to_value(&s).unwrap()["kind"], "pair");
    }
}
