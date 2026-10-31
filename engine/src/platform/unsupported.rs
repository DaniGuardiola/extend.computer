//! Explicit fallbacks until an OS implements an adapter. Unscoped sockets keep
//! the normal system routing policy; unknown links and hardware stay neutral.
use crate::{device_info::DeviceKind, identity::Identity, link::Link};
use anyhow::Result;
use std::net::IpAddr;

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn load_identity(_: &str) -> Result<Identity> {
    anyhow::bail!("persistent identity adapter not implemented on this OS")
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn classify_link(_: IpAddr) -> Option<Link> {
    None
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn device_kind() -> Option<DeviceKind> {
    None
}

pub mod local_network {
    use socket2::SockRef;
    use std::{
        io,
        net::{SocketAddr, TcpListener, TcpStream},
        time::Duration,
    };

    pub fn connect_attached(_: &SocketAddr, _: Duration) -> io::Result<Option<TcpStream>> {
        Ok(None)
    }
    pub fn scope_to_local_interface(_: SockRef<'_>) -> io::Result<()> {
        Ok(())
    }
    pub fn bind_shared_listener(address: SocketAddr) -> io::Result<TcpListener> {
        TcpListener::bind(address)
    }
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
    // Windows reports ready through its own adapter.
    #[cfg_attr(windows, allow(dead_code))]
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
