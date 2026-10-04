//! Explicit fallbacks until persistent identity and Wi-Fi adapters are implemented.
use crate::identity::Identity;
use anyhow::Result;

pub(crate) fn load_identity(_: &str) -> Result<Identity> {
    anyhow::bail!("persistent identity adapter not implemented on this OS")
}

pub mod low_jitter {
    use anyhow::Result;
    use std::{net::IpAddr, path::PathBuf};

    pub struct Lease;
    impl Lease {
        pub fn maintain(&mut self) -> Result<()> {
            Ok(())
        }
    }

    pub fn configure_app_helper(_: PathBuf) -> Result<()> {
        Ok(())
    }
    pub fn required_for_peer(_: IpAddr) -> Result<bool> {
        Ok(false)
    }
    pub fn ready() -> bool {
        false
    }
    pub fn require_ready_for_peer(_: IpAddr) -> Result<()> {
        Ok(())
    }
    pub fn start_required(_: IpAddr) -> Result<Option<Lease>> {
        Ok(None)
    }
    pub fn start_for_peer(_: IpAddr, _: bool) -> Option<Lease> {
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn remote_peers_do_not_launch_macos_helpers() {
            let peer = "192.0.2.1".parse().unwrap();
            assert!(!required_for_peer(peer).unwrap());
            assert!(!ready());
            require_ready_for_peer(peer).unwrap();
            assert!(start_required(peer).unwrap().is_none());
            assert!(start_for_peer(peer, true).is_none());
        }
    }
}
