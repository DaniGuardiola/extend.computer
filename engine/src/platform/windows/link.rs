//! Wi-Fi and Ethernet classification from the adapter owning an address.
use crate::link::Link;
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR},
    NetworkManagement::{
        IpHelper::{
            GetAdaptersAddresses, GetIfEntry2, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
            GAA_FLAG_SKIP_MULTICAST, IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211,
            IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2,
        },
        Ndis::{NdisPhysicalMedium802_3, NET_LUID_LH},
    },
    Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6},
};

pub(crate) fn classify_link(address: IpAddr) -> Option<Link> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    // u64 storage keeps the adapter list aligned. Adapters can appear between
    // the size report and the copy, so retry a few times.
    let mut buffer = Vec::<u64>::new();
    let mut size = 16 * 1024;
    for _ in 0..3 {
        buffer.resize((size as usize).div_ceil(8), 0);
        let list = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // The buffer holds at least `size` bytes throughout this call.
        let result =
            unsafe { GetAdaptersAddresses(AF_UNSPEC.into(), flags, ptr::null(), list, &mut size) };
        match result {
            NO_ERROR => return owner_link(list, address),
            ERROR_BUFFER_OVERFLOW => continue,
            _ => return None,
        }
    }
    None
}

fn owner_link(mut adapter: *const IP_ADAPTER_ADDRESSES_LH, address: IpAddr) -> Option<Link> {
    // Every adapter and address node lives in the caller's buffer.
    while let Some(current) = unsafe { adapter.as_ref() } {
        adapter = current.Next;
        let mut unicast = current.FirstUnicastAddress;
        while let Some(entry) = unsafe { unicast.as_ref() } {
            unicast = entry.Next;
            if ip(entry.Address.lpSockaddr) == Some(address) {
                return Some(match current.IfType {
                    IF_TYPE_IEEE80211 => Link::Wifi,
                    IF_TYPE_ETHERNET_CSMACD if wired(current.Luid) => Link::Ethernet,
                    _ => Link::Other,
                });
            }
        }
    }
    None
}

/// Bluetooth PAN, Hyper-V and VPN adapters also report the Ethernet interface
/// type; only a hardware interface on an 802.3 medium is a cable.
fn wired(luid: NET_LUID_LH) -> bool {
    // MIB_IF_ROW2 is plain data; GetIfEntry2 fills it for the given LUID.
    let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
    row.InterfaceLuid = luid;
    let status = unsafe { GetIfEntry2(&mut row) };
    // Bit 0 of InterfaceAndOperStatusFlags is HardwareInterface.
    status == NO_ERROR
        && row.PhysicalMediumType == NdisPhysicalMedium802_3
        && row.InterfaceAndOperStatusFlags._bitfield & 1 != 0
}

fn ip(address: *const SOCKADDR) -> Option<IpAddr> {
    // Each sockaddr is stored with the full size of its address family.
    match unsafe { address.as_ref() }?.sa_family {
        AF_INET => {
            let v4 = unsafe { &*address.cast::<SOCKADDR_IN>() };
            Some(Ipv4Addr::from(unsafe { v4.sin_addr.S_un.S_addr }.to_ne_bytes()).into())
        }
        AF_INET6 => {
            let v6 = unsafe { &*address.cast::<SOCKADDR_IN6>() };
            Some(Ipv6Addr::from(unsafe { v6.sin6_addr.u.Byte }).into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn classifies_the_default_route_interface() {
        // Connecting a UDP socket only selects a route; nothing is sent.
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        if socket.connect("192.0.2.1:9").is_err() {
            return;
        }
        let local = socket.local_addr().unwrap().ip();
        eprintln!("default route {local}: {:?}", super::classify_link(local));
        assert!(super::classify_link(local).is_some());
    }
}
