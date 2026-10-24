//! Bounded datagram framing for native byte-stream carriers such as USB bulk.
//! This carries already-encrypted RTC packets; signaling and device discovery
//! are separate responsibilities. No USB I/O runs on the RTC polling thread.
use super::{
    queue_metrics::{self, Stamp, Trace},
    DatagramLink,
};
use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

const MAX_PACKET: usize = 2048;
const MAX_PACKETS: usize = 1024;
const MAX_QUEUE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct Queue {
    packets: VecDeque<(Vec<u8>, Option<Stamp>)>,
    trace: Option<Trace>,
    bytes: usize,
}
impl Queue {
    fn snapshot(&mut self) -> serde_json::Value {
        match &mut self.trace {
            Some(trace) => {
                trace.snapshot(self.packets.len(), self.packets.front().and_then(|p| p.1))
            }
            None => serde_json::Value::Null,
        }
    }
    fn full(&self, size: usize) -> bool {
        self.packets.len() >= MAX_PACKETS || self.bytes + size > MAX_QUEUE_BYTES
    }
    fn push(&mut self, packet: Vec<u8>) {
        self.bytes += packet.len();
        let stamp = queue_metrics::stamp(&packet, 4);
        self.packets.push_back((packet, stamp));
        if let Some(trace) = &mut self.trace {
            trace.depth(self.packets.len());
        }
    }
    fn pop(&mut self) -> Option<Vec<u8>> {
        let (packet, stamp) = self.packets.pop_front()?;
        if let Some(trace) = &mut self.trace {
            trace.removed(stamp);
        }
        self.bytes -= packet.len();
        Some(packet)
    }
}
#[derive(Default)]
struct State {
    incoming: Queue,
    outgoing: Queue,
    closed: bool,
    pending_trace: Option<Trace>,
    pending_stamp: Option<Stamp>,
    pending_observed: bool,
}
type Shared = Arc<Mutex<State>>;
fn state(shared: &Shared) -> io::Result<std::sync::MutexGuard<'_, State>> {
    let guard = shared.lock().map_err(|_| io::ErrorKind::BrokenPipe)?;
    if guard.closed {
        return Err(io::ErrorKind::BrokenPipe.into());
    }
    Ok(guard)
}
fn close(shared: &Shared) {
    if let Ok(mut guard) = shared.lock() {
        *guard = State {
            closed: true,
            ..State::default()
        };
    }
}

pub struct FramedPacketLink {
    shared: Shared,
    local: SocketAddr,
    peer: SocketAddr,
}
pub struct PacketDriver {
    shared: Shared,
    header: [u8; 4],
    header_used: usize,
    expected: Option<usize>,
    packet: Vec<u8>,
}

/// One endpoint and one worker-owned driver. Queue limits apply separately in
/// each direction; dropping either half closes the link and releases its queues.
pub fn channel(
    local: SocketAddr,
    peer: SocketAddr,
) -> io::Result<(FramedPacketLink, PacketDriver)> {
    if local.ip().is_unspecified()
        || peer.ip().is_unspecified()
        || local.port() == 0
        || peer.port() == 0
        || local == peer
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut initial = State::default();
    initial.incoming.trace = Trace::new("carrier_incoming");
    initial.outgoing.trace = Trace::new("carrier_outgoing");
    initial.pending_trace = Trace::new("worker_pending_write");
    let shared = Arc::new(Mutex::new(initial));
    Ok((
        FramedPacketLink {
            shared: shared.clone(),
            local,
            peer,
        },
        PacketDriver {
            shared,
            header: [0; 4],
            header_used: 0,
            expected: None,
            packet: Vec::new(),
        },
    ))
}
impl FramedPacketLink {
    /// Fatal router/epoch shutdown also wakes the physical worker-owned driver.
    pub(super) fn shutdown(&self) {
        close(&self.shared);
    }
    pub(super) fn queue_metrics(&self) -> serde_json::Value {
        match state(&self.shared) {
            Ok(mut s) => {
                let pending_stamp = s.pending_stamp;
                let pending = s.pending_trace.as_mut().map(|trace| {
                    trace.snapshot(usize::from(pending_stamp.is_some()), pending_stamp)
                });
                serde_json::json!({"incoming":s.incoming.snapshot(),"outgoing":s.outgoing.snapshot(),"worker_pending_write":if s.pending_observed { pending } else { None },"worker_pending_write_status":if s.pending_observed {"observed"} else {"unavailable_no_worker_write_ack"}})
            }
            Err(_) => serde_json::Value::Null,
        }
    }
}
impl DatagramLink for FramedPacketLink {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local)
    }
    fn send_to(&self, bytes: &[u8], destination: SocketAddr) -> io::Result<usize> {
        if destination != self.peer {
            return Err(io::ErrorKind::AddrNotAvailable.into());
        }
        if bytes.is_empty() || bytes.len() > MAX_PACKET {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut guard = state(&self.shared)?;
        if guard.outgoing.full(bytes.len()) {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        guard.outgoing.push(bytes.to_vec());
        Ok(bytes.len())
    }
    fn recv_from(&self, bytes: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let mut guard = state(&self.shared)?;
        let size = guard
            .incoming
            .packets
            .front()
            .ok_or(io::ErrorKind::WouldBlock)?
            .0
            .len();
        if size > bytes.len() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let packet = guard
            .incoming
            .pop()
            .expect("Front packet checked under lock");
        bytes[..size].copy_from_slice(&packet);
        Ok((size, self.peer))
    }
}
impl PacketDriver {
    pub(super) fn pending_started(&mut self, frame: &[u8]) {
        if !queue_metrics::enabled() {
            return;
        }
        if let Ok(mut state) = state(&self.shared) {
            state.pending_observed = true;
            state.pending_stamp = queue_metrics::stamp(frame.get(4..).unwrap_or_default(), 4);
        }
    }
    pub(super) fn pending_completed(&mut self) {
        if !queue_metrics::enabled() {
            return;
        }
        if let Ok(mut state) = state(&self.shared) {
            let stamp = state.pending_stamp.take();
            if let Some(trace) = &mut state.pending_trace {
                trace.depth(1);
                trace.removed(stamp);
            }
        }
    }
    /// Worker must retain this entire frame across partial USB writes. Taking
    /// another frame before writing this one completely would corrupt framing.
    pub fn take_outgoing(&mut self) -> io::Result<Option<Vec<u8>>> {
        let Some(packet) = state(&self.shared)?.outgoing.pop() else {
            return Ok(None);
        };
        let mut frame = Vec::with_capacity(packet.len() + 4);
        frame.extend_from_slice(&(packet.len() as u32).to_be_bytes());
        frame.extend_from_slice(&packet);
        Ok(Some(frame))
    }
    /// Accept arbitrarily fragmented/coalesced USB reads. Returns consumed byte
    /// count; worker retains the remainder during backpressure and calls again
    /// when receive space is available. An empty slice can flush a complete held
    /// packet. Invalid lengths close the link before allocating payload storage.
    pub fn receive_bytes(&mut self, bytes: &[u8]) -> io::Result<usize> {
        drop(state(&self.shared)?);
        let mut consumed = 0;
        loop {
            if self.expected.is_some_and(|size| self.packet.len() == size) {
                let mut guard = state(&self.shared)?;
                if guard.incoming.full(self.packet.len()) {
                    return Ok(consumed);
                }
                guard.incoming.push(std::mem::take(&mut self.packet));
                self.expected = None;
                self.header_used = 0;
            }
            if consumed == bytes.len() {
                return Ok(consumed);
            }
            if self.expected.is_none() {
                let count = (4 - self.header_used).min(bytes.len() - consumed);
                self.header[self.header_used..self.header_used + count]
                    .copy_from_slice(&bytes[consumed..consumed + count]);
                self.header_used += count;
                consumed += count;
                if self.header_used < 4 {
                    continue;
                }
                let size = u32::from_be_bytes(self.header) as usize;
                if size == 0 || size > MAX_PACKET {
                    close(&self.shared);
                    return Err(io::ErrorKind::InvalidData.into());
                }
                self.expected = Some(size);
                self.packet = Vec::with_capacity(size);
            }
            let count = (self.expected.unwrap() - self.packet.len()).min(bytes.len() - consumed);
            self.packet
                .extend_from_slice(&bytes[consumed..consumed + count]);
            consumed += count;
        }
    }
}
impl Drop for PacketDriver {
    fn drop(&mut self) {
        close(&self.shared);
    }
}
impl Drop for FramedPacketLink {
    fn drop(&mut self) {
        close(&self.shared);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (FramedPacketLink, PacketDriver) {
        channel(
            "192.0.2.1:48180".parse().unwrap(),
            "192.0.2.2:48180".parse().unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn instrumented_queue_retains_fifo_bytes_and_original_capacity_and_close_behavior() {
        let (link, mut driver) = pair();
        {
            let mut state = state(&link.shared).unwrap();
            state.outgoing.trace = Some(Trace::testing("outgoing_test"));
        }
        for n in 0..MAX_PACKETS {
            link.send_to(&(n as u16).to_be_bytes(), link.peer).unwrap();
        }
        assert_eq!(
            link.send_to(&[1], link.peer).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            link.queue_metrics()["outgoing"]["current"]["highwater"],
            MAX_PACKETS
        );
        for n in 0..MAX_PACKETS {
            assert_eq!(
                driver.take_outgoing().unwrap().unwrap(),
                [vec![0, 0, 0, 2], (n as u16).to_be_bytes().to_vec()].concat()
            );
        }
        let state = state(&link.shared).unwrap();
        assert_eq!(state.outgoing.bytes, 0);
        drop(state);
        assert!(driver.take_outgoing().unwrap().is_none());
        drop(driver);
        assert_eq!(
            link.send_to(&[1], link.peer).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
    #[test]
    fn encrypted_rtc_frames_cross_fragmented_bulk_streams() {
        use super::super::{DisplayEvent, Frame, Transport};
        use crate::session::display::VideoCodec;
        use std::time::{Duration, Instant};
        fn transfer(source: &mut PacketDriver, destination: &mut PacketDriver) {
            for _ in 0..64 {
                let Some(frame) = source.take_outgoing().unwrap() else {
                    break;
                };
                assert!(!frame.windows(64).any(|window| window == [0x88; 64]));
                for fragment in frame.chunks(7) {
                    assert_eq!(destination.receive_bytes(fragment).unwrap(), fragment.len());
                }
            }
        }
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let a = "192.0.2.1:48180".parse().unwrap();
            let b = "192.0.2.2:48180".parse().unwrap();
            let (a_link, mut a_driver) = channel(a, b).unwrap();
            let (b_link, mut b_driver) = channel(b, a).unwrap();
            let mut sender = Transport::with_packet_link(codec, Box::new(a_link)).unwrap();
            let mut receiver = Transport::with_packet_link(codec, Box::new(b_link)).unwrap();
            let offer = sender.offer().unwrap();
            let answer = receiver.answer(&offer).unwrap();
            sender.accept_answer(&answer).unwrap();
            let mut expected = match codec {
                VideoCodec::Hevc => vec![0, 0, 0, 1, 38, 1],
                _ => vec![0, 0, 0, 1, 0x65],
            };
            expected.extend(vec![0x88; 400_000]);
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut sent = false;
            loop {
                transfer(&mut a_driver, &mut b_driver);
                transfer(&mut b_driver, &mut a_driver);
                if matches!(sender.poll().unwrap(), Some(DisplayEvent::Connected)) && !sent {
                    assert!(sender
                        .write(Frame {
                            timestamp: 1_000_000_000,
                            keyframe: true,
                            bytes: expected.clone(),
                        })
                        .unwrap());
                    sent = true;
                }
                if let Some(DisplayEvent::Frame(frame)) = receiver.poll().unwrap() {
                    assert_eq!(frame.bytes, expected);
                    assert!(frame.keyframe);
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "Framed encrypted transport timed out"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    #[test]
    fn bulk_fragments_preserve_multiple_packet_boundaries() {
        let (link, mut driver) = pair();
        let mut stream = Vec::new();
        for size in [1, 17, MAX_PACKET] {
            link.send_to(&vec![size as u8; size], link.peer).unwrap();
            stream.extend(driver.take_outgoing().unwrap().unwrap());
        }
        for fragment in stream.chunks(3) {
            assert_eq!(driver.receive_bytes(fragment).unwrap(), fragment.len());
        }
        for size in [1, 17, MAX_PACKET] {
            let mut bytes = [0; MAX_PACKET];
            let (count, source) = link.recv_from(&mut bytes).unwrap();
            assert_eq!(count, size);
            assert_eq!(source, link.peer);
            assert_eq!(&bytes[..size], vec![size as u8; size]);
        }
        assert_eq!(
            link.recv_from(&mut [0; MAX_PACKET]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn full_queues_preserve_held_packet_and_unconsumed_bulk_bytes() {
        let (link, mut driver) = pair();
        for _ in 0..MAX_PACKETS {
            link.send_to(&[7], link.peer).unwrap();
        }
        assert_eq!(
            link.send_to(&[8], link.peer).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        for _ in 0..MAX_PACKETS {
            let frame = driver.take_outgoing().unwrap().unwrap();
            assert_eq!(driver.receive_bytes(&frame).unwrap(), frame.len());
        }
        let frames = [0, 0, 0, 1, 8, 0, 0, 0, 1, 9];
        assert_eq!(driver.receive_bytes(&frames).unwrap(), 5);
        let mut byte = [0];
        link.recv_from(&mut byte).unwrap();
        assert_eq!(driver.receive_bytes(&frames[5..]).unwrap(), 5);
        link.recv_from(&mut byte).unwrap();
        assert_eq!(driver.receive_bytes(&[]).unwrap(), 0);
        for _ in 0..MAX_PACKETS - 2 {
            link.recv_from(&mut byte).unwrap();
            assert_eq!(byte[0], 7);
        }
        link.recv_from(&mut byte).unwrap();
        assert_eq!(byte[0], 8);
        link.recv_from(&mut byte).unwrap();
        assert_eq!(byte[0], 9);
    }
    #[test]
    fn invalid_length_and_worker_disconnect_close_native_endpoint() {
        for length in [0_u32, MAX_PACKET as u32 + 1, u32::MAX] {
            let (link, mut driver) = pair();
            assert_eq!(
                driver
                    .receive_bytes(&length.to_be_bytes())
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(
                link.recv_from(&mut [0; MAX_PACKET]).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
        }
        let (link, driver) = pair();
        drop(driver);
        assert_eq!(
            link.send_to(&[1], link.peer).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
