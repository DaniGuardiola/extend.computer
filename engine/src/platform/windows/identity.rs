//! Persistent device identity in the Windows Credential Manager.
use crate::identity::Identity;
use anyhow::{ensure, Result};
use zeroize::Zeroizing;

pub(crate) fn load_identity(account: &str) -> Result<Identity> {
    const SERVICE: &str = "computer.extend.prototype.identity.v1";
    let entry = keyring::Entry::new(SERVICE, account)?;
    match entry.get_secret() {
        Ok(secret) => {
            let secret = Zeroizing::new(secret);
            ensure!(
                secret.len() == 32,
                "invalid identity in Credential Manager; refusing replacement"
            );
            let mut bytes = [0; 32];
            bytes.copy_from_slice(&secret);
            Ok(Identity::from_secret(bytes))
        }
        Err(keyring::Error::NoEntry) => {
            let identity = Identity::generate();
            entry.set_secret(identity.secret())?;
            Ok(identity)
        }
        Err(error) => Err(error.into()),
    }
}
