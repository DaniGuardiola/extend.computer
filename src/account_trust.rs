//! Short-lived account membership. Never persisted as manual pairing.
use crate::trust::Peer;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct AccountTrust(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    scope: String,
    until: Option<Instant>,
    peers: BTreeMap<String, Peer>,
}
impl AccountTrust {
    pub fn replace(&self, scope: &str, fingerprints: &[String], lifetime: Duration) {
        let mut state = self.0.lock().unwrap();
        let keep = state.scope == scope && state.until.is_some_and(|until| until > Instant::now());
        let mut peers = BTreeMap::new();
        for id in fingerprints {
            if id.len() != 64 || hex::decode(id).map_or(true, |bytes| bytes.len() != 32) {
                continue;
            }
            let consent = keep && state.peers.get(id).is_some_and(|p| p.automatic_input);
            peers.insert(
                id.clone(),
                Peer {
                    automatic_probe: false,
                    automatic_input: consent,
                },
            );
        }
        state.peers = peers;
        state.scope = scope.into();
        state.until = Some(Instant::now() + lifetime.min(Duration::from_secs(90)));
    }
    pub fn clear(&self) {
        *self.0.lock().unwrap() = State::default();
    }
    pub(crate) fn peers(&self) -> BTreeMap<String, Peer> {
        let state = self.0.lock().unwrap();
        if state.until.is_some_and(|until| until > Instant::now()) {
            state.peers.clone()
        } else {
            BTreeMap::new()
        }
    }
    pub(crate) fn allow_control(&self, id: &str) -> bool {
        let mut state = self.0.lock().unwrap();
        if !state.until.is_some_and(|until| until > Instant::now()) {
            return false;
        }
        if let Some(peer) = state.peers.get_mut(id) {
            peer.automatic_input = true;
            true
        } else {
            false
        }
    }
    pub(crate) fn forget(&self, id: &str) {
        self.0.lock().unwrap().peers.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::TrustStore;
    #[test]
    fn membership_and_consent_never_become_manual_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let account = AccountTrust::default();
        let id = "a".repeat(64);
        account.replace("server/account", &[id.clone()], Duration::from_secs(60));
        let store = TrustStore::open(dir.path())
            .unwrap()
            .with_account_trust(account.clone());
        assert!(store.peer(&id).unwrap().is_some());
        assert!(!store.peer(&id).unwrap().unwrap().automatic_input);
        store.allow_control(&id).unwrap();
        assert!(store.peer(&id).unwrap().unwrap().automatic_input);
        assert!(TrustStore::open(dir.path())
            .unwrap()
            .peer(&id)
            .unwrap()
            .is_none());
        account.replace(
            "other-server/account",
            &[id.clone()],
            Duration::from_secs(60),
        );
        assert!(!store.peer(&id).unwrap().unwrap().automatic_input);
        account.clear();
        assert!(store.peer(&id).unwrap().is_none());
        assert!(
            store.allow_control(&id).is_err(),
            "Expired consent must not create manual trust"
        );
        account.replace("server/account", &[id.clone()], Duration::from_secs(60));
        store.complete_unpair(&id).unwrap();
        account.replace("server/account", &[id.clone()], Duration::from_secs(60));
        assert!(
            store.peer(&id).unwrap().is_none(),
            "Refresh must not undo an explicit unpair"
        );
    }
    #[test]
    fn expiry_revocation_and_manual_pairing_remain_authoritative() {
        let dir = tempfile::tempdir().unwrap();
        let account = AccountTrust::default();
        let id = "b".repeat(64);
        let store = TrustStore::open(dir.path())
            .unwrap()
            .with_account_trust(account.clone());
        account.replace("account", &[id.clone()], Duration::ZERO);
        assert!(store.peer(&id).unwrap().is_none());
        store.remember(&id, false).unwrap();
        account.replace("account", &[id.clone()], Duration::from_secs(60));
        account.clear();
        assert!(store.peer(&id).unwrap().is_some());
        store.revoke(&id).unwrap();
        account.replace("account", &[id], Duration::from_secs(60));
        assert!(store.load().unwrap().peers.is_empty());
    }
}
