//! macOS socket scoping with IP_BOUND_IF. A VPN can own a route overlapping a
//! directly attached subnet; scoping keeps those peers on the physical link.
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::{
    ffi::CStr,
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    os::fd::AsRawFd,
    time::Duration,
};

/// Connect over the interface directly attached to an IPv4 peer, if any.
pub fn connect_attached(address: &SocketAddr, timeout: Duration) -> io::Result<Option<TcpStream>> {
    let SocketAddr::V4(peer) = address else {
        return Ok(None);
    };
    let Some(interface) = interface_for_peer(*peer.ip())? else {
        return Ok(None);
    };
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    // macOS source-address binding can change IP_BOUND_IF to a VPN's
    // interface. Apply the explicit interface after binding the source.
    socket.bind(&SocketAddr::from((interface.address, 0)).into())?;
    bind_interface(&socket, interface.index)?;
    socket.connect_timeout(&(*address).into(), timeout)?;
    Ok(Some(socket.into()))
}

/// Scope a socket to the interface owning its concrete IPv4 local address.
pub fn scope_to_local_interface(socket: SockRef<'_>) -> io::Result<()> {
    if let Some(local) = socket.local_addr()?.as_socket_ipv4() {
        if let Some(interface) = interface_for_local(*local.ip())? {
            bind_interface(&*socket, interface.index)?;
        }
    }
    Ok(())
}

/// Bind a listener that can share its port with concrete interface listeners.
pub fn bind_shared_listener(address: SocketAddr) -> io::Result<TcpListener> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    // macOS permits a wildcard and distinct concrete addresses to coexist
    // with SO_REUSEADDR alone. Avoid SO_REUSEPORT so duplicate app listeners
    // on the same address still fail instead of sharing incoming sessions.
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    socket.listen(128)?;
    Ok(socket.into())
}

#[derive(Clone, Debug)]
struct Interface {
    index: u32,
    address: Ipv4Addr,
    mask: u32,
}

fn eligible(name: &str, flags: u32) -> bool {
    flags & libc::IFF_UP as u32 != 0
        && flags & libc::IFF_RUNNING as u32 != 0
        && flags & (libc::IFF_LOOPBACK | libc::IFF_POINTOPOINT) as u32 == 0
        && ![
            "lo", "utun", "tun", "tap", "ppp", "ipsec", "gif", "stf", "awdl", "llw",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn ipv4(address: *const libc::sockaddr) -> Option<Ipv4Addr> {
    // getifaddrs owns these aligned sockaddr values until freeifaddrs.
    if address.is_null() || unsafe { (*address).sa_family } as i32 != libc::AF_INET {
        return None;
    }
    let address = unsafe { &*address.cast::<libc::sockaddr_in>() };
    Some(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()))
}

fn interfaces() -> io::Result<Vec<Interface>> {
    struct Addresses(*mut libc::ifaddrs);
    impl Drop for Addresses {
        fn drop(&mut self) {
            // Only successfully obtained lists are wrapped in Addresses.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let mut head = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let addresses = Addresses(head);
    let mut current = addresses.0;
    let mut result = Vec::new();
    while !current.is_null() {
        // Every node and name remain alive for the duration of Addresses.
        let entry = unsafe { &*current };
        current = entry.ifa_next;
        if entry.ifa_name.is_null() {
            continue;
        }
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }.to_string_lossy();
        if !eligible(&name, entry.ifa_flags) {
            continue;
        }
        let (Some(address), Some(mask)) = (ipv4(entry.ifa_addr), ipv4(entry.ifa_netmask)) else {
            continue;
        };
        let mask = u32::from(mask);
        // Ignore default-like and malformed netmasks: they cannot prove
        // that the destination is directly attached to this interface.
        if address.is_loopback()
            || address.is_unspecified()
            || mask == 0
            || (!mask)
                .checked_add(1)
                .is_some_and(|value| value & !mask != 0)
        {
            continue;
        }
        let index = unsafe { libc::if_nametoindex(entry.ifa_name) };
        if index != 0 {
            result.push(Interface {
                index,
                address,
                mask,
            });
        }
    }
    Ok(result)
}

fn select_peer(interfaces: &[Interface], peer: Ipv4Addr) -> Option<&Interface> {
    if peer.is_loopback() || peer.is_unspecified() || peer.is_multicast() {
        return None;
    }
    if interfaces.iter().any(|interface| interface.address == peer) {
        return None;
    }
    let peer = u32::from(peer);
    interfaces
        .iter()
        .filter(|interface| peer & interface.mask == u32::from(interface.address) & interface.mask)
        .max_by_key(|interface| interface.mask.count_ones())
}

fn interface_for_peer(peer: Ipv4Addr) -> io::Result<Option<Interface>> {
    Ok(select_peer(&interfaces()?, peer).cloned())
}

fn interface_for_local(local: Ipv4Addr) -> io::Result<Option<Interface>> {
    if local.is_loopback() || local.is_unspecified() {
        return Ok(None);
    }
    Ok(interfaces()?
        .into_iter()
        .find(|interface| interface.address == local))
}

fn bind_interface(socket: &impl AsRawFd, index: u32) -> io::Result<()> {
    // index and descriptor remain valid throughout this synchronous call.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_BOUND_IF,
            (&index as *const u32).cast(),
            std::mem::size_of_val(&index) as libc::socklen_t,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_network::{bind_tcp_listener, bind_udp, connect_timeout};

    fn interface(index: u32, address: &str, mask: &str) -> Interface {
        Interface {
            index,
            address: address.parse().unwrap(),
            mask: u32::from(mask.parse::<Ipv4Addr>().unwrap()),
        }
    }

    #[test]
    fn physical_link_wins_despite_vpn_overlapping_routes() {
        let up = (libc::IFF_UP | libc::IFF_RUNNING) as u32;
        assert!(eligible("en10", up));
        assert!(!eligible("utun5", up));
        assert!(!eligible("en0", up | libc::IFF_POINTOPOINT as u32));
        assert!(!eligible("lo0", up | libc::IFF_LOOPBACK as u32));
        assert!(!eligible("en10", 0));
        let interfaces = [
            interface(6, "192.168.1.12", "255.255.255.0"),
            interface(32, "169.254.226.32", "255.255.0.0"),
        ];
        assert_eq!(
            select_peer(&interfaces, "169.254.173.134".parse().unwrap())
                .unwrap()
                .index,
            32
        );
        assert!(select_peer(&interfaces, "203.0.113.2".parse().unwrap()).is_none());
        assert!(select_peer(&interfaces, "127.0.0.1".parse().unwrap()).is_none());
        assert!(select_peer(&interfaces, "169.254.226.32".parse().unwrap()).is_none());
    }

    #[test]
    fn most_specific_subnet_wins() {
        let interfaces = [
            interface(1, "10.0.0.1", "255.0.0.0"),
            interface(2, "10.20.30.1", "255.255.255.0"),
        ];
        assert_eq!(
            select_peer(&interfaces, "10.20.30.2".parse().unwrap())
                .unwrap()
                .index,
            2
        );
    }

    #[test]
    fn datagram_binding_survives_source_address_binding() {
        // Check the actual order against all active physical interfaces.
        // A VPN can implicitly set another index during UdpSocket::bind.
        for interface in interfaces().unwrap() {
            let socket = bind_udp((interface.address, 0).into()).unwrap();
            let mut index: u32 = 0;
            let mut length = std::mem::size_of_val(&index) as libc::socklen_t;
            assert_eq!(
                unsafe {
                    libc::getsockopt(
                        socket.as_raw_fd(),
                        libc::IPPROTO_IP,
                        libc::IP_BOUND_IF,
                        (&mut index as *mut u32).cast(),
                        &mut length,
                    )
                },
                0
            );
            assert_eq!(index, interface.index);
            assert_eq!(socket.local_addr().unwrap().ip(), interface.address);
        }
    }

    #[test]
    fn concrete_listener_is_scoped_after_listen() {
        for interface in interfaces().unwrap() {
            let listener = bind_tcp_listener((interface.address, 0).into()).unwrap();
            let mut index: u32 = 0;
            let mut length = std::mem::size_of_val(&index) as libc::socklen_t;
            assert_eq!(
                unsafe {
                    libc::getsockopt(
                        listener.as_raw_fd(),
                        libc::IPPROTO_IP,
                        libc::IP_BOUND_IF,
                        (&mut index as *mut u32).cast(),
                        &mut length,
                    )
                },
                0
            );
            assert_eq!(index, interface.index);
        }
    }

    #[test]
    fn wildcard_and_concrete_listeners_share_port() {
        let wildcard = bind_tcp_listener("0.0.0.0:0".parse().unwrap()).unwrap();
        let concrete =
            bind_tcp_listener((Ipv4Addr::LOCALHOST, wildcard.local_addr().unwrap().port()).into())
                .unwrap();
        wildcard.set_nonblocking(true).unwrap();
        let stream = connect_timeout(
            &concrete.local_addr().unwrap(),
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        let (accepted, _) = concrete.accept().unwrap();
        assert_eq!(accepted.peer_addr().unwrap(), stream.local_addr().unwrap());
        assert_eq!(
            wildcard.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            bind_tcp_listener(wildcard.local_addr().unwrap())
                .unwrap_err()
                .kind(),
            io::ErrorKind::AddrInUse
        );
    }
}
