//! macOS-only, opt-in arrival timestamps; no packet payload is retained.
use std::{
    io,
    net::{SocketAddr, UdpSocket},
    os::fd::AsRawFd,
};

// Darwin sys/socket.h; libc currently exposes the option, but not its cmsg type.
const SCM_TIMESTAMP_MONOTONIC: libc::c_int = 0x04;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Timestamps {
    pub wall_ns: Option<u64>,
    // XNU ip_savecontrol/ip6_savecontrol_v4 copy mach_absolute_time() directly:
    // this is a raw Mach tick count, not nanoseconds or a wall-clock timestamp.
    pub monotonic_ticks: Option<u64>,
}

pub(super) fn enable_monotonic(socket: &UdpSocket) -> io::Result<()> {
    let value: libc::c_int = 1;
    let mut observed: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&value) as libc::socklen_t;
    // The socket owns its descriptor and all buffers outlive these syscalls.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_TIMESTAMP_MONOTONIC,
            (&value as *const libc::c_int).cast(),
            length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_TIMESTAMP_MONOTONIC,
            (&mut observed as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    // Darwin's boolean socket-option getter returns the option's bitmask
    // (0x0800 here), rather than normalizing enabled to the setter's integer 1.
    if length != std::mem::size_of_val(&value) as libc::socklen_t || observed == 0 {
        return Err(io::Error::other(
            "Kernel arrival timestamp option was not applied",
        ));
    }
    Ok(())
}

pub(super) fn enable(socket: &UdpSocket) -> io::Result<()> {
    let value: libc::c_int = 1;
    // The descriptor and integer remain valid for this synchronous syscall.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_TIMESTAMP,
            (&value as *const libc::c_int).cast(),
            std::mem::size_of_val(&value) as libc::socklen_t,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn receive(
    socket: &UdpSocket,
    bytes: &mut [u8],
) -> io::Result<(usize, SocketAddr, Timestamps)> {
    #[repr(align(8))]
    struct Control([u8; 128]);
    let mut control = Control([0; 128]);
    let mut vector = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    // SockAddr initializes address storage, and recvmsg fills the address and
    // its actual length. All data/control/address buffers outlive this call.
    let ((size, timestamp), address) = unsafe {
        socket2::SockAddr::try_init(|storage, length| {
            let mut message: libc::msghdr = std::mem::zeroed();
            message.msg_name = storage.cast();
            message.msg_namelen = *length;
            message.msg_iov = &mut vector;
            message.msg_iovlen = 1;
            message.msg_control = control.0.as_mut_ptr().cast();
            message.msg_controllen = control.0.len() as _;
            let count = libc::recvmsg(socket.as_raw_fd(), &mut message, 0);
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if message.msg_namelen > *length || count as usize > bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid recvmsg buffer length",
                ));
            }
            *length = message.msg_namelen;
            let mut timestamps = Timestamps::default();
            if message.msg_flags & libc::MSG_CTRUNC == 0 {
                let mut header = libc::CMSG_FIRSTHDR(&message);
                while !header.is_null() {
                    if (*header).cmsg_level == libc::SOL_SOCKET
                        && (*header).cmsg_type == libc::SCM_TIMESTAMP
                        && (*header).cmsg_len
                            >= libc::CMSG_LEN(std::mem::size_of::<libc::timeval>() as _)
                    {
                        let value = std::ptr::read_unaligned(
                            libc::CMSG_DATA(header).cast::<libc::timeval>(),
                        );
                        if value.tv_sec >= 0 && (0..1_000_000).contains(&value.tv_usec) {
                            timestamps.wall_ns = (value.tv_sec as u64)
                                .checked_mul(1_000_000_000)
                                .and_then(|v| v.checked_add(value.tv_usec as u64 * 1_000));
                        }
                    } else if (*header).cmsg_level == libc::SOL_SOCKET
                        && (*header).cmsg_type == SCM_TIMESTAMP_MONOTONIC
                        && (*header).cmsg_len >= libc::CMSG_LEN(std::mem::size_of::<u64>() as _)
                    {
                        timestamps.monotonic_ticks = Some(std::ptr::read_unaligned(
                            libc::CMSG_DATA(header).cast::<u64>(),
                        ));
                    }
                    header = libc::CMSG_NXTHDR(&message, header);
                }
            }
            Ok((count as usize, timestamps))
        })
    }?;
    let address = address
        .as_socket()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Non-IP UDP source"))?;
    Ok((size, address, timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn wall() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }
    #[test]
    fn timestamps_preserve_ipv4_ipv6_packet_source_and_nonblocking_behavior() {
        for bind in ["127.0.0.1:0", "[::1]:0"] {
            let receiver = UdpSocket::bind(bind).unwrap();
            let sender = UdpSocket::bind(bind).unwrap();
            enable(&receiver).unwrap();
            receiver.set_nonblocking(true).unwrap();
            let mut bytes = [0; 128];
            assert_eq!(
                receive(&receiver, &mut bytes).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            let before = wall();
            sender
                .send_to(b"owned timestamp fixture", receiver.local_addr().unwrap())
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            let (size, source, stamp) = loop {
                match receive(&receiver, &mut bytes) {
                    Ok(value) => break value,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::yield_now()
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            assert_eq!(&bytes[..size], b"owned timestamp fixture");
            assert_eq!(source, sender.local_addr().unwrap());
            assert!((before..=wall()).contains(&stamp.wall_ns.unwrap()));
            assert!(stamp.monotonic_ticks.is_none());
        }
    }

    #[test]
    fn queued_ipv4_ipv6_packets_keep_both_wall_and_mach_arrival_times() {
        for bind in ["127.0.0.1:0", "[::1]:0"] {
            let receiver = UdpSocket::bind(bind).unwrap();
            receiver
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            let sender = UdpSocket::bind(bind).unwrap();
            enable(&receiver).unwrap();
            let clock = super::super::kernel_arrival::Receiver::new(Some(&receiver), false);
            assert!(clock.active());
            let before_wall = wall();
            sender
                .send_to(b"first queued packet", receiver.local_addr().unwrap())
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(8));
            sender
                .send_to(b"second queued packet", receiver.local_addr().unwrap())
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(25));
            let mut results = Vec::new();
            for expected in [
                b"first queued packet".as_slice(),
                b"second queued packet".as_slice(),
            ] {
                let mut bytes = [0; 128];
                let (size, source, stamps) = receive(&receiver, &mut bytes).unwrap();
                let (arrival, applied) = clock.receive_time(stamps);
                assert_eq!(&bytes[..size], expected);
                assert_eq!(source, sender.local_addr().unwrap());
                assert!((before_wall..=wall()).contains(&stamps.wall_ns.unwrap()));
                assert!(applied);
                // Receiver processing is deliberately delayed. Using
                // Instant::now() here would erase the real packet spacing.
                assert!(arrival.elapsed() >= std::time::Duration::from_millis(20));
                results.push((stamps.monotonic_ticks.unwrap(), arrival));
            }
            assert!(results[1].0 > results[0].0);
            assert!(
                results[1].1.duration_since(results[0].1) >= std::time::Duration::from_millis(5)
            );
        }
    }
}
