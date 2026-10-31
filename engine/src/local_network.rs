//! Keep directly connected peers on their physical interface when a VPN owns
//! an overlapping route. The binding is socket-scoped and changes no routes.
use crate::platform::local_network as platform;
use std::{
    io,
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    time::Duration,
};

/// Connect to a peer, selecting a directly attached IPv4 interface on macOS.
/// Other destinations and platforms retain the normal system routing policy.
pub fn connect_timeout(address: &SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
    match platform::connect_attached(address, timeout)? {
        Some(stream) => Ok(stream),
        None => TcpStream::connect_timeout(address, timeout),
    }
}

/// Bind a display datagram endpoint to the interface owning its local address.
pub fn bind_udp(address: SocketAddr) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(address)?;
    platform::scope_to_local_interface((&socket).into())?;
    Ok(socket)
}

/// Keep replies from an accepted TCP stream on its concrete local interface.
/// Loopback, unspecified addresses, and tunnel endpoints are left unchanged.
pub fn bind_stream_to_local_interface(stream: &TcpStream) -> io::Result<()> {
    platform::scope_to_local_interface(stream.into())
}

/// Scope a concrete listening address after the listener has begun listening.
/// macOS listen() clears IP_BOUND_IF; scoping it earlier loses the binding.
pub fn bind_listener_to_local_interface(listener: &TcpListener) -> io::Result<()> {
    platform::scope_to_local_interface(listener.into())
}

/// Create a listener that can share its port with concrete interface listeners
/// on macOS. A wildcard listener remains unscoped for all other interfaces.
pub fn bind_tcp_listener(address: SocketAddr) -> io::Result<TcpListener> {
    let listener = platform::bind_shared_listener(address)?;
    bind_listener_to_local_interface(&listener)?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn loopback_tcp_and_udp_keep_normal_behavior() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream =
            connect_timeout(&listener.local_addr().unwrap(), Duration::from_secs(1)).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        bind_stream_to_local_interface(&accepted).unwrap();
        assert_eq!(stream.peer_addr().unwrap(), accepted.local_addr().unwrap());
        let socket = bind_udp("127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(socket.local_addr().unwrap().ip().is_loopback());
    }
}
