use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    /// Grants only diagnostic probes; never inherited by future input/video features.
    pub automatic_probe: bool,
    /// Explicit receiver-side full-control permission; old stores default to ask.
    #[serde(default)]
    pub automatic_input: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Records {
    pub peers: BTreeMap<String, Peer>,
    pub revoked: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pending_unpairs: BTreeSet<String>,
}

#[derive(Clone)]
pub struct TrustStore {
    root: PathBuf,
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
        let store = Self { root };
        store.load()?; // Corrupt state must fail closed.
        Ok(store)
    }
    pub fn load(&self) -> Result<Records> {
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
    pub fn remember(&self, id: &str, automatic_probe: bool) -> Result<()> {
        self.modify(|r| {
            ensure!(
                !r.revoked.contains(id),
                "device revoked; clear revocation locally before re-pairing"
            );
            r.pending_unpairs.remove(id);
            let automatic_input = r.peers.get(id).is_some_and(|p| p.automatic_input);
            r.peers.insert(
                id.to_owned(),
                Peer {
                    automatic_probe,
                    automatic_input,
                },
            );
            Ok(())
        })
    }
    pub fn allow_control(&self, id: &str) -> Result<()> {
        ensure!(
            id.len() == 64 && hex::decode(id)?.len() == 32,
            "expected full device fingerprint"
        );
        self.modify(|r| {
            ensure!(!r.revoked.contains(id), "device revoked");
            let peer = r.peers.entry(id.to_owned()).or_insert(Peer {
                automatic_probe: false,
                automatic_input: false,
            });
            peer.automatic_input = true;
            Ok(())
        })
    }
    pub fn require_control_consent(&self, id: &str) -> Result<()> {
        self.modify(|r| {
            let peer = r.peers.get_mut(id).context("unknown device")?;
            peer.automatic_input = false;
            Ok(())
        })
    }
    /// Forget pairing and remembered permissions without blocking future pairing.
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
        ensure!(
            id.len() == 64 && hex::decode(id)?.len() == 32,
            "expected full device fingerprint"
        );
        self.modify(|r| {
            ensure!(
                r.peers.contains_key(id) || r.pending_unpairs.contains(id),
                "unknown device"
            );
            r.peers.remove(id);
            r.pending_unpairs.insert(id.to_owned());
            Ok(())
        })
    }
    pub fn complete_unpair(&self, id: &str) -> Result<()> {
        self.modify(|r| {
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
        let mut records = self.load()?;
        edit(&mut records)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(&serde_json::to_vec_pretty(&records)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(self.root.join("trust.json"))?;
        Ok(())
    }
}
