//! Fixed-service reliable signaling over the native USB packet carrier.
//! This module supplies private TCP pairs for existing protocol APIs. It never
//! authenticates a peer or forwards to a supplied network address.
use super::usb_link::UsbHub;
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const VERSION: u8 = 1;
const HEADER: usize = 21;
const CHUNK: usize = 2023; // Header + data <= the carrier's 2044-byte payload.
const MAX_STREAMS: usize = 4;
const MAX_INCOMING: usize = 64 * 1024;
const MAX_CONTROL: usize = 32;
const MAX_CLOSED: usize = 64;
const STALL: Duration = Duration::from_secs(5);
const HELLO: u8 = 1;
const OPEN: u8 = 2;
const ACK: u8 = 3;
const DATA: u8 = 4;
const CLOSE: u8 = 5;
const REJECT: u8 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbService {
    Presence = 1,
    Control = 2,
    Display = 3,
}
impl UsbService {
    fn parse(value: u8) -> io::Result<Self> {
        match value {
            1 => Ok(Self::Presence),
            2 => Ok(Self::Control),
            3 => Ok(Self::Display),
            _ => invalid(),
        }
    }
}
fn invalid<T>() -> io::Result<T> {
    Err(io::ErrorKind::InvalidData.into())
}
fn private_pair(request_deadline: Instant) -> io::Result<(TcpStream, TcpStream)> {
    // One overall startup budget, including load-dependent loopback delivery.
    // The temporary listener never accepts an operator/peer-supplied address.
    let deadline = request_deadline.min(Instant::now() + Duration::from_secs(1));
    if Instant::now() >= deadline {
        return Err(io::ErrorKind::TimedOut.into());
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let application = TcpStream::connect_timeout(
        &listener.local_addr()?,
        deadline.saturating_duration_since(Instant::now()),
    )?;
    let (bridge, observed) = loop {
        match listener.accept() {
            Ok(accepted) => break accepted,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    };
    // A racing local process can cause rejection, never substitute its socket.
    if observed != application.local_addr()? {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    application.set_nodelay(true)?;
    bridge.set_nodelay(true)?;
    bridge.set_nonblocking(true)?;
    Ok((application, bridge))
}

struct Stream {
    service: UsbService,
    local: bool,
    acknowledged: bool,
    rejected: bool,
    application: Option<TcpStream>,
    bridge: TcpStream,
    incoming: VecDeque<Vec<u8>>,
    incoming_bytes: usize,
    offset: usize,
    outgoing: Option<Vec<u8>>,
    opened: Instant,
    incoming_stalled: Option<Instant>,
    outgoing_stalled: Option<Instant>,
    remote_closed: bool,
}
impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.bridge.shutdown(Shutdown::Both);
    }
}
pub(super) struct StreamState {
    enabled: bool,
    hello_sent: bool,
    remote_epoch: Option<u64>,
    epoch: u64,
    local_odd: bool,
    next: u32,
    streams: HashMap<u32, Stream>,
    control: VecDeque<Vec<u8>>,
    closed: VecDeque<(u32, UsbService)>,
    last_remote: u32,
    control_stalled: Option<Instant>,
}
impl StreamState {
    pub(super) fn new(local_odd: bool) -> Self {
        let mut epoch = rand::random::<u64>();
        while epoch == 0 {
            epoch = rand::random();
        }
        Self {
            enabled: false,
            hello_sent: false,
            remote_epoch: None,
            epoch,
            local_odd,
            next: if local_odd { 1 } else { 2 },
            streams: HashMap::new(),
            control: VecDeque::new(),
            closed: VecDeque::new(),
            last_remote: 0,
            control_stalled: None,
        }
    }
    fn frame(&self, kind: u8, id: u32, service: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER + payload.len());
        bytes.extend_from_slice(b"EUSB");
        bytes.extend_from_slice(&[VERSION, kind]);
        bytes.extend_from_slice(&self.epoch.to_be_bytes());
        bytes.extend_from_slice(&id.to_be_bytes());
        bytes.extend_from_slice(&[service, 0, 0]);
        bytes.extend_from_slice(payload);
        bytes
    }
    fn enqueue(&mut self, kind: u8, id: u32, service: UsbService) -> io::Result<()> {
        if self.control.len() >= MAX_CONTROL {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.control
            .push_back(self.frame(kind, id, service as u8, &[]));
        Ok(())
    }
    fn ready(&self) -> bool {
        self.enabled && self.hello_sent && self.remote_epoch.is_some()
    }
    fn retire(&mut self, id: u32) {
        if let Some(stream) = self.streams.remove(&id) {
            self.remember_closed(id, stream.service);
        }
    }
    fn remember_closed(&mut self, id: u32, service: UsbService) {
        if !self.closed.iter().any(|(old, _)| *old == id) {
            if self.closed.len() == MAX_CLOSED {
                self.closed.pop_front();
            }
            self.closed.push_back((id, service));
        }
    }
    fn cancel(&mut self, id: u32) -> io::Result<()> {
        if let Some(service) = self.streams.get(&id).map(|s| s.service) {
            self.enqueue(CLOSE, id, service)?;
            self.retire(id);
        }
        Ok(())
    }
    pub(super) fn receive(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() < HEADER || bytes.len() > 2044 || &bytes[..4] != b"EUSB" {
            return invalid();
        }
        if bytes[4] != VERSION {
            return Err(io::ErrorKind::Unsupported.into());
        }
        if bytes[19..21] != [0, 0] {
            return invalid();
        }
        let kind = bytes[5];
        let epoch = u64::from_be_bytes(bytes[6..14].try_into().unwrap());
        let id = u32::from_be_bytes(bytes[14..18].try_into().unwrap());
        let service = bytes[18];
        let payload = &bytes[HEADER..];
        if epoch == 0 {
            return invalid();
        }
        if kind == HELLO {
            if id != 0 || service != 0 || payload != [0, 0, 0, 1] {
                return invalid();
            }
            if self.remote_epoch.is_some_and(|old| old != epoch) {
                return invalid();
            }
            self.remote_epoch = Some(epoch);
            return Ok(());
        }
        if !self.ready() || self.remote_epoch != Some(epoch) || id == 0 {
            return invalid();
        }
        let service = UsbService::parse(service)?;
        if kind != DATA && !payload.is_empty() {
            return invalid();
        }
        if kind == OPEN {
            if (id % 2 == 1) == self.local_odd
                || self.streams.contains_key(&id)
                || id <= self.last_remote
            {
                return invalid();
            }
            self.last_remote = id;
            if self.streams.len() >= MAX_STREAMS {
                self.remember_closed(id, service);
                return self.enqueue(REJECT, id, service);
            }
            let (application, bridge) = private_pair(Instant::now() + Duration::from_secs(1))?;
            self.streams.insert(
                id,
                Stream {
                    service,
                    local: false,
                    acknowledged: true,
                    rejected: false,
                    application: Some(application),
                    bridge,
                    incoming: VecDeque::new(),
                    incoming_bytes: 0,
                    offset: 0,
                    outgoing: None,
                    opened: Instant::now(),
                    incoming_stalled: None,
                    outgoing_stalled: None,
                    remote_closed: false,
                },
            );
            return self.enqueue(ACK, id, service);
        }
        // Explicitly cancelled/closed sockets may have opposite-direction bytes
        // already in flight. Discard only after that terminal local outcome.
        if kind == DATA && (payload.is_empty() || payload.len() > CHUNK) {
            return invalid();
        }
        if let Some((_, old_service)) = self.closed.iter().find(|(old, _)| *old == id) {
            if *old_service != service {
                return invalid();
            }
            return match kind {
                ACK | DATA | CLOSE | REJECT => Ok(()),
                _ => invalid(),
            };
        }
        let stream = self
            .streams
            .get_mut(&id)
            .ok_or(io::ErrorKind::InvalidData)?;
        if stream.service != service {
            return invalid();
        }
        match kind {
            ACK if stream.local && !stream.acknowledged && !stream.rejected => {
                stream.acknowledged = true
            }
            REJECT if stream.local && !stream.acknowledged && !stream.rejected => {
                stream.rejected = true
            }
            DATA if stream.acknowledged
                && !stream.remote_closed
                && !payload.is_empty()
                && payload.len() <= CHUNK =>
            {
                if stream.incoming_bytes + payload.len() > MAX_INCOMING {
                    return Err(io::ErrorKind::OutOfMemory.into());
                }
                stream.incoming_bytes += payload.len();
                stream.incoming.push_back(payload.to_vec());
            }
            CLOSE if !stream.remote_closed => stream.remote_closed = true,
            _ => return invalid(),
        }
        Ok(())
    }
    pub(super) fn shutdown(&mut self) {
        self.streams.clear();
        self.control.clear();
        self.closed.clear();
        self.enabled = false;
    }
    pub(super) fn drive(&mut self, hub: &UsbHub) -> io::Result<()> {
        while let Some(packet) = self.control.front() {
            match hub.send_stream_packet(packet) {
                Ok(()) => {
                    self.control.pop_front();
                    self.control_stalled = None;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let since = self.control_stalled.get_or_insert_with(Instant::now);
                    if since.elapsed() > STALL {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }
        self.control_stalled = None;
        let ids = self.streams.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let mut terminate = false;
            let stream = self.streams.get_mut(&id).unwrap();
            if stream.application.is_some() && stream.opened.elapsed() > STALL {
                terminate = true;
            } else if stream.rejected {
                continue; // Opening caller consumes the bounded rejection.
            } else if stream.acknowledged {
                let mut incoming_progress = false;
                // Flush received bytes in order before a remote terminal close.
                while let Some(front) = stream.incoming.front() {
                    match stream.bridge.write(&front[stream.offset..]) {
                        Ok(0) => {
                            terminate = true;
                            break;
                        }
                        Ok(written) => {
                            incoming_progress = true;
                            stream.offset += written;
                            stream.incoming_bytes -= written;
                            if stream.offset == front.len() {
                                stream.incoming.pop_front();
                                stream.offset = 0;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            terminate = true;
                            break;
                        }
                    }
                }
                if stream.remote_closed && stream.incoming.is_empty() {
                    terminate = true;
                }
                if !terminate && !stream.remote_closed {
                    if stream.outgoing.is_none() {
                        let mut data = vec![0; CHUNK];
                        match stream.bridge.read(&mut data) {
                            Ok(0) => terminate = true,
                            Ok(size) => {
                                data.truncate(size);
                                stream.outgoing = Some(data)
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(_) => terminate = true,
                        }
                    }
                    if let Some(data) = &stream.outgoing {
                        let mut packet = Vec::with_capacity(HEADER + data.len());
                        packet.extend_from_slice(b"EUSB");
                        packet.extend_from_slice(&[VERSION, DATA]);
                        packet.extend_from_slice(&self.epoch.to_be_bytes());
                        packet.extend_from_slice(&id.to_be_bytes());
                        packet.extend_from_slice(&[stream.service as u8, 0, 0]);
                        packet.extend_from_slice(data);
                        match hub.send_stream_packet(&packet) {
                            Ok(()) => {
                                stream.outgoing = None;
                                stream.outgoing_stalled = None;
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
                if stream.incoming_bytes != 0 {
                    if incoming_progress {
                        stream.incoming_stalled = Some(Instant::now());
                    }
                    let stalled = stream.incoming_stalled.get_or_insert_with(Instant::now);
                    if stalled.elapsed() > STALL {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                } else {
                    stream.incoming_stalled = None;
                }
                if stream.outgoing.is_some() {
                    let stalled = stream.outgoing_stalled.get_or_insert_with(Instant::now);
                    if stalled.elapsed() > STALL {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                } else {
                    stream.outgoing_stalled = None;
                }
            }
            if terminate {
                self.cancel(id)?;
            }
        }
        Ok(())
    }
}
impl UsbHub {
    /// Opt in before any signaling opens. Legacy media-only construction does
    /// not transmit a new frame. The platform adapter owns identity and epoch.
    pub fn enable_streams(self: &Arc<Self>) -> io::Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let mut worker = self
            .stream_worker
            .lock()
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if worker.is_some() {
            return Ok(());
        }
        {
            let mut state = self.streams.lock().map_err(|_| io::ErrorKind::BrokenPipe)?;
            if self.failed.load(Ordering::Acquire) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            state.enabled = true;
            let hello = state.frame(HELLO, 0, 0, &[0, 0, 0, 1]);
            state.control.push_back(hello);
        }
        let weak = Arc::downgrade(self);
        let spawned = thread::Builder::new()
            .name("extend-usb-signaling".into())
            .spawn(move || {
                loop {
                    let Some(hub) = weak.upgrade() else { break };
                    if hub.failed.load(Ordering::Acquire) {
                        break;
                    }
                    let result = hub.pump().and_then(|_| {
                        let mut state =
                            hub.streams.lock().map_err(|_| io::ErrorKind::BrokenPipe)?;
                        state.drive(&hub)?;
                        // Hello is first in the FIFO; readiness begins only once sent.
                        state.hello_sent =
                            state.enabled && state.control.iter().all(|p| p[5] != HELLO);
                        Ok(())
                    });
                    if result.is_err() {
                        hub.fail();
                        break;
                    }
                    drop(hub);
                    thread::sleep(Duration::from_millis(2));
                }
            });
        match spawned {
            Ok(started) => {
                *worker = Some(started);
                Ok(())
            }
            Err(error) => {
                self.fail();
                Err(error)
            }
        }
    }
    pub fn streams_ready(&self) -> bool {
        !self.failed.load(Ordering::Acquire) && self.streams.lock().is_ok_and(|s| s.ready())
    }
    /// The caller's original deadline covers capability negotiation and OPEN.
    /// Cancellation does not extend that deadline or authenticate the USB hint.
    pub fn open_stream(
        self: &Arc<Self>,
        service: UsbService,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> io::Result<TcpStream> {
        let mut id = None;
        loop {
            let failure = if self.failed.load(Ordering::Acquire) {
                Some(io::ErrorKind::BrokenPipe)
            } else if stop.load(Ordering::Acquire) {
                Some(io::ErrorKind::Interrupted)
            } else if Instant::now() >= deadline {
                Some(io::ErrorKind::TimedOut)
            } else {
                None
            };
            if let Some(kind) = failure {
                if let Some(id) = id {
                    if let Ok(mut state) = self.streams.lock() {
                        if state.cancel(id).is_err() {
                            drop(state);
                            self.fail();
                        }
                    }
                }
                return Err(kind.into());
            }
            {
                let mut state = self.streams.lock().map_err(|_| io::ErrorKind::BrokenPipe)?;
                if !state.enabled {
                    return Err(io::ErrorKind::Unsupported.into());
                }
                if state.ready() && id.is_none() {
                    if state.streams.len() >= MAX_STREAMS {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                    let next = state.next;
                    state.next = next.checked_add(2).ok_or(io::ErrorKind::AddrNotAvailable)?;
                    let (application, bridge) = private_pair(deadline)?;
                    state.streams.insert(
                        next,
                        Stream {
                            service,
                            local: true,
                            acknowledged: false,
                            rejected: false,
                            application: Some(application),
                            bridge,
                            incoming: VecDeque::new(),
                            incoming_bytes: 0,
                            offset: 0,
                            outgoing: None,
                            opened: Instant::now(),
                            incoming_stalled: None,
                            outgoing_stalled: None,
                            remote_closed: false,
                        },
                    );
                    if let Err(e) = state.enqueue(OPEN, next, service) {
                        state.retire(next);
                        return Err(e);
                    }
                    id = Some(next);
                }
                if let Some(id) = id {
                    let stream = state
                        .streams
                        .get_mut(&id)
                        .ok_or(io::ErrorKind::ConnectionAborted)?;
                    if stream.rejected {
                        state.retire(id);
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                    if stream.acknowledged {
                        return stream
                            .application
                            .take()
                            .ok_or_else(|| io::ErrorKind::ConnectionAborted.into());
                    }
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
    /// Only the three fixed protocol services are returned; no destination
    /// address is accepted or exposed by the bridge implementation.
    pub fn accept_stream(&self) -> io::Result<Option<(UsbService, TcpStream)>> {
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let mut state = self.streams.lock().map_err(|_| io::ErrorKind::BrokenPipe)?;
        if !state.ready() {
            return Ok(None);
        }
        Ok(state
            .streams
            .values_mut()
            .find(|s| !s.local && !s.remote_closed && s.application.is_some())
            .map(|s| (s.service, s.application.take().unwrap())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{packet_link::PacketDriver, DatagramLink};
    struct Pair {
        a: Arc<UsbHub>,
        b: Arc<UsbHub>,
        stop: Arc<AtomicBool>,
        pause: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }
    fn deliver(source: &mut PacketDriver, target: &mut PacketDriver) -> io::Result<()> {
        for _ in 0..16 {
            let Some(frame) = source.take_outgoing()? else {
                break;
            };
            // Real PacketDrivers see every length/header/body split, not a mock stream decoder.
            for chunk in frame.chunks(7) {
                let mut used = 0;
                while used < chunk.len() {
                    let consumed = target.receive_bytes(&chunk[used..])?;
                    if consumed == 0 {
                        thread::sleep(Duration::from_millis(1));
                    }
                    used += consumed;
                }
            }
        }
        Ok(())
    }
    impl Pair {
        fn new() -> Self {
            let (a, mut ad) =
                UsbHub::new("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()).unwrap();
            let (b, mut bd) =
                UsbHub::new("192.0.2.2".parse().unwrap(), "192.0.2.1".parse().unwrap()).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let pause = Arc::new(AtomicBool::new(false));
            let paused = pause.clone();
            let worker = thread::spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    if paused.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    if deliver(&mut ad, &mut bd)
                        .and_then(|_| deliver(&mut bd, &mut ad))
                        .is_err()
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            });
            a.enable_streams().unwrap();
            b.enable_streams().unwrap();
            Self {
                a,
                b,
                stop,
                pause,
                worker: Some(worker),
            }
        }
        fn open(&self, from_a: bool, service: UsbService) -> (TcpStream, TcpStream) {
            let (source, target) = if from_a {
                (&self.a, &self.b)
            } else {
                (&self.b, &self.a)
            };
            let socket = source
                .open_stream(
                    service,
                    &AtomicBool::new(false),
                    Instant::now() + Duration::from_secs(2),
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some((observed, remote)) = target.accept_stream().unwrap() {
                    assert_eq!(observed, service);
                    for stream in [&socket, &remote] {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                    }
                    return (socket, remote);
                }
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    impl Drop for Pair {
        fn drop(&mut self) {
            self.a.close();
            self.b.close();
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }
    #[test]
    fn stream_first_fragmented_32k_both_directions_then_media_and_close() {
        let pair = Pair::new();
        for (side, service) in [
            (true, UsbService::Presence),
            (false, UsbService::Control),
            (true, UsbService::Display),
        ] {
            let (mut local, mut remote) = pair.open(side, service);
            let data = (0..32768).map(|n| (n % 251) as u8).collect::<Vec<_>>();
            local.write_all(&data).unwrap();
            let mut received = vec![0; data.len()];
            remote.read_exact(&mut received).unwrap();
            assert_eq!(data, received);
            remote.write_all(&data).unwrap();
            local.read_exact(&mut received).unwrap();
            assert_eq!(data, received);
            local.shutdown(Shutdown::Both).unwrap();
            assert_eq!(remote.read(&mut [0]).unwrap(), 0);
        }
        let a = pair.a.endpoint().unwrap();
        let b = pair.b.endpoint().unwrap();
        a.send_to(b"unchanged-media", b.local_addr().unwrap())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut data = [0; 64];
        loop {
            match b.recv_from(&mut data) {
                Ok((size, source)) => {
                    assert_eq!(&data[..size], b"unchanged-media");
                    assert_eq!(source, a.local_addr().unwrap());
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(e) => panic!("{e}"),
            }
            assert!(Instant::now() < deadline);
        }
        assert_eq!(pair.a.counters().0, 1);
        assert_eq!(pair.b.counters().1, 1);
    }
    #[test]
    fn four_combined_streams_capacity_and_slot_reuse() {
        let pair = Pair::new();
        let mut sockets = Vec::new();
        for side in [true, false, true, false] {
            sockets.push(pair.open(side, UsbService::Control));
        }
        assert_eq!(
            pair.a
                .open_stream(
                    UsbService::Display,
                    &AtomicBool::new(false),
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        sockets.remove(0);
        let deadline = Instant::now() + Duration::from_secs(1);
        while pair.a.streams.lock().unwrap().streams.len() == 4 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let _replacement = pair.open(true, UsbService::Display);
    }
    #[test]
    fn driver_drop_without_media_and_hub_close_wake_every_socket() {
        let pair = Pair::new();
        let (mut local, mut remote) = pair.open(true, UsbService::Control);
        pair.stop.store(true, Ordering::Release); // Dropping real drivers is physical EOF/detach.
        let deadline = Instant::now() + Duration::from_secs(1);
        while !pair.a.failed.load(Ordering::Acquire) || !pair.b.failed.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(local.read(&mut [0]).unwrap(), 0);
        assert_eq!(remote.read(&mut [0]).unwrap(), 0);
        assert!(!pair.a.streams_ready());
        let pair = Pair::new();
        let (mut local, _) = pair.open(true, UsbService::Display);
        pair.a.close();
        assert_eq!(local.read(&mut [0]).unwrap(), 0);
    }
    #[test]
    fn malformed_inflight_service_closes_carrier_and_all_private_sockets() {
        let pair = Pair::new();
        let (mut local, mut remote) = pair.open(true, UsbService::Presence);
        let bad = pair.b.streams.lock().unwrap().frame(
            DATA,
            1,
            UsbService::Control as u8,
            b"wrong-service",
        );
        pair.b.send_stream_packet(&bad).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !pair.a.failed.load(Ordering::Acquire) || !pair.b.failed.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(local.read(&mut [0]).unwrap(), 0);
        assert_eq!(remote.read(&mut [0]).unwrap(), 0);
        assert!(pair.a.streams.lock().unwrap().streams.is_empty());
        assert!(pair.b.streams.lock().unwrap().streams.is_empty());
    }
    #[test]
    fn disabled_old_peer_deadline_and_cancel_do_not_open_or_authenticate() {
        let (hub, _driver) =
            UsbHub::new("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()).unwrap();
        assert_eq!(
            hub.open_stream(
                UsbService::Presence,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Unsupported
        );
        hub.enable_streams().unwrap();
        assert_eq!(
            hub.open_stream(
                UsbService::Presence,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_millis(25)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            hub.open_stream(
                UsbService::Presence,
                &AtomicBool::new(true),
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(hub.streams.lock().unwrap().streams.is_empty());
        hub.close();
    }
    #[test]
    fn cancelled_open_after_hello_frees_slot_and_bounded_socket() {
        let pair = Pair::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !pair.a.streams_ready() || !pair.b.streams_ready() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        pair.pause.store(true, Ordering::Release);
        thread::sleep(Duration::from_millis(10));
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let hub = pair.a.clone();
        let opener = thread::spawn(move || {
            hub.open_stream(
                UsbService::Control,
                &stopped,
                Instant::now() + Duration::from_secs(1),
            )
        });
        while pair.a.streams.lock().unwrap().streams.is_empty() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::Release);
        assert_eq!(
            opener.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(pair.a.streams.lock().unwrap().streams.is_empty());
        pair.pause.store(false, Ordering::Release);
        // OPEN followed by explicit CLOSE must not leave an accepted live socket.
        thread::sleep(Duration::from_millis(30));
        assert!(pair.b.streams.lock().unwrap().streams.is_empty());
        let _replacement = pair.open(false, UsbService::Display);
    }
    fn negotiated() -> (StreamState, StreamState) {
        let mut local = StreamState::new(true);
        let remote = StreamState::new(false);
        local.enabled = true;
        local.hello_sent = true;
        local
            .receive(&remote.frame(HELLO, 0, 0, &[0, 0, 0, 1]))
            .unwrap();
        (local, remote)
    }
    #[test]
    fn malformed_version_epoch_service_parity_and_reused_ids_rejected() {
        let (mut local, remote) = negotiated();
        let open = remote.frame(OPEN, 2, UsbService::Presence as u8, &[]);
        for corrupt in [4usize, 6, 18, 19] {
            let mut bytes = open.clone();
            bytes[corrupt] ^= 0x7f;
            assert!(local.receive(&bytes).is_err());
        }
        assert!(local.receive(&remote.frame(OPEN, 1, 1, &[])).is_err());
        assert!(local.receive(&remote.frame(OPEN, 2, 1, &[1])).is_err());
        local.receive(&open).unwrap();
        assert!(local.receive(&open).is_err());
        assert!(local
            .receive(&remote.frame(DATA, 2, 2, b"wrong-service"))
            .is_err());
        assert!(local.receive(&remote.frame(DATA, 2, 1, &[])).is_err());
    }
    #[test]
    fn progress_clears_incoming_stall_without_masking_stalled_outgoing() {
        let (hub, _driver) =
            UsbHub::new("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()).unwrap();
        let (mut local, remote) = negotiated();
        local.receive(&remote.frame(OPEN, 2, 1, &[])).unwrap();
        local.control.clear();
        local
            .receive(&remote.frame(DATA, 2, 1, b"delivered-incoming"))
            .unwrap();
        let stream = local.streams.get_mut(&2).unwrap();
        let mut application = stream.application.take().unwrap();
        application
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream.incoming_stalled = Some(Instant::now() - STALL - Duration::from_secs(1));
        stream.outgoing = Some(vec![1; CHUNK]);
        loop {
            match hub.send_stream_packet(&remote.frame(HELLO, 0, 0, &[0, 0, 0, 1])) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
        local.drive(&hub).unwrap(); // Incoming made progress even though outgoing is blocked.
        let mut observed = vec![0; 18];
        application.read_exact(&mut observed).unwrap();
        assert_eq!(observed, b"delivered-incoming");
        assert!(local.streams[&2].incoming_stalled.is_none());
        assert!(local.streams[&2].outgoing_stalled.is_some());
        local.streams.get_mut(&2).unwrap().outgoing_stalled =
            Some(Instant::now() - STALL - Duration::from_secs(1));
        local
            .receive(&remote.frame(DATA, 2, 1, b"more-incoming"))
            .unwrap();
        assert_eq!(
            local.drive(&hub).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
    #[test]
    fn incoming_overflow_and_control_overflow_fail_instead_of_dropping() {
        let (mut local, remote) = negotiated();
        local.receive(&remote.frame(OPEN, 2, 1, &[])).unwrap();
        let data = remote.frame(DATA, 2, 1, &vec![1; CHUNK]);
        for _ in 0..MAX_INCOMING / CHUNK {
            local.receive(&data).unwrap();
        }
        assert_eq!(
            local.receive(&data).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        let (mut local, remote) = negotiated();
        for id in (2..=8).step_by(2) {
            local.receive(&remote.frame(OPEN, id, 1, &[])).unwrap();
        }
        for id in (10..=64).step_by(2) {
            local.receive(&remote.frame(OPEN, id, 1, &[])).unwrap();
        }
        assert_eq!(local.streams.len(), 4);
        assert_eq!(local.control.len(), 32);
        assert_eq!(
            local
                .receive(&remote.frame(OPEN, 66, 1, &[]))
                .unwrap_err()
                .kind(),
            io::ErrorKind::OutOfMemory
        );
    }
}
