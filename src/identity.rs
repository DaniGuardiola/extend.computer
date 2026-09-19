use anyhow::{ensure, Result};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

pub struct Identity {
    secret: Zeroizing<[u8; 32]>,
    public: [u8; 32],
}

impl Identity {
    pub fn generate() -> Self {
        let mut bytes = [0; 32];
        OsRng.fill_bytes(&mut bytes);
        Self::from_secret(bytes)
    }

    fn from_secret(bytes: [u8; 32]) -> Self {
        let public = PublicKey::from(&StaticSecret::from(bytes)).to_bytes();
        Self {
            secret: Zeroizing::new(bytes),
            public,
        }
    }

    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }
    pub fn public(&self) -> &[u8; 32] {
        &self.public
    }
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public)
    }

    /// Explicit debug-only storage. Never reads or exports a Keychain identity.
    #[cfg(all(feature = "dev-identity", debug_assertions, unix))]
    pub fn load_development(root: &std::path::Path) -> Result<Self> {
        use fs2::FileExt;
        use std::{
            fs,
            io::{Read, Write},
            os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        };
        fs::create_dir_all(root)?;
        ensure!(
            !fs::symlink_metadata(root)?.file_type().is_symlink(),
            "development identity directory must not be a symlink"
        );
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        let path = root.join("development-identity.key");
        // create_new refuses pre-existing files/symlinks; existing files are
        // checked again by inode after opening, before any content is read.
        let (mut file, created) = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let before = fs::symlink_metadata(&path)?;
                ensure!(
                    before.is_file() && before.mode() & 0o077 == 0,
                    "unsafe development identity permissions or file type"
                );
                let file = fs::OpenOptions::new().read(true).write(true).open(&path)?;
                let after = file.metadata()?;
                ensure!(
                    before.ino() == after.ino() && before.dev() == after.dev(),
                    "development identity changed during open"
                );
                (file, false)
            }
            Err(e) => return Err(e.into()),
        };
        file.lock_exclusive()?;
        let length = file.metadata()?.len();
        if created {
            let identity = Self::generate();
            file.write_all(identity.secret())?;
            file.sync_all()?;
            return Ok(identity);
        }
        ensure!(
            length == 32,
            "invalid development identity; refusing replacement"
        );
        let mut bytes = [0; 32];
        file.read_exact(&mut bytes)?;
        Ok(Self::from_secret(bytes))
    }

    #[cfg(target_os = "macos")]
    pub fn load_keychain(account: &str) -> Result<Self> {
        use security_framework::passwords::{get_generic_password, set_generic_password};
        const SERVICE: &str = "computer.extend.prototype.identity.v1";
        match get_generic_password(SERVICE, account) {
            Ok(secret) => {
                let secret = Zeroizing::new(secret);
                ensure!(
                    secret.len() == 32,
                    "invalid identity in Keychain; refusing replacement"
                );
                let mut bytes = [0; 32];
                bytes.copy_from_slice(&secret);
                Ok(Self::from_secret(bytes))
            }
            Err(error) if error.code() == -25300 => {
                let identity = Self::generate();
                set_generic_password(SERVICE, account, identity.secret())?;
                Ok(identity)
            }
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn load_keychain(_: &str) -> Result<Self> {
        anyhow::bail!("persistent identity adapter not implemented on this OS")
    }
}

pub fn fingerprint(public: &[u8]) -> String {
    hex::encode(Sha256::digest(public))
}
