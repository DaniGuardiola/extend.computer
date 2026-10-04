use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// A verified pairing. Older permission flags are accepted only for migration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(from = "LegacyPeer")]
pub struct Peer {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPeer {
    #[serde(default, rename = "automatic_probe")]
    _probe: bool,
    #[serde(default, rename = "automatic_input")]
    _input: bool,
}
impl From<LegacyPeer> for Peer {
    fn from(_: LegacyPeer) -> Self {
        Self {}
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Records {
    pub peers: BTreeMap<String, Peer>,
    pub revoked: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pending_unpairs: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub account_blocked: BTreeSet<String>,
}

#[derive(Clone)]
pub struct TrustStore {
    root: PathBuf,
    account: crate::account_trust::AccountTrust,
}

impl TrustStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        let store = Self {
            root,
            account: Default::default(),
        };
        store.load()?; // Corrupt state must fail closed.
        Ok(store)
    }
    pub fn with_account_trust(mut self, account: crate::account_trust::AccountTrust) -> Self {
        self.account = account;
        self
    }
    pub fn is_account_peer(&self, id: &str) -> Result<bool> {
        Ok(!self.load_records()?.peers.contains_key(id) && self.account.peers().contains_key(id))
    }
    pub fn load(&self) -> Result<Records> {
        let mut records = self.load_records()?;
        for (id, peer) in self.account.peers() {
            if !records.revoked.contains(&id)
                && !records.pending_unpairs.contains(&id)
                && !records.account_blocked.contains(&id)
            {
                records.peers.entry(id).or_insert(peer);
            }
        }
        Ok(records)
    }
    fn load_records(&self) -> Result<Records> {
        match fs::read(self.root.join("trust.json")) {
            Ok(data) => {
                ensure!(data.len() < 1024 * 1024, "trust store too large");
                serde_json::from_slice(&data).context("invalid trust store")
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Records::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn peer(&self, id: &str) -> Result<Option<Peer>> {
        let data = self.load()?;
        ensure!(!data.revoked.contains(id), "device revoked");
        Ok(data.peers.get(id).cloned())
    }
    pub fn is_revoked(&self, id: &str) -> Result<bool> {
        Ok(self.load()?.revoked.contains(id))
    }
    pub fn remember(&self, id: &str) -> Result<()> {
        self.modify(|r| {
            ensure!(
                !r.revoked.contains(id),
                "device revoked; clear revocation locally before re-pairing"
            );
            r.pending_unpairs.remove(id);
            r.account_blocked.remove(id);
            r.peers.insert(id.to_owned(), Peer::default());
            Ok(())
        })
    }
    /// Forget pairing without blocking future pairing.
    /// Explicit revocation is separate and must never be cleared by this action.
    pub fn forget(&self, id: &str) -> Result<()> {
        ensure!(
            id.len() == 64 && hex::decode(id)?.len() == 32,
            "expected full device fingerprint"
        );
        self.modify(|r| {
            r.peers.remove(id);
            Ok(())
        })
    }
    /// Atomically remove local access and retain only a notification target.
    pub fn begin_unpair(&self, id: &str) -> Result<()> {
        let account_peer = self.is_account_peer(id)?;
        self.account.forget(id);
        ensure!(
            id.len() == 64 && hex::decode(id)?.len() == 32,
            "expected full device fingerprint"
        );
        self.modify(|r| {
            ensure!(
                account_peer || r.peers.contains_key(id) || r.pending_unpairs.contains(id),
                "unknown device"
            );
            r.peers.remove(id);
            r.pending_unpairs.insert(id.to_owned());
            if account_peer {
                r.account_blocked.insert(id.to_owned());
            }
            Ok(())
        })
    }
    pub fn complete_unpair(&self, id: &str) -> Result<()> {
        let account_peer = self.account.peers().contains_key(id);
        self.account.forget(id);
        self.modify(|r| {
            if account_peer {
                r.account_blocked.insert(id.to_owned());
            }
            r.peers.remove(id);
            r.pending_unpairs.remove(id);
            Ok(())
        })
    }
    pub fn revoke(&self, id: &str) -> Result<()> {
        ensure!(
            id.len() == 64 && hex::decode(id)?.len() == 32,
            "expected full device fingerprint"
        );
        self.modify(|r| {
            r.peers.remove(id);
            r.revoked.insert(id.to_owned());
            Ok(())
        })
    }
    fn modify(&self, edit: impl FnOnce(&mut Records) -> Result<()>) -> Result<()> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("trust.lock"))?;
        lock.lock_exclusive()?;
        let mut records = self.load_records()?;
        edit(&mut records)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(&serde_json::to_vec_pretty(&records)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(self.root.join("trust.json"))?;
        Ok(())
    }
}
