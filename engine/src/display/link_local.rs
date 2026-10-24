//! ICE rejects IPv4 link-local candidates. Keep those addresses on the physical
//! socket and present a private, per-session tuple to ICE instead. The peer IP
//! comes from the signaling connection; SDP cannot select another physical IP.
use anyhow::{ensure, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[derive(Clone, Copy)]
pub(super) struct Route {
    local: SocketAddr,
    peer: Ipv4Addr,
}

fn logical(ip: Ipv4Addr) -> Ipv4Addr {
    let bytes = ip.octets();
    Ipv4Addr::new(10, 254, bytes[2], bytes[3])
}

impl Route {
    pub fn new(local: SocketAddr, peer: IpAddr) -> Result<Option<Self>> {
        let (IpAddr::V4(local_ip), IpAddr::V4(peer)) = (local.ip(), peer) else {
            return Ok(None);
        };
        if !local_ip.is_link_local() && !peer.is_link_local() {
            return Ok(None);
        }
        ensure!(
            local_ip.is_link_local() && peer.is_link_local() && local_ip != peer,
            "Link-local display requires distinct directly connected IPv4 endpoints"
        );
        Ok(Some(Self { local, peer }))
    }

    pub fn local_candidate(self) -> SocketAddr {
        let IpAddr::V4(ip) = self.local.ip() else {
            unreachable!()
        };
        SocketAddr::new(logical(ip).into(), self.local.port())
    }

    pub fn physical_destination(self, destination: SocketAddr) -> Result<SocketAddr> {
        ensure!(
            destination.ip() == IpAddr::V4(logical(self.peer)) && destination.port() != 0,
            "Display candidate does not match the signaling peer"
        );
        Ok(SocketAddr::new(self.peer.into(), destination.port()))
    }

    pub fn received_source(self, source: SocketAddr) -> Option<SocketAddr> {
        (source.ip() == IpAddr::V4(self.peer) && source.port() != 0)
            .then(|| SocketAddr::new(logical(self.peer).into(), source.port()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_is_pinned_to_physical_peer_and_preserves_udp_ports() {
        let route = Route::new(
            "169.254.226.32:55555".parse().unwrap(),
            "169.254.173.134".parse().unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            route.local_candidate(),
            "10.254.226.32:55555".parse().unwrap()
        );
        assert_eq!(
            route
                .physical_destination("10.254.173.134:44444".parse().unwrap())
                .unwrap(),
            "169.254.173.134:44444".parse().unwrap()
        );
        assert_eq!(
            route.received_source("169.254.173.134:44444".parse().unwrap()),
            Some("10.254.173.134:44444".parse().unwrap())
        );
        for address in ["10.254.173.135:44444", "8.8.8.8:44444", "10.254.173.134:0"] {
            assert!(route
                .physical_destination(address.parse().unwrap())
                .is_err());
        }
        for address in [
            "169.254.173.135:44444",
            "10.254.173.134:44444",
            "169.254.173.134:0",
        ] {
            assert!(route.received_source(address.parse().unwrap()).is_none());
        }
        assert!(Route::new(
            "192.168.1.1:1234".parse().unwrap(),
            "192.168.1.2".parse().unwrap()
        )
        .unwrap()
        .is_none());
        assert!(Route::new(
            "169.254.1.1:1234".parse().unwrap(),
            "169.254.1.1".parse().unwrap()
        )
        .is_err());
    }
}
