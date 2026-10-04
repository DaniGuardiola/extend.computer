//! Persistent device identity in the macOS Keychain.
use crate::identity::Identity;
use anyhow::{ensure, Result};
use zeroize::Zeroizing;

pub(crate) fn load_identity(account: &str) -> Result<Identity> {
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
            Ok(Identity::from_secret(bytes))
        }
        Err(error) if error.code() == -25300 => {
            let identity = Identity::generate();
            set_generic_password(SERVICE, account, identity.secret())?;
            Ok(identity)
        }
        Err(error) => Err(error.into()),
    }
}
