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
        let mut peers = BTreeMap::new();
        for id in fingerprints {
            if id.len() != 64 || hex::decode(id).map_or(true, |bytes| bytes.len() != 32) {
                continue;
            }
            peers.insert(id.clone(), Peer::default());
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
    pub(crate) fn forget(&self, id: &str) {
        self.0.lock().unwrap().peers.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::TrustStore;
    #[test]
    fn membership_grants_access_without_creating_manual_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let account = AccountTrust::default();
        let id = "a".repeat(64);
        account.replace(
            "server/account",
            std::slice::from_ref(&id),
            Duration::from_secs(60),
        );
        let store = TrustStore::open(dir.path())
            .unwrap()
            .with_account_trust(account.clone());
        assert!(store.peer(&id).unwrap().is_some());
        assert!(TrustStore::open(dir.path())
            .unwrap()
            .peer(&id)
            .unwrap()
            .is_none());
        account.clear();
        assert!(store.peer(&id).unwrap().is_none());
        account.replace(
            "server/account",
            std::slice::from_ref(&id),
            Duration::from_secs(60),
        );
        store.complete_unpair(&id).unwrap();
        account.replace(
            "server/account",
            std::slice::from_ref(&id),
            Duration::from_secs(60),
        );
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
        account.replace("account", std::slice::from_ref(&id), Duration::ZERO);
        assert!(store.peer(&id).unwrap().is_none());
        store.remember(&id).unwrap();
        account.replace(
            "account",
            std::slice::from_ref(&id),
            Duration::from_secs(60),
        );
        account.clear();
        assert!(store.peer(&id).unwrap().is_some());
        store.revoke(&id).unwrap();
        account.replace("account", &[id], Duration::from_secs(60));
        assert!(store.load().unwrap().peers.is_empty());
    }
}
