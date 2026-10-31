//! Wi-Fi and Ethernet classification through SystemConfiguration.
use crate::link::Link;
use std::{ffi::c_void, ffi::CStr, net::IpAddr, ptr};

type CFTypeRef = *const c_void;
#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {
    fn SCNetworkInterfaceCopyAll() -> CFTypeRef;
    fn SCNetworkInterfaceGetBSDName(interface: CFTypeRef) -> CFTypeRef;
    fn SCNetworkInterfaceGetInterfaceType(interface: CFTypeRef) -> CFTypeRef;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut u8, size: isize, encoding: u32) -> u8;
    fn CFRelease(value: CFTypeRef);
}
const UTF8: u32 = 0x08000100;

fn string(value: CFTypeRef) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let mut buffer = [0u8; 64];
    unsafe { CFStringGetCString(value, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) != 0 }
        .then(|| CStr::from_bytes_until_nul(&buffer).ok())
        .flatten()
        .and_then(|s| s.to_str().ok())
        .map(String::from)
}

/// The BSD name (en0, en7…) of the interface that owns this address.
fn interface_name(address: IpAddr) -> Option<String> {
    let mut list = ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut entry = list;
    while !entry.is_null() {
        let current = unsafe { &*entry };
        entry = current.ifa_next;
        let Some(socket) = (unsafe { current.ifa_addr.as_ref() }) else {
            continue;
        };
        let ip = match socket.sa_family as i32 {
            libc::AF_INET => {
                let v4 = unsafe { &*(current.ifa_addr as *const libc::sockaddr_in) };
                IpAddr::from(u32::from_be(v4.sin_addr.s_addr).to_be_bytes())
            }
            libc::AF_INET6 => {
                let v6 = unsafe { &*(current.ifa_addr as *const libc::sockaddr_in6) };
                IpAddr::from(v6.sin6_addr.s6_addr)
            }
            _ => continue,
        };
        if ip == address {
            found = unsafe { CStr::from_ptr(current.ifa_name) }
                .to_str()
                .ok()
                .map(String::from);
            break;
        }
    }
    unsafe { libc::freeifaddrs(list) };
    found
}

pub(crate) fn classify_link(address: IpAddr) -> Option<Link> {
    let name = interface_name(address)?;
    unsafe {
        let all = SCNetworkInterfaceCopyAll();
        if all.is_null() {
            return None;
        }
        let mut link = None;
        for index in 0..CFArrayGetCount(all).clamp(0, 256) {
            let interface = CFArrayGetValueAtIndex(all, index);
            if string(SCNetworkInterfaceGetBSDName(interface)).as_deref() != Some(&name) {
                continue;
            }
            link = Some(
                match string(SCNetworkInterfaceGetInterfaceType(interface)).as_deref() {
                    Some("IEEE80211") => Link::Wifi,
                    Some("Ethernet") => Link::Ethernet,
                    _ => Link::Other,
                },
            );
            break;
        }
        CFRelease(all);
        // Virtual interfaces (VPN tunnels, bridges) have no hardware entry.
        Some(link.unwrap_or(Link::Other))
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
    #[test]
    fn classifies_loopback_owner_without_panicking() {
        assert_eq!(
            super::interface_name("127.0.0.1".parse().unwrap()).as_deref(),
            Some("lo0")
        );
    }
}
