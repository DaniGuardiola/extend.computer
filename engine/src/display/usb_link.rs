//! Multiplex encrypted media sessions over a single native accessory byte pipe.
//! Device permission, pinned signaling and physical USB I/O remain outside this
//! router. Each RTC endpoint retains distinct logical source/destination ports.
use super::{
    packet_link::{self, FramedPacketLink, PacketDriver},
    queue_metrics::{self, Stamp, Trace},
    DatagramLink,
};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const CARRIER_PORT: u16 = 40000;
const MAX_ENDPOINTS: usize = 16;
const MAX_QUEUED_PACKETS: usize = 512;
const MAX_MEDIA_PACKET: usize = 2044;
struct Ingress {
    queue: VecDeque<(Vec<u8>, u16, Option<Stamp>)>,
    trace: Option<Trace>,
    failed: bool,
}
pub struct UsbHub {
    carrier: FramedPacketLink,
    local: IpAddr,
    peer: IpAddr,
    endpoints: Mutex<HashMap<u16, Ingress>>,
    next_port: Mutex<u16>,
    pub(super) failed: AtomicBool,
    pump_lock: Mutex<()>,
    pub(super) streams: Mutex<super::usb_stream::StreamState>,
    pub(super) stream_worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    sent: AtomicU64,
    received: AtomicU64,
    unregistered: AtomicU64,
}
pub struct UsbEndpoint {
    hub: Arc<UsbHub>,
    port: u16,
}
impl UsbHub {
    pub fn new(local: IpAddr, peer: IpAddr) -> io::Result<(Arc<Self>, PacketDriver)> {
        if local == peer {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let (carrier, driver) = packet_link::channel(
            SocketAddr::new(local, CARRIER_PORT),
            SocketAddr::new(peer, CARRIER_PORT),
        )?;
        Ok((
            Arc::new(Self {
                carrier,
                local,
                peer,
                endpoints: Mutex::new(HashMap::new()),
                next_port: Mutex::new(CARRIER_PORT + 1),
                failed: AtomicBool::new(false),
                pump_lock: Mutex::new(()),
                streams: Mutex::new(super::usb_stream::StreamState::new(local < peer)),
                stream_worker: Mutex::new(None),
                sent: AtomicU64::new(0),
                received: AtomicU64::new(0),
                unregistered: AtomicU64::new(0),
            }),
            driver,
        ))
    }
    pub fn endpoint(self: &Arc<Self>) -> io::Result<UsbEndpoint> {
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let mut endpoints = self
            .endpoints
            .lock()
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        if endpoints.len() >= MAX_ENDPOINTS {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let mut next = self
            .next_port
            .lock()
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        let port = *next;
        *next = next.checked_add(1).ok_or(io::ErrorKind::AddrNotAvailable)?;
        endpoints.insert(
            port,
            Ingress {
                queue: VecDeque::new(),
                trace: Trace::new("endpoint_incoming"),
                failed: false,
            },
        );
        Ok(UsbEndpoint {
            hub: self.clone(),
            port,
        })
    }
    pub fn counters(&self) -> (u64, u64, u64) {
        (
            self.sent.load(Ordering::Relaxed),
            self.received.load(Ordering::Relaxed),
            self.unregistered.load(Ordering::Relaxed),
        )
    }
    pub fn queue_metrics(&self) -> serde_json::Value {
        if !queue_metrics::enabled() {
            return serde_json::Value::Null;
        }
        let endpoints = self.endpoints.lock().ok().map(|mut endpoints| {
            endpoints
                .values_mut()
                .filter_map(|ingress| {
                    ingress.trace.as_mut().map(|trace| {
                        trace.snapshot(ingress.queue.len(), ingress.queue.front().and_then(|p| p.2))
                    })
                })
                .collect::<Vec<_>>()
        });
        serde_json::json!({"carrier":self.carrier.queue_metrics(),"endpoint_incoming":endpoints,"clock":"process_monotonic_local_only"})
    }
    pub(super) fn send_stream_packet(&self, payload: &[u8]) -> io::Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let mut packet = Vec::with_capacity(payload.len() + 4);
        packet.extend_from_slice(&[0, 0, 0, 0]);
        packet.extend_from_slice(payload);
        self.carrier
            .send_to(&packet, SocketAddr::new(self.peer, CARRIER_PORT))?;
        Ok(())
    }
    pub(super) fn fail(&self) {
        self.failed.store(true, Ordering::Release);
        self.carrier.shutdown();
        if let Ok(mut streams) = self.streams.lock() {
            streams.shutdown();
        }
    }
    /// Shuts every private signaling socket and media endpoint, then joins the
    /// single nonblocking signaling worker. Never called under the router lock.
    pub fn close(&self) {
        self.fail();
        let worker = self
            .stream_worker
            .lock()
            .ok()
            .and_then(|mut worker| worker.take());
        if let Some(worker) = worker {
            if worker.thread().id() != std::thread::current().id() {
                let _ = worker.join();
            }
        }
    }
    pub(super) fn pump(&self) -> io::Result<()> {
        let result = self.pump_inner();
        if result.is_err() {
            self.fail();
        }
        result
    }
    fn pump_inner(&self) -> io::Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let _pump = self
            .pump_lock
            .lock()
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        let mut bytes = [0; MAX_MEDIA_PACKET + 4];
        for _ in 0..64 {
            let (size, _) = match self.carrier.recv_from(&mut bytes) {
                Ok(value) => value,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            };
            if size <= 4 {
                self.failed.store(true, Ordering::Release);
                return Err(io::ErrorKind::InvalidData.into());
            }
            let source = u16::from_be_bytes([bytes[0], bytes[1]]);
            let destination = u16::from_be_bytes([bytes[2], bytes[3]]);
            if source == 0 && destination == 0 {
                self.streams
                    .lock()
                    .map_err(|_| io::ErrorKind::BrokenPipe)?
                    .receive(&bytes[4..size])?;
                continue;
            }
            if source <= CARRIER_PORT || destination <= CARRIER_PORT {
                self.failed.store(true, Ordering::Release);
                return Err(io::ErrorKind::InvalidData.into());
            }
            self.received.fetch_add(1, Ordering::Relaxed);
            let mut endpoints = self
                .endpoints
                .lock()
                .map_err(|_| io::ErrorKind::BrokenPipe)?;
            let Some(ingress) = endpoints.get_mut(&destination) else {
                self.unregistered.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            if ingress.failed {
                continue;
            }
            if ingress.queue.len() == MAX_QUEUED_PACKETS {
                // A stalled consumer fails its own session instead of silently
                // dropping encrypted handshake/reference packets or growing RAM.
                ingress.failed = true;
                ingress.queue.clear();
                continue;
            }
            ingress.queue.push_back((
                bytes[4..size].to_vec(),
                source,
                queue_metrics::stamp(&bytes[4..size], 0),
            ));
            if let Some(trace) = &mut ingress.trace {
                trace.depth(ingress.queue.len());
            }
        }
        Ok(())
    }
}
impl DatagramLink for UsbEndpoint {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::new(self.hub.local, self.port))
    }
    fn send_to(&self, bytes: &[u8], destination: SocketAddr) -> io::Result<usize> {
        if self.hub.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if destination.ip() != self.hub.peer || destination.port() <= CARRIER_PORT {
            return Err(io::ErrorKind::AddrNotAvailable.into());
        }
        if bytes.is_empty() || bytes.len() > MAX_MEDIA_PACKET {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut packet = Vec::with_capacity(bytes.len() + 4);
        packet.extend_from_slice(&self.port.to_be_bytes());
        packet.extend_from_slice(&destination.port().to_be_bytes());
        packet.extend_from_slice(bytes);
        self.hub
            .carrier
            .send_to(&packet, SocketAddr::new(self.hub.peer, CARRIER_PORT))?;
        self.hub.sent.fetch_add(1, Ordering::Relaxed);
        Ok(bytes.len())
    }
    fn recv_from(&self, bytes: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.hub.pump()?;
        let mut endpoints = self
            .hub
            .endpoints
            .lock()
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        let ingress = endpoints
            .get_mut(&self.port)
            .ok_or(io::ErrorKind::BrokenPipe)?;
        if ingress.failed {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let size = ingress
            .queue
            .front()
            .ok_or(io::ErrorKind::WouldBlock)?
            .0
            .len();
        if size > bytes.len() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let (packet, source, stamp) = ingress
            .queue
            .pop_front()
            .expect("Front packet checked under lock");
        if let Some(trace) = &mut ingress.trace {
            trace.removed(stamp);
        }
        bytes[..size].copy_from_slice(&packet);
        Ok((size, SocketAddr::new(self.hub.peer, source)))
    }
}
impl Drop for UsbEndpoint {
    fn drop(&mut self) {
        if let Ok(mut endpoints) = self.hub.endpoints.lock() {
            endpoints.remove(&self.port);
        }
    }
}

/// Physical worker adapter. Both handles must be nonblocking. Partial reads and
/// writes retain their original bytes; buffers and per-tick batches are bounded.
/// Returning drops the driver and disconnects all RTC endpoints.
pub fn run_io(
    mut driver: PacketDriver,
    mut reader: impl Read,
    mut writer: impl Write,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut read_buffer = [0; 16384];
    let (mut read_start, mut read_end) = (0, 0);
    let mut pending = Vec::new();
    let mut written = 0;
    while !stop.load(Ordering::Acquire) {
        let mut progress = false;
        for _ in 0..64 {
            if pending.is_empty() {
                let Some(frame) = driver.take_outgoing()? else {
                    break;
                };
                driver.pending_started(&frame);
                pending = frame;
                written = 0;
            }
            match writer.write(&pending[written..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => {
                    written += count;
                    progress = true;
                    if written == pending.len() {
                        driver.pending_completed();
                        pending.clear();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        for _ in 0..64 {
            if read_start == read_end {
                match reader.read(&mut read_buffer) {
                    Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                    Ok(count) => {
                        read_start = 0;
                        read_end = count;
                        progress = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
            let count = driver.receive_bytes(&read_buffer[read_start..read_end])?;
            if count == 0 {
                break;
            }
            read_start += count;
            progress = true;
        }
        if !progress {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{
        display::{DisplayEvent, Frame, Transport},
        session::display::VideoCodec,
    };
    use std::{os::unix::net::UnixStream, time::Instant};
    #[test]
    fn independent_bidirectional_encrypted_sessions_share_native_byte_pipe() {
        let (host, host_driver) =
            UsbHub::new("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()).unwrap();
        let (tablet, tablet_driver) =
            UsbHub::new("192.0.2.2".parse().unwrap(), "192.0.2.1".parse().unwrap()).unwrap();
        let (host_io, tablet_io) = UnixStream::pair().unwrap();
        host_io.set_nonblocking(true).unwrap();
        tablet_io.set_nonblocking(true).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop_host = stopped.clone();
        let host_write = host_io.try_clone().unwrap();
        let host_worker =
            std::thread::spawn(move || run_io(host_driver, host_io, host_write, &stop_host));
        let stop_tablet = stopped.clone();
        let tablet_write = tablet_io.try_clone().unwrap();
        let tablet_worker = std::thread::spawn(move || {
            run_io(tablet_driver, tablet_io, tablet_write, &stop_tablet)
        });
        // Signaling starts before any RTC endpoint exists, then stays attached
        // while two independent encrypted media sessions use the same carrier.
        host.enable_streams().unwrap();
        tablet.enable_streams().unwrap();
        let mut signaling = host
            .open_stream(
                super::super::usb_stream::UsbService::Presence,
                &stopped,
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap();
        let mut accepted = loop {
            if let Some((service, stream)) = tablet.accept_stream().unwrap() {
                assert_eq!(service, super::super::usb_stream::UsbService::Presence);
                break stream;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        accepted
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        signaling.write_all(b"signaling-before-media").unwrap();
        let mut observed = [0; 22];
        accepted.read_exact(&mut observed).unwrap();
        assert_eq!(&observed, b"signaling-before-media");
        let mut host_source =
            Transport::with_packet_link(VideoCodec::H264, Box::new(host.endpoint().unwrap()))
                .unwrap();
        let mut host_receiver =
            Transport::with_packet_link(VideoCodec::Hevc, Box::new(host.endpoint().unwrap()))
                .unwrap();
        // Opposite allocation order ensures routing uses negotiated ports,
        // rather than assuming the two hosts have matching session counters.
        let mut tablet_source =
            Transport::with_packet_link(VideoCodec::Hevc, Box::new(tablet.endpoint().unwrap()))
                .unwrap();
        let mut tablet_receiver =
            Transport::with_packet_link(VideoCodec::H264, Box::new(tablet.endpoint().unwrap()))
                .unwrap();
        let offer = host_source.offer().unwrap();
        let answer = tablet_receiver.answer(&offer).unwrap();
        host_source.accept_answer(&answer).unwrap();
        let offer = tablet_source.offer().unwrap();
        let answer = host_receiver.answer(&offer).unwrap();
        tablet_source.accept_answer(&answer).unwrap();
        let mut avc = vec![0, 0, 0, 1, 0x65];
        avc.extend(vec![0x88; 400_000]);
        let mut hevc = vec![0, 0, 0, 1, 38, 1];
        hevc.extend(vec![0x99; 400_000]);
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut host_sent, mut tablet_sent, mut host_received, mut tablet_received) =
            (false, false, false, false);
        while !host_received || !tablet_received {
            if matches!(host_source.poll().unwrap(), Some(DisplayEvent::Connected)) && !host_sent {
                assert!(host_source
                    .write(Frame {
                        timestamp: 1_000_000_000,
                        keyframe: true,
                        bytes: avc.clone()
                    })
                    .unwrap());
                host_sent = true;
            }
            if matches!(tablet_source.poll().unwrap(), Some(DisplayEvent::Connected))
                && !tablet_sent
            {
                assert!(tablet_source
                    .write(Frame {
                        timestamp: 2_000_000_000,
                        keyframe: true,
                        bytes: hevc.clone()
                    })
                    .unwrap());
                tablet_sent = true;
            }
            if let Some(DisplayEvent::Frame(frame)) = host_receiver.poll().unwrap() {
                assert_eq!(frame.bytes, hevc);
                assert!(frame.keyframe);
                host_received = true;
            }
            if let Some(DisplayEvent::Frame(frame)) = tablet_receiver.poll().unwrap() {
                assert_eq!(frame.bytes, avc);
                assert!(frame.keyframe);
                tablet_received = true;
            }
            assert!(
                Instant::now() < deadline,
                "Multiplexed encrypted sessions timed out"
            );
            // Each poll moves about one packet; a millisecond sleep per pass
            // would stretch two 400 KB keyframes past the reorder deadline.
            std::thread::yield_now();
        }
        assert!(host.counters().0 > 100 && host.counters().1 > 100);
        assert_eq!(host.counters().2, 0);
        assert_eq!(tablet.counters().2, 0);
        stopped.store(true, Ordering::Release);
        for worker in [host_worker, tablet_worker] {
            let result = worker.join().unwrap();
            assert!(result.is_ok() || result.unwrap_err().kind() == io::ErrorKind::UnexpectedEof);
        }
        assert_eq!(
            host_source.poll().err().unwrap().root_cause().to_string(),
            io::Error::from(io::ErrorKind::BrokenPipe).to_string()
        );
    }
}
