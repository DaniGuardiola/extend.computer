//! Identify USB networking from its driver, never from a link-local address or
//! an assumed interface name. macOS exposes both host and device NCM endpoints.
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

// A wildcard listener cannot scope the SYN-ACK to the USB interface. Concrete
// companions share its port and get the interface binding after listen(),
// before any handshake. They appear and disappear with cable addresses.
pub(super) struct Listeners {
    listener: TcpListener,
    usb: Vec<(Ipv4Addr, u32, TcpListener)>,
    next_refresh: Instant,
}

impl Listeners {
    pub fn bind(port: u16) -> io::Result<Self> {
        let listener = extend_computer_agent::local_network::bind_tcp_listener(SocketAddr::from(
            ([0, 0, 0, 0], port),
        ))?;
        listener.set_nonblocking(true)?;
        let mut listeners = Self {
            listener,
            usb: Vec::new(),
            next_refresh: Instant::now(),
        };
        listeners.refresh();
        Ok(listeners)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn set_nonblocking(&self, value: bool) -> io::Result<()> {
        self.listener.set_nonblocking(value)?;
        for (_, _, listener) in &self.usb {
            listener.set_nonblocking(value)?;
        }
        Ok(())
    }

    pub fn accept(&mut self) -> io::Result<(TcpStream, SocketAddr)> {
        if Instant::now() >= self.next_refresh {
            self.refresh();
        }
        for (_, _, listener) in &self.usb {
            match listener.accept() {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                result => return result,
            }
        }
        self.listener.accept()
    }

    fn refresh(&mut self) {
        self.next_refresh = Instant::now() + Duration::from_secs(2);
        let interfaces = interfaces();
        self.usb.retain(|(ip, index, _)| {
            interfaces
                .iter()
                .any(|interface| interface.ip == *ip && interface.index == *index)
        });
        let Ok(address) = self.listener.local_addr() else {
            return;
        };
        for interface in interfaces {
            let ip = interface.ip;
            if self
                .usb
                .iter()
                .any(|(known, index, _)| *known == ip && *index == interface.index)
            {
                continue;
            }
            let address = SocketAddr::new(IpAddr::V4(ip), address.port());
            let result = extend_computer_agent::local_network::bind_tcp_listener(address).and_then(
                |listener| {
                    listener.set_nonblocking(true)?;
                    Ok(listener)
                },
            );
            match result {
                Ok(listener) => {
                    eprintln!("USB network listener: {address}");
                    self.usb.push((ip, interface.index, listener));
                }
                Err(error) => eprintln!("USB network listener {address} unavailable: {error}"),
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Interface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub index: u32,
}

impl Interface {
    pub fn contains(&self, peer: IpAddr) -> bool {
        let IpAddr::V4(peer) = peer else {
            return false;
        };
        let mask = u32::from(self.netmask);
        mask != 0 && peer != self.ip && u32::from(peer) & mask == u32::from(self.ip) & mask
    }
}

pub(super) fn interfaces() -> Vec<Interface> {
    let names = usb_interface_names();
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|interface| {
            interface.is_oper_up()
                && names.contains(&interface.name)
                && interface.index.is_some_and(|index| index != 0)
        })
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(address) if !address.ip.is_unspecified() => Some(Interface {
                name: interface.name,
                ip: address.ip,
                netmask: address.netmask,
                index: interface.index.unwrap(),
            }),
            _ => None,
        })
        .collect()
}

pub(super) fn socket_route(socket: &TcpStream) -> Option<&'static str> {
    let local = socket.local_addr().ok()?.ip();
    interfaces()
        .iter()
        .any(|interface| IpAddr::V4(interface.ip) == local)
        .then_some("usb")
}

/// Passive neighbor hints cover USB links where multicast discovery is blocked.
/// They never identify a peer: callers must use the regular pinned presence check.
pub(super) fn neighbor_candidates(port: u16) -> Vec<SocketAddr> {
    let mut candidates = Vec::new();
    for interface in interfaces().into_iter().take(4) {
        let Some(output) = read_neighbors(&interface.name) else {
            continue;
        };
        for address in parse_neighbors(&output, &interface, port) {
            if candidates.len() == 16 {
                return candidates;
            }
            if !candidates.contains(&address) {
                candidates.push(address);
            }
        }
    }
    candidates
}

fn read_neighbors(name: &str) -> Option<String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::mpsc,
    };
    let mut child = Command::new("/usr/sbin/arp")
        .args(["-an", "-i", name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(16 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let deadline = Instant::now() + Duration::from_millis(250);
    let successful = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    if !successful {
        return None;
    }
    let bytes = rx.recv_timeout(Duration::from_millis(20)).ok()?.ok()?;
    String::from_utf8(bytes).ok()
}

fn parse_neighbors(output: &str, interface: &Interface, port: u16) -> Vec<SocketAddr> {
    output
        .lines()
        .take(256)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _ = fields.next()?;
            let address = fields.next()?.strip_prefix('(')?.strip_suffix(')')?;
            if fields.next()? != "at" {
                return None;
            }
            let mac = fields.next()?;
            if mac == "(incomplete)" || fields.next()? != "on" || fields.next()? != interface.name {
                return None;
            }
            let mac: Vec<_> = mac.split(':').collect();
            if mac.len() != 6
                || !mac.iter().all(|part| {
                    (1..=2).contains(&part.len())
                        && part.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            {
                return None;
            }
            let ip: Ipv4Addr = address.parse().ok()?;
            let broadcast = u32::from(interface.ip) | !u32::from(interface.netmask);
            if !interface.contains(IpAddr::V4(ip))
                || ip.is_multicast()
                || u32::from(ip) == broadcast
            {
                return None;
            }
            Some(SocketAddr::from((ip, port)))
        })
        .take(16)
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn usb_interface_names() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn usb_interface_names() -> Vec<String> {
    use std::{ffi::c_void, ptr};
    type Object = u32;
    type Cf = *const c_void;
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const std::ffi::c_char) -> *mut c_void;
        fn IOServiceGetMatchingServices(
            port: u32,
            matching: *mut c_void,
            result: *mut Object,
        ) -> i32;
        fn IOIteratorNext(iterator: Object) -> Object;
        fn IOObjectRelease(object: Object) -> i32;
        fn IOObjectConformsTo(object: Object, class: *const std::ffi::c_char) -> u32;
        fn IORegistryEntryGetParentEntry(
            entry: Object,
            plane: *const std::ffi::c_char,
            parent: *mut Object,
        ) -> i32;
        fn IORegistryEntryCreateCFProperty(
            entry: Object,
            key: Cf,
            allocator: Cf,
            options: u32,
        ) -> Cf;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: Cf,
            text: *const std::ffi::c_char,
            encoding: u32,
        ) -> Cf;
        fn CFStringGetCString(
            string: Cf,
            buffer: *mut std::ffi::c_char,
            size: isize,
            encoding: u32,
        ) -> bool;
        fn CFGetTypeID(value: Cf) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFRelease(value: Cf);
    }
    struct OwnedObject(Object);
    impl Drop for OwnedObject {
        fn drop(&mut self) {
            unsafe { IOObjectRelease(self.0) };
        }
    }
    struct OwnedCf(Cf);
    impl Drop for OwnedCf {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }
    const UTF8: u32 = 0x0800_0100;
    let mut names = Vec::new();
    // Matching consumes its dictionary. Every iterator, entry and CF property
    // below owns one retain and releases it on each early-return path.
    unsafe {
        let matching = IOServiceMatching(c"IOEthernetInterface".as_ptr());
        if matching.is_null() {
            return names;
        }
        let mut iterator = 0;
        if IOServiceGetMatchingServices(0, matching, &mut iterator) != 0 || iterator == 0 {
            return names;
        }
        let iterator = OwnedObject(iterator);
        let key = CFStringCreateWithCString(ptr::null(), c"BSD Name".as_ptr(), UTF8);
        if key.is_null() {
            return names;
        }
        let key = OwnedCf(key);
        loop {
            let entry = IOIteratorNext(iterator.0);
            if entry == 0 {
                break;
            }
            let entry = OwnedObject(entry);
            let mut parent = 0;
            if IORegistryEntryGetParentEntry(entry.0, c"IOService".as_ptr(), &mut parent) != 0 {
                continue;
            }
            let parent = OwnedObject(parent);
            if IOObjectConformsTo(parent.0, c"AppleUSBNCMData".as_ptr()) == 0
                && IOObjectConformsTo(parent.0, c"AppleUSBDeviceNCM11Data".as_ptr()) == 0
                && IOObjectConformsTo(parent.0, c"AppleUSBDeviceNCMData".as_ptr()) == 0
            {
                continue;
            }
            let property = IORegistryEntryCreateCFProperty(entry.0, key.0, ptr::null(), 0);
            if property.is_null() {
                continue;
            }
            let property = OwnedCf(property);
            let mut buffer = [0u8; 64];
            if CFGetTypeID(property.0) != CFStringGetTypeID()
                || !CFStringGetCString(property.0, buffer.as_mut_ptr().cast(), 64, UTF8)
            {
                continue;
            }
            let end = buffer.iter().position(|&byte| byte == 0).unwrap_or(64);
            if let Ok(name) = std::str::from_utf8(&buffer[..end]) {
                // Restricted/private NCM interfaces are not app networking.
                if name.starts_with("en") && !names.iter().any(|value| value == name) {
                    names.push(name.to_owned());
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_subnet_hint_excludes_self_other_subnets_and_ipv6() {
        let interface = Interface {
            name: "en42".into(),
            ip: "169.254.226.32".parse().unwrap(),
            netmask: "255.255.0.0".parse().unwrap(),
            index: 42,
        };
        assert!(interface.contains("169.254.173.134".parse().unwrap()));
        for peer in ["169.254.226.32", "10.0.0.2", "::1"] {
            assert!(!interface.contains(peer.parse().unwrap()));
        }
    }

    #[test]
    fn passive_neighbors_require_resolved_usb_interface_and_exclude_self_and_broadcasts() {
        let interface = Interface {
            name: "en42".into(),
            ip: "169.254.226.32".parse().unwrap(),
            netmask: "255.255.0.0".parse().unwrap(),
            index: 42,
        };
        let output = "? (169.254.173.134) at 52:af:77:d:b1:cb on en42 [ethernet]\n\
            ? (169.254.226.32) at 52:af:77:d:b1:34 on en42 permanent [ethernet]\n\
            ? (169.254.173.133) at (incomplete) on en42 [ethernet]\n\
            ? (169.254.173.132) at 52:af:77:d:b1:cb on en0 [ethernet]\n\
            ? (169.254.255.255) at ff:ff:ff:ff:ff:ff on en42 [ethernet]\n\
            ? (224.0.0.251) at 1:0:5e:0:0:fb on en42 ifscope permanent [ethernet]\n\
            ? (10.0.0.2) at 52:af:77:d:b1:cb on en42 [ethernet]\n";
        assert_eq!(
            parse_neighbors(output, &interface, 48178),
            vec!["169.254.173.134:48178".parse().unwrap()]
        );
    }
}
