//! Native encoded-video transport. SDP must travel through authenticated signaling.
use crate::session::display::{DisplayFeatures, VideoCodec};
pub mod input;
#[cfg(target_os = "macos")]
mod kernel_arrival;
mod link_local;
pub mod metrics;
pub mod packet_link;
mod queue_metrics;
#[cfg(target_os = "macos")]
mod socket_timing;
mod udp_recovery;
#[cfg(target_os = "macos")]
pub mod usb_host;
pub mod usb_link;
pub mod usb_stream;
use anyhow::{ensure, Context, Result};
use std::{
    collections::VecDeque,
    net::{IpAddr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::{ChannelConfig, ChannelId, Reliability};
use str0m::format::Codec;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc, RtcConfig};

// Development-only comparison of IPv4 socket-wide DSCP. The default remains
// best effort; networks may ignore or reject priority markings.
fn experimental_dscp(value: Option<&str>) -> Result<Option<u32>> {
    value
        .map(|value| {
            let dscp: u32 = value.parse().context("Invalid display DSCP")?;
            ensure!(dscp <= 63, "Display DSCP must be 0..63");
            Ok(dscp << 2) // Leave the ECN bits clear.
        })
        .transpose()
}

fn experimental_reordering(value: Option<&str>) -> Result<Duration> {
    let milliseconds = match value {
        None => 500,
        Some(value) => value
            .parse::<u64>()
            .context("Invalid display reordering deadline")?,
    };
    ensure!(
        (20..=1000).contains(&milliseconds),
        "Display reordering deadline must be 20..1000 ms"
    );
    Ok(Duration::from_millis(milliseconds))
}

#[cfg(target_os = "macos")]
fn responsive_screen_service(socket: &UdpSocket) -> Result<()> {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn setsockopt(
            fd: i32,
            level: i32,
            name: i32,
            value: *const std::ffi::c_void,
            length: u32,
        ) -> i32;
        fn getsockopt(
            fd: i32,
            level: i32,
            name: i32,
            value: *mut std::ffi::c_void,
            length: *mut u32,
        ) -> i32;
    }
    // Public Darwin sys/socket.h: responsive multimedia explicitly includes
    // screen sharing; the service type selects host/Wi-Fi queue treatment.
    const SOL_SOCKET: i32 = 0xffff;
    const SO_NET_SERVICE_TYPE: i32 = 0x1116;
    let requested: i32 = 5; // NET_SERVICE_TYPE_RV, not the voice class.
    let mut observed: i32 = -1;
    let mut length = std::mem::size_of::<i32>() as u32;
    // Both buffers and the length pointer remain valid for these synchronous
    // calls. The socket owns its descriptor throughout the calls.
    let set = unsafe {
        setsockopt(
            socket.as_raw_fd(),
            SOL_SOCKET,
            SO_NET_SERVICE_TYPE,
            &requested as *const i32 as *const std::ffi::c_void,
            length,
        )
    };
    if set != 0 {
        return Err(std::io::Error::last_os_error())
            .context("Could not set screen-sharing service type");
    }
    let get = unsafe {
        getsockopt(
            socket.as_raw_fd(),
            SOL_SOCKET,
            SO_NET_SERVICE_TYPE,
            &mut observed as *mut i32 as *mut std::ffi::c_void,
            &mut length,
        )
    };
    if get != 0 {
        return Err(std::io::Error::last_os_error())
            .context("Could not verify screen-sharing service type");
    }
    ensure!(
        length == std::mem::size_of::<i32>() as u32 && observed == requested,
        "Screen-sharing service type was not applied"
    );
    Ok(())
}

fn local_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback(),
    }
}

fn cursor_is_shape(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .is_some_and(|v| v.get("png").is_some())
}
pub fn validate_cursor(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= 128 * 1024, "Cursor message too large");
    let v: serde_json::Value = serde_json::from_slice(bytes)?;
    ensure!(v["revision"].as_u64().is_some(), "Invalid cursor revision");
    if let Some(png) = v.get("png") {
        ensure!(
            png.as_str().is_some_and(|s| s.len() <= 120 * 1024),
            "Invalid cursor image"
        );
        for key in ["width", "height"] {
            ensure!(
                v[key].as_f64().is_some_and(|n| n > 0.0 && n <= 512.0),
                "Invalid cursor size"
            );
        }
        for key in ["hotspot_x", "hotspot_y"] {
            ensure!(
                v[key].as_f64().is_some_and(|n| (0.0..=512.0).contains(&n)),
                "Invalid cursor hotspot"
            );
        }
    } else {
        ensure!(
            v["sequence"].as_u64().is_some() && v["visible"].is_boolean(),
            "Invalid cursor position"
        );
        for key in ["x", "y"] {
            ensure!(
                v[key].as_f64().is_some_and(|n| (0.0..=1.0).contains(&n)),
                "Invalid cursor coordinates"
            );
        }
    }
    Ok(())
}

// Opt-in numeric timing only: inspect the clear RTP header, never media payload.
// Socket timestamps are userspace observations, not kernel/NIC arrival times.
#[derive(Clone, Copy, Debug)]
struct TimingRtpHeader {
    pt: u8,
    ssrc: u32,
    timestamp: u32,
    sequence: u16,
    marker: bool,
    padding_flag: bool,
}
fn timing_rtp_header(bytes: &[u8]) -> Option<TimingRtpHeader> {
    if bytes.len() < 12 || bytes[0] >> 6 != 2 || (192..=223).contains(&bytes[1]) {
        return None; // STUN, DTLS and multiplexed RTCP are not RTP.
    }
    let mut length = 12 + usize::from(bytes[0] & 15) * 4;
    if bytes.len() < length {
        return None;
    }
    if bytes[0] & 16 != 0 {
        if bytes.len() < length + 4 {
            return None;
        }
        let words = u16::from_be_bytes([bytes[length + 2], bytes[length + 3]]) as usize;
        length += 4 + words * 4;
        if bytes.len() < length {
            return None;
        }
    }
    Some(TimingRtpHeader {
        pt: bytes[1] & 127,
        marker: bytes[1] & 128 != 0,
        padding_flag: bytes[0] & 32 != 0,
        sequence: u16::from_be_bytes(bytes[2..4].try_into().ok()?),
        timestamp: u32::from_be_bytes(bytes[4..8].try_into().ok()?),
        ssrc: u32::from_be_bytes(bytes[8..12].try_into().ok()?),
    })
}
#[derive(Default)]
struct TimingDistribution {
    last: Option<Instant>,
    count: u64,
    sum_us: u128,
    max_us: u128,
    bins: [u64; 6],
}
impl TimingDistribution {
    fn record(&mut self, duration: Duration) {
        let us = duration.as_micros();
        self.count += 1;
        self.sum_us += us;
        self.max_us = self.max_us.max(us);
        let bin = [3_000, 12_000, 25_000, 50_000, 100_000]
            .iter()
            .position(|&limit| us < limit)
            .unwrap_or(5);
        self.bins[bin] += 1;
    }
    fn at(&mut self, now: Instant) {
        if let Some(last) = self.last {
            self.record(now.saturating_duration_since(last));
        }
        self.last = Some(now);
    }
    fn take_json(&mut self) -> serde_json::Value {
        let value = serde_json::json!({"count":self.count,
            "mean_ms":(self.count > 0).then(||self.sum_us as f64 / self.count as f64 / 1000.0),
            "max_ms":(self.count > 0).then(||self.max_us as f64 / 1000.0),
            "bins_ms_lt_3_12_25_50_100_else":self.bins});
        self.count = 0;
        self.sum_us = 0;
        self.max_us = 0;
        self.bins = [0; 6];
        value
    }
}
struct TimingStream {
    pt: u8,
    ssrc: u32,
    packets: u64,
    udp_payload_bytes: u64,
    padding_flag_packets: u64,
    padding_flag_udp_payload_bytes: u64,
    packet_gaps: TimingDistribution,
    group_gaps: TimingDistribution,
    recent: VecDeque<u32>,
}
#[derive(Default)]
struct TimingDirection {
    udp_packets: u64,
    udp_gaps: TimingDistribution,
    streams: Vec<TimingStream>,
    untracked_stream_packets: u64,
}
impl TimingDirection {
    fn observe(&mut self, bytes: &[u8], now: Instant) -> Option<TimingRtpHeader> {
        self.udp_packets += 1;
        self.udp_gaps.at(now);
        let header = timing_rtp_header(bytes)?;
        let index = if let Some(index) = self
            .streams
            .iter()
            .position(|s| s.pt == header.pt && s.ssrc == header.ssrc)
        {
            index
        } else {
            if self.streams.len() >= 8 {
                self.untracked_stream_packets += 1;
                return None;
            }
            self.streams.push(TimingStream {
                pt: header.pt,
                ssrc: header.ssrc,
                packets: 0,
                udp_payload_bytes: 0,
                padding_flag_packets: 0,
                padding_flag_udp_payload_bytes: 0,
                packet_gaps: TimingDistribution::default(),
                group_gaps: TimingDistribution::default(),
                recent: VecDeque::new(),
            });
            self.streams.len() - 1
        };
        let stream = &mut self.streams[index];
        stream.packets += 1;
        stream.udp_payload_bytes += bytes.len() as u64;
        if header.padding_flag {
            stream.padding_flag_packets += 1;
            stream.padding_flag_udp_payload_bytes += bytes.len() as u64;
        }
        stream.packet_gaps.at(now);
        if !stream.recent.contains(&header.timestamp) {
            stream.group_gaps.at(now);
            stream.recent.push_back(header.timestamp);
            if stream.recent.len() > 32 {
                stream.recent.pop_front();
            }
        }
        Some(header)
    }
    fn take_json(&mut self) -> serde_json::Value {
        let streams:Vec<_>=self.streams.iter_mut().enumerate().map(|(index,s)| {
            let value=serde_json::json!({"session_stream_index":index,"payload_type":s.pt,"packets":s.packets,
                "udp_payload_bytes":s.udp_payload_bytes,"padding_flag_packets":s.padding_flag_packets,
                "padding_flag_udp_payload_bytes":s.padding_flag_udp_payload_bytes,
                "packet_gaps":s.packet_gaps.take_json(),"first_seen_rtp_group_gaps":s.group_gaps.take_json()});
            s.packets=0; s.udp_payload_bytes=0; s.padding_flag_packets=0;
            s.padding_flag_udp_payload_bytes=0; value
        }).collect();
        let value = serde_json::json!({"udp_packets":self.udp_packets,"udp_gaps":self.udp_gaps.take_json(),
            "rtp_streams":streams,"untracked_stream_packets":self.untracked_stream_packets});
        self.udp_packets = 0;
        self.untracked_stream_packets = 0;
        value
    }
}
struct TimingFrame {
    header: TimingRtpHeader,
    first: Instant,
    last: Instant,
    first_sequence: u16,
    last_sequence: u16,
    packets: u64,
}
struct KernelTimingStream {
    pt: u8,
    ssrc: u32,
    last_group_ns: Option<u64>,
    recent: VecDeque<u32>,
    group_gaps: TimingDistribution,
}
#[derive(Default)]
struct KernelTiming {
    timestamped: u64,
    missing: u64,
    invalid_clock: u64,
    last_ns: Option<u64>,
    packet_gaps: TimingDistribution,
    queue_delay: TimingDistribution,
    streams: Vec<KernelTimingStream>,
}
impl KernelTiming {
    fn gap(distribution: &mut TimingDistribution, previous: &mut Option<u64>, now: u64) -> bool {
        let valid = if let Some(last) = *previous {
            if let Some(gap) = now.checked_sub(last) {
                distribution.record(Duration::from_nanos(gap));
                true
            } else {
                false
            }
        } else {
            true
        };
        *previous = Some(now);
        valid
    }
    fn observe(&mut self, bytes: &[u8], stamp: Option<u64>, read_wall: Option<u64>) {
        let Some(stamp) = stamp else {
            self.missing += 1;
            return;
        };
        self.timestamped += 1;
        if !Self::gap(&mut self.packet_gaps, &mut self.last_ns, stamp) {
            self.invalid_clock += 1;
        }
        match read_wall.and_then(|read| read.checked_sub(stamp)) {
            Some(delay) if delay <= 60_000_000_000 => {
                self.queue_delay.record(Duration::from_nanos(delay))
            }
            _ => self.invalid_clock += 1,
        }
        let Some(header) = timing_rtp_header(bytes) else {
            return;
        };
        let index = if let Some(index) = self
            .streams
            .iter()
            .position(|s| s.pt == header.pt && s.ssrc == header.ssrc)
        {
            index
        } else {
            if self.streams.len() >= 8 {
                return;
            }
            self.streams.push(KernelTimingStream {
                pt: header.pt,
                ssrc: header.ssrc,
                last_group_ns: None,
                recent: VecDeque::new(),
                group_gaps: TimingDistribution::default(),
            });
            self.streams.len() - 1
        };
        let stream = &mut self.streams[index];
        if !stream.recent.contains(&header.timestamp) {
            if !Self::gap(&mut stream.group_gaps, &mut stream.last_group_ns, stamp) {
                self.invalid_clock += 1;
            }
            stream.recent.push_back(header.timestamp);
            if stream.recent.len() > 32 {
                stream.recent.pop_front();
            }
        }
    }
    fn take_json(&mut self) -> serde_json::Value {
        let streams: Vec<_> = self
            .streams
            .iter_mut()
            .enumerate()
            .map(|(index, s)| {
                serde_json::json!({"session_stream_index":index,"payload_type":s.pt,
                "first_seen_rtp_group_gaps":s.group_gaps.take_json()})
            })
            .collect();
        let result = serde_json::json!({"timestamped_datagrams":self.timestamped,"missing_timestamp_datagrams":self.missing,
            "invalid_clock_samples":self.invalid_clock,"udp_gaps":self.packet_gaps.take_json(),
            "kernel_to_userspace":self.queue_delay.take_json(),"rtp_streams":streams,
            "clock":"Darwin SO_TIMESTAMP realtime clock, microsecond precision; receiver-local only",
            "scope":"Socket ingress timestamp, not radio arrival, frame readiness, panel latency or cross-host latency; realtime clock adjustments can invalidate gaps"});
        self.timestamped = 0;
        self.missing = 0;
        self.invalid_clock = 0;
        result
    }
}
struct TransportTiming {
    origin: Instant,
    since: Instant,
    receive: TimingDirection,
    kernel_receive: KernelTiming,
    send: TimingDirection,
    assembled: u64,
    assembly_gaps: TimingDistribution,
    first_input_to_assembly: TimingDistribution,
    last_read_to_assembly: TimingDistribution,
    frames: VecDeque<TimingFrame>,
    samples: VecDeque<serde_json::Value>,
    slow_samples: Vec<serde_json::Value>,
    unmatched_assembly: u64,
    evicted_groups: u64,
}
impl TransportTiming {
    fn new(now: Instant) -> Self {
        Self {
            origin: now,
            since: now,
            receive: TimingDirection::default(),
            kernel_receive: KernelTiming::default(),
            send: TimingDirection::default(),
            assembled: 0,
            assembly_gaps: TimingDistribution::default(),
            first_input_to_assembly: TimingDistribution::default(),
            last_read_to_assembly: TimingDistribution::default(),
            frames: VecDeque::new(),
            samples: VecDeque::new(),
            slow_samples: Vec::new(),
            unmatched_assembly: 0,
            evicted_groups: 0,
        }
    }
    fn kernel_observation(&mut self, bytes: &[u8], stamp: Option<u64>) {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_nanos()).ok());
        self.kernel_receive.observe(bytes, stamp, wall);
    }
    fn received(&mut self, bytes: &[u8], now: Instant) {
        let Some(header) = self.receive.observe(bytes, now) else {
            return;
        };
        if let Some(frame) = self.frames.iter_mut().find(|f| {
            f.header.pt == header.pt
                && f.header.ssrc == header.ssrc
                && f.header.timestamp == header.timestamp
        }) {
            frame.last = now;
            frame.last_sequence = header.sequence;
            frame.packets += 1;
            frame.header.marker |= header.marker;
        } else {
            if self.frames.len() >= 128 {
                self.frames.pop_front();
                self.evicted_groups += 1;
            }
            self.frames.push_back(TimingFrame {
                header,
                first: now,
                last: now,
                first_sequence: header.sequence,
                last_sequence: header.sequence,
                packets: 1,
            });
        }
    }
    fn assembled(
        &mut self,
        pt: u8,
        timestamp: u32,
        network_time: Instant,
        now: Instant,
        sequence_range: (u64, u64),
        contiguous: bool,
    ) {
        self.assembled += 1;
        self.assembly_gaps.at(now);
        self.first_input_to_assembly
            .record(now.saturating_duration_since(network_time));
        let mut matches = self
            .frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.header.pt == pt && f.header.timestamp == timestamp);
        let Some((index, _)) = matches.next() else {
            self.unmatched_assembly += 1;
            return;
        };
        if matches.next().is_some() {
            self.unmatched_assembly += 1;
            return;
        }
        let frame = self.frames.remove(index).unwrap();
        self.last_read_to_assembly
            .record(now.saturating_duration_since(frame.last));
        let sample = serde_json::json!({"payload_type":pt,"rtp_timestamp":timestamp,
            "assembled_sequence_start":sequence_range.0,"assembled_sequence_end":sequence_range.1,
            "contiguous_from_previous":contiguous,
            "first_sequence_seen":frame.first_sequence,"last_sequence_seen":frame.last_sequence,
            "rtp_packets_observed":frame.packets,"marker_seen":frame.header.marker,
            "first_read_us":frame.first.saturating_duration_since(self.origin).as_micros(),
            "last_read_us":frame.last.saturating_duration_since(self.origin).as_micros(),
            "assembled_us":now.saturating_duration_since(self.origin).as_micros(),
            "last_matching_read_to_assembly_us":now.saturating_duration_since(frame.last).as_micros()});
        // Keep the four worst observed holds, rather than losing startup evidence
        // when later fast frames replace the latest-group sample ring.
        if now.saturating_duration_since(frame.last) >= Duration::from_millis(25) {
            self.slow_samples.push(sample.clone());
            self.slow_samples.sort_by_key(|s| {
                std::cmp::Reverse(s["last_matching_read_to_assembly_us"].as_u64().unwrap_or(0))
            });
            self.slow_samples.truncate(4);
        }
        if self.samples.len() >= 4 {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
    fn maybe_log(&mut self, now: Instant) {
        if now.saturating_duration_since(self.since) < Duration::from_secs(5) {
            return;
        }
        let wall_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos());
        let value = serde_json::json!({"schema_version":1,"window_ms":now.saturating_duration_since(self.since).as_millis(),
            "wall_end_ns":wall_ns,"session_elapsed_ms":now.saturating_duration_since(self.origin).as_millis(),
            "clock":"host-local Instant; userspace socket observation; hosts not synchronized",
            "receive":self.receive.take_json(),"kernel_receive":self.kernel_receive.take_json(),"send":self.send.take_json(),"assembled_frames":self.assembled,
            "assembly_gaps":self.assembly_gaps.take_json(),"first_input_to_assembly":self.first_input_to_assembly.take_json(),
            "last_matching_rtp_read_to_assembly":self.last_read_to_assembly.take_json(),
            "unmatched_assembly":self.unmatched_assembly,"evicted_rtp_groups":self.evicted_groups,
            "slow_assembled_group_samples":std::mem::take(&mut self.slow_samples),
            "latest_assembled_group_samples":self.samples.drain(..).collect::<Vec<_>>()});
        eprintln!("Display transport timing: {value}");
        self.since = now;
        self.assembled = 0;
        self.unmatched_assembly = 0;
        self.evicted_groups = 0;
    }
}

pub struct Frame {
    pub timestamp: u64,
    pub keyframe: bool,
    pub bytes: Vec<u8>,
}
pub enum DisplayEvent {
    Connected,
    Frame(Frame),
    Keyframe,
    Bitrate(u64),
    /// Periodic RTCP statistics: round-trip time and the fraction of media
    /// packets lost (0 to 1), when the peer reported them.
    Stats {
        rtt: Option<Duration>,
        loss: Option<f32>,
    },
    Cursor(Vec<u8>),
    Input(input::Message),
    InputClosed,
}
/// Native packet carriers (for example USB accessory bulk endpoints) provide
/// datagram boundaries without replacing the shared DTLS/SRTP media protocol.
/// Methods must not block: receive reports WouldBlock while idle; send reports
/// WouldBlock when its bounded queue is full. Advertised addresses are logical
/// endpoint identifiers exchanged through pinned signaling, not OS interfaces.
pub trait DatagramLink: Send {
    fn local_addr(&self) -> std::io::Result<SocketAddr>;
    fn send_to(&self, bytes: &[u8], destination: SocketAddr) -> std::io::Result<usize>;
    fn recv_from(&self, bytes: &mut [u8]) -> std::io::Result<(usize, SocketAddr)>;
}
enum DatagramSocket {
    Udp(UdpSocket),
    Link(Box<dyn DatagramLink>),
}
impl DatagramSocket {
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        match self {
            Self::Udp(socket) => socket.local_addr(),
            Self::Link(link) => link.local_addr(),
        }
    }
    fn send_to(&self, bytes: &[u8], destination: SocketAddr) -> std::io::Result<usize> {
        let sent = match self {
            Self::Udp(socket) => socket.send_to(bytes, destination),
            Self::Link(link) => link.send_to(bytes, destination),
        }?;
        if sent != bytes.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "Partial display datagram",
            ));
        }
        Ok(sent)
    }
    fn recv_from(&self, bytes: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        let result = match self {
            Self::Udp(socket) => socket.recv_from(bytes),
            Self::Link(link) => link.recv_from(bytes),
        }?;
        if result.0 > bytes.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Oversized display datagram",
            ));
        }
        Ok(result)
    }
    #[cfg(target_os = "macos")]
    fn receive_timestamped(
        &self,
        bytes: &mut [u8],
    ) -> std::io::Result<(usize, SocketAddr, socket_timing::Timestamps)> {
        match self {
            Self::Udp(socket) => socket_timing::receive(socket, bytes),
            Self::Link(_) => self
                .recv_from(bytes)
                .map(|(size, source)| (size, source, socket_timing::Timestamps::default())),
        }
    }
}
pub struct Transport {
    rtc: Rtc,
    codec: VideoCodec,
    cursor_position: Option<ChannelId>,
    cursor_shape: Option<ChannelId>,
    input_channel: Option<ChannelId>,
    text_input: bool,
    shortcuts: bool,
    input_role: Option<input::Role>,
    socket: DatagramSocket,
    link_local_route: Option<link_local::Route>,
    udp_peer: Option<IpAddr>,
    usb_recovery: Option<udp_recovery::Recovery>,
    usb_input_retiring: bool,
    pending_transmit: Option<(SocketAddr, Vec<u8>)>,
    mid: Option<Mid>,
    pending: Option<SdpPendingOffer>,
    events: VecDeque<DisplayEvent>,
    next: Instant,
    last_keyframe: Option<Instant>,
    diagnostics: Option<(Instant, [u64; 7])>,
    timing: Option<TransportTiming>,
    kernel_timestamping: bool,
    #[cfg(target_os = "macos")]
    kernel_arrival: Option<kernel_arrival::Receiver>,
    awaiting_keyframe: bool,
    capture_paused: bool,
    lan_route: bool,
    connected: bool,
    paced_lan: bool,
}
impl Transport {
    pub fn new(ip: IpAddr) -> Result<Self> {
        Self::with_codec(ip, VideoCodec::H264)
    }
    pub fn with_codec(ip: IpAddr, codec: VideoCodec) -> Result<Self> {
        let value = std::env::var("EXTEND_DISPLAY_REORDERING_MS").ok();
        Self::configured(ip, experimental_reordering(value.as_deref())?, codec)
    }
    /// UDP transport for a peer reached over authenticated display signaling.
    /// Link-local endpoints use logical ICE addresses while the real socket
    /// stays on its physical interface and only sends to this peer's IP.
    pub fn with_udp_peer(ip: IpAddr, peer: IpAddr, codec: VideoCodec) -> Result<Self> {
        let value = std::env::var("EXTEND_DISPLAY_REORDERING_MS").ok();
        Self::configured_link(
            ip,
            experimental_reordering(value.as_deref())?,
            codec,
            None,
            Some(peer),
        )
    }
    fn configured(ip: IpAddr, reordering_timeout: Duration, codec: VideoCodec) -> Result<Self> {
        Self::configured_link(ip, reordering_timeout, codec, None, None)
    }
    pub fn with_packet_link(codec: VideoCodec, link: Box<dyn DatagramLink>) -> Result<Self> {
        let address = link.local_addr()?;
        ensure!(
            !address.ip().is_unspecified() && address.port() != 0,
            "Missing native packet endpoint"
        );
        let value = std::env::var("EXTEND_DISPLAY_REORDERING_MS").ok();
        Self::configured_link(
            address.ip(),
            experimental_reordering(value.as_deref())?,
            codec,
            Some(link),
            None,
        )
    }
    fn configured_link(
        ip: IpAddr,
        reordering_timeout: Duration,
        codec: VideoCodec,
        link: Option<Box<dyn DatagramLink>>,
        peer: Option<IpAddr>,
    ) -> Result<Self> {
        ensure!(!ip.is_unspecified(), "Missing local display address");
        static CRYPTO: std::sync::Once = std::sync::Once::new();
        CRYPTO.call_once(|| str0m::crypto::from_feature_flags().install_process_default());
        let socket = match link {
            Some(link) => DatagramSocket::Link(link),
            None => DatagramSocket::Udp(crate::local_network::bind_udp((ip, 0).into())?),
        };
        let link_local_route = peer
            .map(|peer| link_local::Route::new(socket.local_addr()?, peer))
            .transpose()?
            .flatten();
        if let DatagramSocket::Udp(socket) = &socket {
            // Allow short paced RTP/probe bursts without losing packets in the OS
            // before the bounded receive loop can inspect them.
            let buffers = socket2::SockRef::from(&socket);
            buffers.set_recv_buffer_size(2 * 1024 * 1024)?;
            buffers.set_send_buffer_size(512 * 1024)?;
            let dscp = std::env::var("EXTEND_DISPLAY_DSCP").ok();
            if let Some(tos) = experimental_dscp(dscp.as_deref())? {
                ensure!(ip.is_ipv4(), "Display DSCP experiment requires IPv4");
                buffers.set_tos_v4(tos)?;
                ensure!(buffers.tos_v4()? == tos, "Display DSCP was not applied");
            }
            if std::env::var_os("EXTEND_DISPLAY_MACOS_RESPONSIVE").is_some() {
                #[cfg(target_os = "macos")]
                responsive_screen_service(socket)?;
                #[cfg(not(target_os = "macos"))]
                anyhow::bail!("Screen-sharing service experiment requires macOS");
            }
            socket.set_nonblocking(true)?;
            if std::env::var_os("EXTEND_DISPLAY_METRICS").is_some()
                || std::env::var_os("EXTEND_DISPLAY_BENCHMARK_CONTEXT").is_some()
            {
                eprintln!(
                    "Display socket buffers: receive={} send={}",
                    buffers.recv_buffer_size()?,
                    buffers.send_buffer_size()?
                );
            }
        }
        let timing_enabled = std::env::var("EXTEND_DISPLAY_TRANSPORT_TIMING").as_deref() == Ok("1");
        #[cfg(target_os = "macos")]
        let kernel_timestamping = timing_enabled
            && match &socket {
                DatagramSocket::Udp(udp) => match socket_timing::enable(udp) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("Display kernel timestamps unavailable: {error}");
                        false
                    }
                },
                DatagramSocket::Link(_) => false,
            };
        #[cfg(not(target_os = "macos"))]
        let kernel_timestamping = false;
        let kernel_arrival_requested =
            std::env::var("EXTEND_DISPLAY_KERNEL_ARRIVAL").as_deref() == Ok("1");
        #[cfg(target_os = "macos")]
        let kernel_arrival = kernel_arrival_requested.then(|| {
            kernel_arrival::Receiver::new(
                match &socket {
                    DatagramSocket::Udp(udp) => Some(udp),
                    DatagramSocket::Link(_) => None,
                },
                false,
            )
        });
        #[cfg(not(target_os = "macos"))]
        if kernel_arrival_requested {
            eprintln!("Display receive clock: requested=kernel-monotonic actual=userspace; kernel arrival experiment requires macOS");
        }
        let diagnostic = std::env::var_os("EXTEND_DISPLAY_METRICS").is_some()
            || std::env::var_os("EXTEND_DISPLAY_BENCHMARK_CONTEXT").is_some();
        metrics::reordering_configuration(reordering_timeout.as_millis() as usize);
        let mut rtc = RtcConfig::new()
            .clear_codecs()
            .enable_h264(codec == VideoCodec::H264)
            .enable_h265(codec == VideoCodec::Hevc)
            // Interactive display must recover instead of holding a missing
            // frame for the general video default of two seconds.
            .set_reordering_timeout_video(Some(reordering_timeout))
            .enable_bwe(Some(str0m::bwe::Bitrate::from(20_000_000)))
            // Statistics also feed the live connection quality shown to users.
            .set_stats_interval(Some(Duration::from_secs(if diagnostic { 5 } else { 2 })))
            .build(Instant::now());
        let ice_address =
            link_local_route.map_or(socket.local_addr()?, |route| route.local_candidate());
        rtc.add_local_candidate(Candidate::host(ice_address, "udp")?)
            .context("Invalid display candidate")?;
        let mut transport = Self {
            rtc,
            codec,
            cursor_position: None,
            cursor_shape: None,
            input_channel: None,
            text_input: false,
            shortcuts: false,
            input_role: None,
            socket,
            link_local_route,
            udp_peer: peer,
            usb_recovery: None,
            usb_input_retiring: false,
            pending_transmit: None,
            mid: None,
            pending: None,
            events: VecDeque::new(),
            next: Instant::now(),
            last_keyframe: None,
            awaiting_keyframe: false,
            capture_paused: false,
            lan_route: false,
            connected: false,
            paced_lan: std::env::var("EXTEND_DISPLAY_PACED_TRANSPORT").as_deref() == Ok("1"),
            timing: timing_enabled.then(|| TransportTiming::new(Instant::now())),
            kernel_timestamping,
            #[cfg(target_os = "macos")]
            kernel_arrival,
            diagnostics: diagnostic.then(|| (Instant::now(), [0; 7])),
        };
        transport.drain()?;
        Ok(transport)
    }

    /// Enable a bounded route-error grace after the caller verifies an actual
    /// macOS USB NCM interface and its authenticated signaling peer. Addresses
    /// alone never establish USB provenance. Packet links and generic UDP
    /// transports cannot enable this policy.
    pub fn enable_physical_usb_recovery(&mut self) -> Result<()> {
        ensure!(
            cfg!(target_os = "macos"),
            "USB route recovery requires macOS"
        );
        ensure!(
            matches!(self.socket, DatagramSocket::Udp(_)),
            "USB route recovery requires a physical UDP socket"
        );
        let local = self.socket.local_addr()?.ip();
        let peer = self
            .udp_peer
            .context("USB route recovery requires an authenticated UDP peer")?;
        let valid = |ip: IpAddr| {
            ip.is_ipv4() && !ip.is_unspecified() && !ip.is_loopback() && !ip.is_multicast()
        };
        ensure!(
            valid(local) && valid(peer) && local != peer,
            "Invalid USB route endpoints"
        );
        // Repeated configuration must not reset an active recovery deadline.
        if self.usb_recovery.is_none() {
            self.usb_recovery = Some(udp_recovery::Recovery::default());
            eprintln!("Display USB route recovery enabled: grace_ms=2000 retry_ms=50 control_fresh_ms=3000");
        }
        Ok(())
    }

    /// Only a validated authenticated Ping/Pong may provide this timestamp.
    /// Cancellation, lost trust, or a stopped control worker must publish None.
    pub fn update_usb_control_health(&mut self, authenticated: Option<Instant>) {
        if let Some(recovery) = &mut self.usb_recovery {
            recovery.update_control(authenticated);
        }
    }

    pub fn usb_recovery_active(&self) -> bool {
        self.usb_recovery
            .as_ref()
            .is_some_and(udp_recovery::Recovery::active)
    }

    /// Remember every outage even if it recovers between the caller's ticks.
    /// A new epoch retires input ownership; a successful retry supplies the
    /// start of the desktop's fixed post-recovery request quarantine.
    pub fn usb_recovery_checkpoint(&self) -> (u64, Option<Instant>) {
        self.usb_recovery
            .as_ref()
            .map_or((0, None), udp_recovery::Recovery::checkpoint)
    }

    /// Prefer accurate arrival times on a desktop-verified physical USB route.
    /// Generic UDP callers keep their existing clock. An explicit experiment
    /// setting (including "0") overrides this preference. Unsupported macOS
    /// sockets/timestamps fall back to userspace without ending the session.
    pub fn prefer_kernel_arrival(&mut self) {
        #[cfg(target_os = "macos")]
        self.prefer_kernel_arrival_if_unset(
            std::env::var_os("EXTEND_DISPLAY_KERNEL_ARRIVAL").is_none(),
        );
    }

    #[cfg(target_os = "macos")]
    fn prefer_kernel_arrival_if_unset(&mut self, unset: bool) {
        if unset && self.kernel_arrival.is_none() {
            if let DatagramSocket::Udp(socket) = &self.socket {
                self.kernel_arrival = Some(kernel_arrival::Receiver::new(Some(socket), true));
            }
        }
    }
    pub fn offer(&mut self) -> Result<String> {
        self.offer_with_features(DisplayFeatures::default())
    }
    pub fn offer_with_features(&mut self, features: DisplayFeatures) -> Result<String> {
        ensure!(
            self.mid.is_none() && self.pending.is_none(),
            "Display already offered"
        );
        self.input_role = features.input_enabled().then_some(input::Role::Source);
        self.text_input = features.text_enabled();
        self.shortcuts = features.shortcuts_enabled();
        let mut change = self.rtc.sdp_api();
        self.mid = Some(change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None));
        change.add_channel_with_config(ChannelConfig {
            label: "cursor-position-v1".into(),
            ordered: false,
            reliability: Reliability::MaxRetransmits { retransmits: 0 },
            ..Default::default()
        });
        change.add_channel("cursor-shape-v1".into());
        if features.input_enabled() {
            change.add_channel(crate::protocol::DISPLAY_INPUT.into());
        }
        let (offer, pending) = change.apply().context("Missing display offer")?;
        self.pending = Some(pending);
        self.drain()?;
        Ok(serde_json::to_string(&offer)?)
    }
    pub fn answer(&mut self, offer: &str) -> Result<String> {
        self.answer_with_features(offer, DisplayFeatures::default())
    }
    pub fn answer_with_features(
        &mut self,
        offer: &str,
        features: DisplayFeatures,
    ) -> Result<String> {
        ensure!(
            self.mid.is_none() && self.input_channel.is_none(),
            "Display already answered"
        );
        self.input_role = features.input_enabled().then_some(input::Role::Receiver);
        self.text_input = features.text_enabled();
        self.shortcuts = features.shortcuts_enabled();
        let answer = self
            .rtc
            .sdp_api()
            .accept_offer(serde_json::from_str::<SdpOffer>(offer)?)?;
        self.drain()?;
        Ok(serde_json::to_string(&answer)?)
    }
    pub fn accept_answer(&mut self, answer: &str) -> Result<()> {
        let pending = self.pending.take().context("Unexpected display answer")?;
        self.rtc
            .sdp_api()
            .accept_answer(pending, serde_json::from_str::<SdpAnswer>(answer)?)?;
        self.drain()
    }
    pub fn write_cursor(&mut self, bytes: &[u8]) -> Result<bool> {
        validate_cursor(bytes)?;
        // Keep only the caller's latest cursor slots during a route outage;
        // do not accumulate reliable SCTP shapes behind the pending datagram.
        if self.usb_recovery_active() {
            return Ok(false);
        }
        let id = if cursor_is_shape(bytes) {
            self.cursor_shape
        } else {
            self.cursor_position
        };
        let Some(id) = id else { return Ok(false) };
        let Some(mut channel) = self.rtc.channel(id) else {
            return Ok(false);
        };
        let accepted = channel.write(true, bytes)?;
        if accepted {
            metrics::cursor_event(
                if cursor_is_shape(bytes) {
                    "cursor_shape_transport_submitted"
                } else {
                    "cursor_position_transport_submitted"
                },
                bytes,
            );
        }
        self.drain()?;
        Ok(accepted)
    }
    /// Small ordered messages use the same DTLS association and native carrier
    /// as video. OS input authorization belongs to the source's active lease.
    pub fn shortcuts_negotiated(&self) -> bool {
        self.shortcuts
    }
    pub fn text_input_negotiated(&self) -> bool {
        self.text_input
    }
    pub fn input_negotiated(&self) -> bool {
        self.input_role.is_some()
    }
    pub fn write_input(&mut self, message: &input::Message) -> Result<bool> {
        let role = self
            .input_role
            .context("Display input was not negotiated")?;
        message.validate_sender(role)?;
        self.validate_text_input(message)?;
        let bytes = message.encode()?;
        let Some(id) = self.input_channel else {
            return Ok(false);
        };
        let Some(mut channel) = self.rtc.channel(id) else {
            return Ok(false);
        };
        if channel.buffered_amount().saturating_add(bytes.len()) > input::MAX_BUFFERED_BYTES {
            return Ok(false);
        }
        let accepted = channel.write(true, &bytes)?;
        self.drain()?;
        Ok(accepted)
    }
    fn validate_text_input(&self, message: &input::Message) -> Result<()> {
        if let input::Message::Granted {
            shortcut_policy, ..
        } = message
        {
            crate::input::ShortcutPolicy::validate_grant(*shortcut_policy, self.shortcuts)?;
        }
        if matches!(
            message,
            input::Message::Command {
                command: input::Command::Input {
                    event: crate::input::InputEvent::Text { .. },
                    ..
                },
                ..
            }
        ) {
            ensure!(self.text_input, "Committed text was not negotiated");
        }
        Ok(())
    }
    fn receive_input(&mut self, binary: bool, bytes: &[u8]) -> Result<()> {
        ensure!(binary, "Display input must be a binary envelope");
        let message = input::Message::decode(bytes)?;
        let remote = match self.input_role.context("Unnegotiated display input")? {
            input::Role::Source => input::Role::Receiver,
            input::Role::Receiver => input::Role::Source,
        };
        message.validate_sender(remote)?;
        self.validate_text_input(&message)?;
        // The retained outbound packet can block RTC output while receive
        // continues. Retire input through the first idle output boundary after
        // recovery; desktop additionally quarantines requests for their full
        // receiver-side intent TTL, including late SCTP retransmissions.
        if self.usb_input_retiring {
            return Ok(());
        }
        ensure!(
            self.events
                .iter()
                .filter(|e| matches!(e, DisplayEvent::Input(_)))
                .count()
                < input::MAX_PENDING_MESSAGES,
            "Display input receive queue is full"
        );
        self.events.push_back(DisplayEvent::Input(message));
        Ok(())
    }
    pub fn write(&mut self, frame: Frame) -> Result<bool> {
        self.refresh_send_queue()?;
        if self.usb_recovery_active() {
            return Ok(false);
        }
        let mid = self.mid.context("Missing video track")?;
        // Capture supplies the frame cadence on direct LAN routes. Send each
        // encoded frame promptly; keep encryption, feedback and adaptive bitrate.
        // Public-address routes retain the transport's packet pacing.
        if let Some(stream) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
            stream.set_unpaced(self.lan_route && !self.paced_lan);
        }

        let (queued, stale, age_ms) = self
            .rtc
            .direct_api()
            .stream_tx_by_mid(mid, None)
            .and_then(|stream| stream.queue_info())
            .map(|queue| {
                (
                    queue.packet_count() > 0,
                    queue
                        .first_unsent()
                        .is_some_and(|first| first.elapsed() > Duration::from_millis(250)),
                    queue
                        .first_unsent()
                        .map(|t| t.elapsed().as_millis() as u64)
                        .unwrap_or(0),
                )
            })
            .unwrap_or((false, false, 0));
        if let Some((_, counts)) = &mut self.diagnostics {
            counts[6] = counts[6].max(age_ms);
        }
        if stale && !self.lan_route && !self.awaiting_keyframe {
            if let Some((_, counts)) = &mut self.diagnostics {
                counts[5] += 1;
            }
            self.awaiting_keyframe = true;
            self.events.push_back(DisplayEvent::Keyframe);
        }
        if self.awaiting_keyframe {
            if queued || !frame.keyframe {
                // A keyframe requested while old packets were draining may also
                // have been dropped. Ask again once the send queue is clear.
                if !queued
                    && self
                        .last_keyframe
                        .is_none_or(|last| last.elapsed() >= Duration::from_millis(300))
                {
                    self.last_keyframe = Some(Instant::now());
                    self.events.push_back(DisplayEvent::Keyframe);
                }
                if let Some((_, counts)) = &mut self.diagnostics {
                    counts[4] += 1;
                }
                return Ok(false);
            }
            self.awaiting_keyframe = false;
        }
        if let Some((_, counts)) = &mut self.diagnostics {
            counts[0] += 1;
        }
        ensure!(
            frame.bytes.len() <= 16 * 1024 * 1024,
            "Video frame too large"
        );
        let writer = self
            .rtc
            .writer(self.mid.context("Missing video track")?)
            .context("Missing video writer")?;
        let pt = writer
            .payload_params()
            .find(|p| {
                p.spec().codec
                    == match self.codec {
                        VideoCodec::H264 => Codec::H264,
                        VideoCodec::Hevc => Codec::H265,
                    }
            })
            .map(|p| p.pt())
            .context("Peer does not support selected video codec")?;
        writer.write(
            pt,
            Instant::now(),
            MediaTime::from(Duration::from_nanos(frame.timestamp)),
            frame.bytes,
        )?;
        // Packetization creates StreamTx lazily on the next timeout, rather
        // than during Writer::write. Service that timeout before looking up
        // the stream so the first keyframe gets the same route policy.
        self.refresh_send_queue()?;
        if let Some(stream) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
            stream.set_unpaced(self.lan_route && !self.paced_lan);
        }
        self.refresh_send_queue()?;
        Ok(true)
    }
    /// Backpressure before encoding preserves the codec reference chain. Once
    /// two frames of pacing delay accumulate, let queued media drain before
    /// capturing another frame; encoded-frame rejection is a last resort.
    pub fn pause_capture(&mut self, fps: u32) -> Result<bool> {
        self.refresh_send_queue()?;
        if self.usb_recovery_active() {
            self.capture_paused = true;
            return Ok(true);
        }
        // Packet pacing does not hold LAN media frames. Its aggregate snapshot
        // also includes probes and retransmissions, which must not pause capture.
        if self.lan_route {
            self.capture_paused = false;
            return Ok(false);
        }
        let budget = Duration::from_secs_f64(2.0 / fps.max(1) as f64);
        let queue = self.mid.and_then(|mid| {
            self.rtc
                .direct_api()
                .stream_tx_by_mid(mid, None)
                .and_then(|stream| {
                    stream.queue_info().map(|q| {
                        (
                            q.packet_count() > 0,
                            q.first_unsent().map(|t| t.elapsed()).unwrap_or_default(),
                        )
                    })
                })
        });
        match queue {
            Some((true, age)) if age >= budget => self.capture_paused = true,
            Some((false, _)) | None => self.capture_paused = false,
            _ => {}
        }
        Ok(self.capture_paused)
    }
    fn refresh_send_queue(&mut self) -> Result<()> {
        // queue_info is sampled before packets are drained, so even a recent
        // snapshot can describe packets already sent. Advance the RTC before
        // every admission/backpressure decision to resample the actual queue.
        // str0m permits timeout input before its next scheduled deadline.
        self.rtc.handle_input(Input::Timeout(Instant::now()))?;
        self.drain()
    }
    pub fn request_keyframe(&mut self) -> Result<()> {
        if self
            .last_keyframe
            .is_some_and(|last| last.elapsed() < Duration::from_millis(300))
        {
            return Ok(());
        }
        self.last_keyframe = Some(Instant::now());
        if let Some(mid) = self.mid {
            if let Some(mut writer) = self.rtc.writer(mid) {
                writer.request_keyframe(None, KeyframeRequestKind::Pli)?;
            }
        }
        self.drain()
    }
    pub fn set_target_bitrate(&mut self, bitrate: u32) -> Result<()> {
        self.rtc
            .bwe()
            .set_desired_bitrate(str0m::bwe::Bitrate::from(bitrate as u64));
        self.drain()
    }
    fn drain(&mut self) -> Result<()> {
        if let Some((destination, bytes)) = self.pending_transmit.take() {
            if !self.transmit(&bytes, destination)? {
                self.pending_transmit = Some((destination, bytes));
                return Ok(());
            }
        }
        let mut immediate_timeouts = 0;
        loop {
            match self.rtc.poll_output()? {
                Output::Timeout(next) => {
                    self.next = next;
                    // The pacer requires a timeout between packet emissions,
                    // including unpaced LAN packets. Returning for an already
                    // due timeout would add the app loop's 1 ms sleep to every
                    // packet and throttle a fragmented frame below its bitrate.
                    // Bound the batch so incoming packets and cancellation still
                    // get serviced; future pacing deadlines remain untouched.
                    let now = Instant::now();
                    if next <= now && immediate_timeouts < 64 {
                        immediate_timeouts += 1;
                        self.rtc.handle_input(Input::Timeout(now))?;
                        continue;
                    }
                    if next > now && !self.usb_recovery_active() {
                        self.usb_input_retiring = false;
                    }
                    return Ok(());
                }
                Output::Transmit(packet) => {
                    // Logical native endpoints also use host candidates, but
                    // must not inherit optimizations for an actual LAN socket.
                    self.lan_route = matches!(self.socket, DatagramSocket::Udp(_))
                        && local_address(self.socket.local_addr()?.ip())
                        && local_address(self.physical_destination(packet.destination)?.ip());
                    if !self.transmit(&packet.contents, packet.destination)? {
                        // Retain at most one encrypted packet. The carrier's
                        // bounded queue can resume without dropping handshake
                        // traffic or allocating an unbounded second queue.
                        self.pending_transmit =
                            Some((packet.destination, packet.contents.to_vec()));
                        return Ok(());
                    }
                }
                Output::Event(Event::ChannelOpen(id, label)) => match label.as_str() {
                    "cursor-position-v1" => self.cursor_position = Some(id),
                    "cursor-shape-v1" => self.cursor_shape = Some(id),
                    crate::protocol::DISPLAY_INPUT => {
                        ensure!(
                            self.input_role.is_some() && self.input_channel.is_none(),
                            "Unnegotiated or duplicate display input channel"
                        );
                        let channel = self
                            .rtc
                            .channel(id)
                            .context("Missing display input channel")?;
                        let config = channel
                            .config()
                            .context("Missing display input reliability")?;
                        ensure!(
                            config.ordered
                                && config.reliability == Reliability::Reliable
                                && config.negotiated.is_none(),
                            "Display input must be ordered and reliable"
                        );
                        self.input_channel = Some(id);
                    }
                    _ => anyhow::bail!("Unexpected display data channel"),
                },
                Output::Event(Event::ChannelData(data)) => {
                    if Some(data.id) == self.input_channel {
                        self.receive_input(data.binary, &data.data)?;
                        continue;
                    }
                    ensure!(
                        Some(data.id) == self.cursor_position || Some(data.id) == self.cursor_shape,
                        "Unexpected cursor channel"
                    );
                    validate_cursor(&data.data)?;
                    // Positions are latest-state messages; stale queued samples have no value.
                    let shape = Some(data.id) == self.cursor_shape;
                    metrics::cursor_event(
                        if shape {
                            "cursor_shape_transport_received"
                        } else {
                            "cursor_position_transport_received"
                        },
                        &data.data,
                    );
                    self.events.retain(|event| !matches!(event, DisplayEvent::Cursor(old) if cursor_is_shape(old) == shape));
                    self.events.push_back(DisplayEvent::Cursor(data.data));
                }
                Output::Event(Event::ChannelClose(id)) if Some(id) == self.input_channel => {
                    self.input_channel = None;
                    self.input_role = None;
                    self.events.retain(|e| !matches!(e, DisplayEvent::Input(_)));
                    self.events.push_back(DisplayEvent::InputClosed);
                }
                Output::Event(Event::Connected) => {
                    self.connected = true;
                    self.events.push_back(DisplayEvent::Connected);
                }
                Output::Event(Event::PeerStats(stats))
                    if self.diagnostics.is_some() && self.connected =>
                {
                    if let Some(pair) = stats.selected_candidate_pair {
                        // Selected ICE tuple, not an SDP candidate or a guessed
                        // destination. Benchmark reports hash addresses after
                        // matching this tuple to the owning OS socket.
                        eprintln!(
                            "Display route {}",
                            serde_json::json!({
                                "wall_ns":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
                                "protocol":format!("{:?}",pair.protocol).to_lowercase(),
                                "local":pair.local.addr.to_string(),"remote":pair.remote.addr.to_string(),
                                "responses_received":pair.responses_received,"paced_lan":self.paced_lan,
                                "socket_local":self.socket.local_addr()?.to_string(),
                                "socket_remote":self.physical_destination(pair.remote.addr)?.to_string()
                            })
                        );
                    }
                }
                Output::Event(Event::MediaAdded(media)) => {
                    ensure!(
                        media.kind == MediaKind::Video && self.mid.is_none(),
                        "Unexpected display track"
                    );
                    self.mid = Some(media.mid);
                }
                Output::Event(Event::MediaData(frame)) => {
                    if let Some(timing) = &mut self.timing {
                        timing.assembled(
                            *frame.pt,
                            frame.time.numer() as u32,
                            frame.network_time,
                            Instant::now(),
                            (**frame.seq_range.start(), **frame.seq_range.end()),
                            frame.contiguous,
                        );
                    }
                    if let Some((_, counts)) = &mut self.diagnostics {
                        counts[1] += 1;
                    }
                    ensure!(
                        frame.params.spec().codec
                            == match self.codec {
                                VideoCodec::H264 => Codec::H264,
                                VideoCodec::Hevc => Codec::H265,
                            },
                        "Unexpected video codec"
                    );
                    ensure!(
                        frame.data.len() <= 16 * 1024 * 1024,
                        "Video frame too large"
                    );
                    let keyframe = match frame.codec_extra {
                        str0m::format::CodecExtra::H264(extra) => extra.is_keyframe,
                        str0m::format::CodecExtra::H265(extra) => extra.is_keyframe,
                        _ => false,
                    };
                    if !frame.contiguous && !keyframe {
                        if let Some((_, counts)) = &mut self.diagnostics {
                            counts[2] += 1;
                        }
                        self.events.push_back(DisplayEvent::Keyframe);
                        continue;
                    }
                    self.events.push_back(DisplayEvent::Frame(Frame {
                        timestamp: (frame.time.as_seconds() * 1e9) as u64,
                        keyframe,
                        bytes: frame.data.to_vec(),
                    }));
                }
                Output::Event(Event::KeyframeRequest(_)) => {
                    if let Some((_, counts)) = &mut self.diagnostics {
                        counts[3] += 1;
                    }
                    self.events.push_back(DisplayEvent::Keyframe)
                }
                Output::Event(Event::MediaEgressStats(stats)) => {
                    if self.diagnostics.is_some() {
                        eprintln!(
                            "Display RTP sent: packets={} bytes={} nacks={} plis={} loss={:?} rtt_ms={:?} remote_lost={:?} lan={}",
                            stats.packets, stats.bytes, stats.nacks, stats.plis, stats.loss,
                            stats.rtt.map(|rtt| rtt.as_millis()),
                            stats.remote.map(|remote| remote.packets_lost), self.lan_route
                        );
                    }
                    self.events.push_back(DisplayEvent::Stats {
                        rtt: stats.rtt,
                        loss: stats.loss,
                    });
                }
                Output::Event(Event::MediaIngressStats(stats)) => {
                    if self.diagnostics.is_some() {
                        eprintln!(
                            "Display RTP received: packets={} bytes={} nacks={} plis={} loss={:?} jitter_ticks={}",
                            stats.packets, stats.bytes, stats.nacks, stats.plis, stats.loss, stats.jitter
                        );
                    }
                    self.events.push_back(DisplayEvent::Stats {
                        rtt: stats.rtt,
                        loss: stats.loss,
                    });
                }
                Output::Event(Event::EgressBitrateEstimate(estimate)) => {
                    let bitrate = match estimate {
                        str0m::bwe::BweKind::Twcc {
                            estimate,
                            can_probe: true,
                        }
                        | str0m::bwe::BweKind::Remb { estimate, .. } => Some(estimate),
                        _ => None,
                    };
                    if let Some(bitrate) = bitrate {
                        self.events
                            .push_back(DisplayEvent::Bitrate(bitrate.as_u64()));
                    }
                }
                _ => {}
            }
        }
    }
    fn transmit(&mut self, bytes: &[u8], destination: SocketAddr) -> Result<bool> {
        let destination = self.physical_destination(destination)?;
        let now = Instant::now();
        if let Some(recovery) = &mut self.usb_recovery {
            match recovery.ready(now) {
                Ok(false) => return Ok(false),
                Ok(true) => {}
                Err(failure) => return Err(recovery.error()).context(failure.message()),
            }
        }
        let sent = self.socket.send_to(bytes, destination);
        // A scheduling pause around the syscall must not shorten the desktop's
        // post-recovery intent quarantine or bypass the fixed grace deadline.
        let completed = Instant::now();
        if let Some(recovery) = &mut self.usb_recovery {
            recovery.attempted(completed);
        }
        self.transmitted(bytes, destination, completed, sent)
    }

    fn transmitted(
        &mut self,
        bytes: &[u8],
        destination: SocketAddr,
        now: Instant,
        sent: std::io::Result<usize>,
    ) -> Result<bool> {
        if let Some(recovery) = &self.usb_recovery {
            if let Err(failure) = recovery.validate(now) {
                return Err(recovery.error()).context(failure.message());
            }
        }
        match sent {
            Ok(_) => {
                if self.udp_peer == Some(destination.ip())
                    && self
                        .usb_recovery
                        .as_mut()
                        .is_some_and(|recovery| recovery.succeeded(now))
                {
                    eprintln!("Display USB route recovered; requesting a fresh keyframe");
                    if !self
                        .events
                        .iter()
                        .any(|event| matches!(event, DisplayEvent::Keyframe))
                    {
                        self.events.push_back(DisplayEvent::Keyframe);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => {
                if self.connected
                    && self.udp_peer == Some(destination.ip())
                    && udp_recovery::route_error(&error)
                {
                    if let Some(recovery) = &mut self.usb_recovery {
                        match recovery.failed(&error, now) {
                            Ok(first) => {
                                if first {
                                    self.awaiting_keyframe = true;
                                    self.usb_input_retiring = true;
                                    self.events
                                        .retain(|event| !matches!(event, DisplayEvent::Input(_)));
                                    eprintln!("Display USB route temporarily unreachable: errno={} grace_ms=2000 retry_ms=50", error.raw_os_error().unwrap());
                                }
                                return Ok(false);
                            }
                            Err(failure) => return Err(error).context(failure.message()),
                        }
                    }
                }
                let local = self.socket.local_addr().map_or_else(
                    |_| "an unbound socket".into(),
                    |address| address.to_string(),
                );
                return Err(crate::wire::TransportFailure(error)).with_context(|| {
                    format!(
                        "Display datagram send from {local} to {destination} ({} bytes)",
                        bytes.len()
                    )
                });
            }
        }
        metrics::datagram_event("dtls_datagram_sent", bytes);
        if let Some(timing) = &mut self.timing {
            timing.send.observe(bytes, Instant::now());
        }
        Ok(true)
    }
    fn physical_destination(&self, destination: SocketAddr) -> Result<SocketAddr> {
        match self.link_local_route {
            Some(route) => route.physical_destination(destination),
            None => Ok(destination),
        }
    }
    pub fn poll(&mut self) -> Result<Option<DisplayEvent>> {
        // Check even between scheduled send retries. A dead control peer or an
        // expired grace cannot remain alive by filling the receive/event queue.
        if let Some(recovery) = &self.usb_recovery {
            if let Err(failure) = recovery.validate(Instant::now()) {
                return Err(recovery.error()).context(failure.message());
            }
        }
        // Carrier readiness is independent of RTC's next timeout. In
        // particular, queued DTLS handshake packets must resume immediately
        // after a bulk worker frees space, even with no incoming traffic.
        if self.pending_transmit.is_some() {
            self.drain()?;
        }
        // Pending media/control events must not starve UDP receive or pacing.
        // A packet burst can produce several events before the caller consumes
        // them; keep servicing the socket on every tick while draining that queue.
        // Drain a bounded burst each tick. One packet per millisecond caps a
        // desktop stream near 10 Mbps and turns large frames into a socket backlog.
        let mut bytes = [0u8; 2048];
        for _ in 0..64 {
            #[cfg(target_os = "macos")]
            let received = if self.kernel_timestamping
                || self
                    .kernel_arrival
                    .as_ref()
                    .is_some_and(|arrival| arrival.active())
            {
                self.socket.receive_timestamped(&mut bytes)
            } else {
                self.socket
                    .recv_from(&mut bytes)
                    .map(|(size, source)| (size, source, socket_timing::Timestamps::default()))
            }
            .map(|(size, source, stamps)| (size, source, stamps.wall_ns, stamps.monotonic_ticks));
            #[cfg(not(target_os = "macos"))]
            let received = self
                .socket
                .recv_from(&mut bytes)
                .map(|(size, source)| (size, source, None::<u64>, None::<u64>));
            match received {
                Ok((size, source, kernel_wall_ns, _kernel_monotonic_ticks)) => {
                    let source = match self.link_local_route {
                        Some(route) => {
                            let Some(source) = route.received_source(source) else {
                                continue;
                            };
                            source
                        }
                        None => source,
                    };
                    metrics::datagram_event("dtls_datagram_received", &bytes[..size]);
                    if let Some(timing) = &mut self.timing {
                        timing.received(&bytes[..size], Instant::now());
                        timing.kernel_observation(&bytes[..size], kernel_wall_ns);
                    }
                    let Ok(contents) = (&bytes[..size]).try_into() else {
                        continue;
                    };
                    #[cfg(target_os = "macos")]
                    let (receive_time, kernel_applied) = self.kernel_arrival.as_ref().map_or_else(
                        || (Instant::now(), false),
                        |arrival| {
                            arrival.receive_time(socket_timing::Timestamps {
                                wall_ns: kernel_wall_ns,
                                monotonic_ticks: _kernel_monotonic_ticks,
                            })
                        },
                    );
                    #[cfg(not(target_os = "macos"))]
                    let receive_time = Instant::now();
                    // RTC explicitly accepts an older receive Instant (e.g.
                    // queued UDP demux) without moving its timeout clock back.
                    let input = Input::Receive(
                        receive_time,
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination: self
                                .link_local_route
                                .map_or(self.socket.local_addr()?, |route| route.local_candidate()),
                            contents,
                        },
                    );
                    if self.rtc.accepts(&input) {
                        self.rtc.handle_input(input)?;
                        #[cfg(target_os = "macos")]
                        if let Some(arrival) = &mut self.kernel_arrival {
                            arrival.record(kernel_applied, Instant::now());
                        }
                        self.drain()?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("Display datagram receive on {:?}", self.socket.local_addr())
                    })
                }
            }
        }
        if Instant::now() >= self.next {
            self.rtc.handle_input(Input::Timeout(Instant::now()))?;
            self.drain()?;
        }
        if let Some((since, counts)) = &mut self.diagnostics {
            if since.elapsed() >= Duration::from_secs(5) {
                eprintln!(
                    "Display transport /5s: sent={} assembled={} discontinuous={} pli={} admission_dropped={} recovery={} max_queue_ms={}",
                    counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]
                );
                *since = Instant::now();
                *counts = [0; 7];
            }
        }
        if let Some(timing) = &mut self.timing {
            timing.maybe_log(Instant::now());
        }
        Ok(self.events.pop_front())
    }
}

/// Whether `bytes` holds 64 consecutive copies of the plaintext filler that
/// encryption tests send, in one linear pass per packet.
#[cfg(test)]
fn contains_test_plaintext(bytes: &[u8]) -> bool {
    let mut run = 0;
    bytes.iter().any(|&byte| {
        run = if byte == 0x88 { run + 1 } else { 0 };
        run >= 64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recovery_transport() -> Transport {
        let mut transport = Transport::with_udp_peer(
            "127.0.0.1".parse().unwrap(),
            "127.0.0.2".parse().unwrap(),
            VideoCodec::H264,
        )
        .unwrap();
        // Unit fault injection bypasses only the external NCM eligibility
        // boundary. The public API rejects these loopback test endpoints.
        transport.usb_recovery = Some(udp_recovery::Recovery::default());
        transport.connected = true;
        transport
    }

    #[test]
    fn usb_recovery_requires_explicit_eligibility_established_rtc_and_pinned_peer() {
        let peer: SocketAddr = "127.0.0.2:48180".parse().unwrap();
        let now = Instant::now();
        let mut transport = recovery_transport();
        transport.usb_recovery = None;
        assert!(transport.enable_physical_usb_recovery().is_err());
        let unreachable = || Err(std::io::Error::from_raw_os_error(65));
        assert!(transport
            .transmitted(&[20], peer, now, unreachable())
            .is_err());
        transport.usb_recovery = Some(udp_recovery::Recovery::default());
        transport.update_usb_control_health(Some(now));
        transport.connected = false;
        assert!(transport
            .transmitted(&[20], peer, now, unreachable())
            .is_err());
        assert!(!transport.usb_recovery_active());
        transport.connected = true;
        assert!(transport
            .transmitted(
                &[20],
                "127.0.0.3:48180".parse().unwrap(),
                now,
                unreachable()
            )
            .is_err());
        assert!(!transport.usb_recovery_active());
        assert!(!transport
            .transmitted(&[20], peer, now, unreachable())
            .unwrap());
        assert!(transport.usb_recovery_active());
        assert!(transport.awaiting_keyframe);
        assert!(transport
            .transmitted(&[20], peer, now, Err(std::io::Error::from_raw_os_error(22)))
            .is_err());
        assert!(transport.usb_recovery_active());
    }

    #[test]
    fn usb_recovery_success_queues_one_keyframe_and_capture_waits_for_it() {
        let peer: SocketAddr = "127.0.0.2:48180".parse().unwrap();
        let now = Instant::now();
        let mut transport = recovery_transport();
        transport.update_usb_control_health(Some(now));
        transport
            .transmitted(&[20], peer, now, Err(std::io::Error::from_raw_os_error(65)))
            .unwrap();
        transport.lan_route = true;
        assert!(
            transport.pause_capture(60).unwrap(),
            "LAN fast path must not bypass an active outage"
        );
        assert!(!transport
            .write(Frame {
                timestamp: 1,
                keyframe: false,
                bytes: vec![0x88; 16]
            })
            .unwrap());
        assert!(!transport
            .transmitted(
                &[20],
                peer,
                now + Duration::from_millis(49),
                Err(std::io::Error::from_raw_os_error(51))
            )
            .unwrap());
        transport
            .transmitted(&[20], peer, now + Duration::from_millis(100), Ok(1))
            .unwrap();
        transport
            .transmitted(&[20], peer, now + Duration::from_millis(101), Ok(1))
            .unwrap();
        assert!(!transport.usb_recovery_active());
        assert!(
            transport.awaiting_keyframe,
            "Dropped frames require a new encoder reference"
        );
        assert_eq!(
            transport
                .events
                .iter()
                .filter(|event| matches!(event, DisplayEvent::Keyframe))
                .count(),
            1
        );
        assert!(!transport.pause_capture(60).unwrap());
    }

    #[test]
    fn usb_recovery_retires_preexisting_and_rtc_backlogged_input() {
        let peer: SocketAddr = "127.0.0.2:48180".parse().unwrap();
        let now = Instant::now();
        let mut transport = recovery_transport();
        transport.input_role = Some(input::Role::Source);
        let request = input::Message::Request { request: 7 };
        transport
            .receive_input(true, &request.encode().unwrap())
            .unwrap();
        assert_eq!(transport.events.len(), 1);
        transport.update_usb_control_health(Some(now));
        transport
            .transmitted(&[20], peer, now, Err(std::io::Error::from_raw_os_error(65)))
            .unwrap();
        assert!(!transport
            .events
            .iter()
            .any(|event| matches!(event, DisplayEvent::Input(_))));
        transport
            .receive_input(true, &request.encode().unwrap())
            .unwrap();
        assert!(!transport
            .events
            .iter()
            .any(|event| matches!(event, DisplayEvent::Input(_))));
        transport
            .transmitted(&[20], peer, now + Duration::from_millis(50), Ok(1))
            .unwrap();
        assert!(!transport.usb_recovery_active());
        transport
            .receive_input(true, &request.encode().unwrap())
            .unwrap();
        assert!(
            !transport
                .events
                .iter()
                .any(|event| matches!(event, DisplayEvent::Input(_))),
            "Successful send alone must not release old RTC input"
        );
        transport.drain().unwrap();
        assert!(!transport.usb_input_retiring);
        // Late retransmits past this boundary are subject to the desktop's
        // request quarantine and the receiver's original pre-grant TTL.
        transport
            .receive_input(true, &request.encode().unwrap())
            .unwrap();
        assert!(transport
            .events
            .iter()
            .any(|event| matches!(event, DisplayEvent::Input(_))));
    }

    #[test]
    fn usb_recovery_success_after_fixed_deadline_remains_fatal() {
        let peer: SocketAddr = "127.0.0.2:48180".parse().unwrap();
        let now = Instant::now();
        let mut transport = recovery_transport();
        transport.update_usb_control_health(Some(now));
        transport
            .transmitted(&[20], peer, now, Err(std::io::Error::from_raw_os_error(65)))
            .unwrap();
        let expired = now + udp_recovery::GRACE;
        transport.update_usb_control_health(Some(expired));
        let error = transport
            .transmitted(&[20], peer, expired, Ok(1))
            .unwrap_err();
        assert!(format!("{error:#}").contains("fixed USB route recovery grace expired"));
        assert_eq!(transport.usb_recovery_checkpoint(), (1, None));
        assert!(transport.usb_recovery_active());
    }

    struct ReceiveCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl DatagramLink for ReceiveCounter {
        fn local_addr(&self) -> std::io::Result<SocketAddr> {
            Ok("192.0.2.1:48180".parse().unwrap())
        }
        fn send_to(&self, _: &[u8], _: SocketAddr) -> std::io::Result<usize> {
            panic!("Recovery must wait for the scheduled retry")
        }
        fn recv_from(&self, _: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(std::io::ErrorKind::WouldBlock.into())
        }
    }

    #[test]
    fn usb_recovery_wait_keeps_receiving_events_and_exactly_one_pending_packet() {
        let received = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut transport = Transport::with_packet_link(
            VideoCodec::H264,
            Box::new(ReceiveCounter(received.clone())),
        )
        .unwrap();
        assert!(
            transport.enable_physical_usb_recovery().is_err(),
            "Packet links must never enable physical USB recovery"
        );
        // Fault adapter tests poll fairness independently of OS interfaces.
        transport.usb_recovery = Some(udp_recovery::Recovery::default());
        let now = Instant::now();
        transport.update_usb_control_health(Some(now));
        transport
            .usb_recovery
            .as_mut()
            .unwrap()
            .failed(&std::io::Error::from_raw_os_error(65), now)
            .unwrap();
        let pending = ("192.0.2.2:48180".parse().unwrap(), vec![20, 1, 2]);
        transport.pending_transmit = Some(pending.clone());
        transport.events.push_back(DisplayEvent::Keyframe);
        assert!(matches!(
            transport.poll().unwrap(),
            Some(DisplayEvent::Keyframe)
        ));
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(transport.pending_transmit, Some(pending));
        transport.update_usb_control_health(None);
        let error = transport
            .poll()
            .err()
            .expect("Dead control must end recovery");
        assert!(format!("{error:#}").contains("authenticated control heartbeat"));
        assert_eq!(
            received.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "Dead control is rejected before more socket work"
        );
    }

    #[test]
    fn shortcut_grant_requires_negotiation_and_crosses_actual_encrypted_display_channel() {
        for negotiated in [false, true] {
            let features = if negotiated {
                DisplayFeatures::shortcut_input_for_test()
            } else {
                DisplayFeatures::input_for_test()
            };
            let (mut source, mut receiver) = input_pair(features);
            assert_eq!(source.shortcuts_negotiated(), negotiated);
            assert_eq!(receiver.shortcuts_negotiated(), negotiated);
            let message = input::Message::Granted {
                request: 1,
                binding: input::Binding::new(1, 0),
                text_supported: false,
                shortcut_policy: Some(crate::input::ShortcutPolicy::Command),
                geometry: input::Geometry {
                    content: input::Rect {
                        x: 0.,
                        y: 0.,
                        width: 1.,
                        height: 1.,
                    },
                    canvas: input::Size {
                        width: 1000,
                        height: 800,
                    },
                    raster: input::Size {
                        width: 1000,
                        height: 800,
                    },
                    source_size: input::Size {
                        width: 1000,
                        height: 800,
                    },
                },
            };
            if !negotiated {
                assert!(source.write_input(&message).is_err());
                assert!(receiver
                    .receive_input(true, &message.encode().unwrap())
                    .is_err());
                assert!(receiver
                    .events
                    .iter()
                    .all(|e| !matches!(e, DisplayEvent::Input(_))));
                continue;
            }
            assert!(source.write_input(&message).unwrap());
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(DisplayEvent::Input(delivered)) = receiver.poll().unwrap() {
                    assert_eq!(delivered, message);
                    break;
                }
                source.poll().unwrap();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            let mut invalid = serde_json::to_value(&message).unwrap();
            invalid["shortcut_policy"] = "meta".into();
            assert!(receiver
                .receive_input(true, &serde_json::to_vec(&invalid).unwrap())
                .is_err());
        }
    }
    #[test]
    fn unnegotiated_text_never_enters_display_channel() {
        let (mut source, mut receiver) = input_pair(DisplayFeatures::input_for_test());
        let message = input::Message::Command {
            binding: input::Binding::new(1, 0),
            sequence: 1,
            command: input::Command::Input {
                point: None,
                event: crate::input::InputEvent::Text {
                    value: "é🙂".into(),
                },
            },
        };
        assert!(receiver.write_input(&message).is_err());
        assert!(source
            .receive_input(true, &message.encode().unwrap())
            .is_err());
        assert!(source
            .events
            .iter()
            .all(|e| !matches!(e, DisplayEvent::Input(_))));
        source.text_input = true;
        receiver.text_input = true;
        assert!(receiver.write_input(&message).unwrap());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(DisplayEvent::Input(delivered)) = source.poll().unwrap() {
                assert_eq!(delivered, message);
                break;
            }
            receiver.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn input_pair(features: DisplayFeatures) -> (Transport, Transport) {
        let ip = "127.0.0.1".parse().unwrap();
        let mut source = Transport::new(ip).unwrap();
        let mut receiver = Transport::new(ip).unwrap();
        let offer = source.offer_with_features(features).unwrap();
        let answer = receiver.answer_with_features(&offer, features).unwrap();
        source.accept_answer(&answer).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            source.poll().unwrap();
            receiver.poll().unwrap();
            if source.connected
                && receiver.connected
                && source.cursor_shape.is_some()
                && receiver.cursor_shape.is_some()
                && (!features.input_enabled()
                    || (source.input_channel.is_some() && receiver.input_channel.is_some()))
            {
                return (source, receiver);
            }
            assert!(Instant::now() < deadline, "Input transport setup timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn display_input_is_optional_and_legacy_cursor_channels_are_unchanged() {
        let (mut source, mut receiver) = input_pair(DisplayFeatures::default());
        assert!(source.input_channel.is_none() && receiver.input_channel.is_none());
        assert!(source.input_role.is_none() && receiver.input_role.is_none());
        assert!(receiver
            .write_input(&input::Message::Request { request: 1 })
            .is_err());
        let position = br#"{"revision":1,"sequence":1,"x":0.5,"y":0.2,"visible":true}"#;
        assert!(source.write_cursor(position).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            source.poll().unwrap();
            if let Some(DisplayEvent::Cursor(bytes)) = receiver.poll().unwrap() {
                assert_eq!(bytes, position);
                break;
            }
            assert!(Instant::now() < deadline, "Legacy cursor timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn display_input_crosses_encrypted_transport_in_both_directions_without_video() {
        let (mut source, mut receiver) = input_pair(DisplayFeatures::input_for_test());
        let id = source.input_channel.unwrap();
        let channel = source.rtc.channel(id).unwrap();
        assert!(channel.config().unwrap().ordered);
        assert_eq!(channel.config().unwrap().reliability, Reliability::Reliable);
        let request = input::Message::Request { request: 7 };
        assert!(source.write_input(&request).is_err());
        assert!(receiver.write_input(&request).unwrap());
        let binding = input::Binding::new(4, 2);
        let applied = input::Message::Applied {
            binding,
            sequence: 1,
        };
        assert!(source.write_input(&applied).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = (false, false);
        loop {
            if let Some(DisplayEvent::Input(message)) = source.poll().unwrap() {
                assert_eq!(message, request);
                received.0 = true;
            }
            if let Some(DisplayEvent::Input(message)) = receiver.poll().unwrap() {
                assert_eq!(message, applied);
                received.1 = true;
            }
            if received == (true, true) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Encrypted input transport timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn display_input_rejects_malformed_and_wrong_direction_encrypted_messages() {
        for bytes in [
            br#"{"type":"request","request":1,"display_id":1}"#.to_vec(),
            input::Message::Applied {
                binding: input::Binding::new(0, 0),
                sequence: 1,
            }
            .encode()
            .unwrap(),
            vec![b' '; input::MAX_MESSAGE_BYTES + 1],
        ] {
            let (mut source, mut receiver) = input_pair(DisplayFeatures::input_for_test());
            let id = receiver.input_channel.unwrap();
            assert!(receiver
                .rtc
                .channel(id)
                .unwrap()
                .write(true, &bytes)
                .unwrap());
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                receiver.poll().unwrap();
                if source.poll().is_err() {
                    break;
                }
                assert!(Instant::now() < deadline, "Invalid input was not rejected");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    #[test]
    fn display_input_rejects_channels_without_authenticated_capability() {
        let ip = "127.0.0.1".parse().unwrap();
        let mut source = Transport::new(ip).unwrap();
        let mut receiver = Transport::new(ip).unwrap();
        let offer = source
            .offer_with_features(DisplayFeatures::input_for_test())
            .unwrap();
        let answer = receiver.answer(&offer).unwrap();
        source.accept_answer(&answer).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            source.poll().unwrap();
            if receiver.poll().is_err() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Unnegotiated input channel was accepted"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn display_input_receive_queue_is_bounded_and_requires_binary_envelopes() {
        let mut source = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
        source.input_role = Some(input::Role::Source);
        let bytes = input::Message::Request { request: 1 }.encode().unwrap();
        assert!(source.receive_input(false, &bytes).is_err());
        for _ in 0..input::MAX_PENDING_MESSAGES {
            source.receive_input(true, &bytes).unwrap();
        }
        assert!(source.receive_input(true, &bytes).is_err());
        assert_eq!(source.events.len(), input::MAX_PENDING_MESSAGES);
        source.events.pop_front();
        source.receive_input(true, &bytes).unwrap();
    }
    #[test]
    fn display_input_channel_close_notifies_adapter_and_keeps_cursor_stream_alive() {
        let (mut source, mut receiver) = input_pair(DisplayFeatures::input_for_test());
        receiver
            .rtc
            .direct_api()
            .close_data_channel(receiver.input_channel.unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            receiver.poll().unwrap();
            if matches!(source.poll().unwrap(), Some(DisplayEvent::InputClosed)) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Input close did not notify adapter"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(source.input_channel.is_none() && source.input_role.is_none());
        let position = br#"{"revision":1,"sequence":1,"x":0.5,"y":0.2,"visible":true}"#;
        assert!(source.write_cursor(position).unwrap());
        loop {
            source.poll().unwrap();
            if let Some(DisplayEvent::Cursor(bytes)) = receiver.poll().unwrap() {
                assert_eq!(bytes, position);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Input close stopped cursor stream"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn timing_packet(sequence: u16, timestamp: u32, ssrc: u32, marker: bool) -> Vec<u8> {
        let mut packet = vec![0x80, 98 | if marker { 128 } else { 0 }];
        packet.extend(sequence.to_be_bytes());
        packet.extend(timestamp.to_be_bytes());
        packet.extend(ssrc.to_be_bytes());
        packet.extend([0; 16]);
        packet
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_sharing_service_is_verified_on_ipv4_and_ipv6() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let socket = UdpSocket::bind(address).unwrap();
            responsive_screen_service(&socket).unwrap();
        }
    }

    #[test]
    fn experimental_priority_preserves_default_and_validates_dscp() {
        assert_eq!(experimental_dscp(None).unwrap(), None);
        assert_eq!(experimental_dscp(Some("34")).unwrap(), Some(136));
        assert_eq!(experimental_dscp(Some("0")).unwrap(), Some(0));
        assert!(experimental_dscp(Some("64")).is_err());
        assert!(experimental_dscp(Some("-1")).is_err());
        assert!(experimental_dscp(Some("af41")).is_err());
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let reference = socket2::SockRef::from(&socket);
        reference.set_tos_v4(136).unwrap();
        assert_eq!(reference.tos_v4().unwrap(), 136);
    }

    #[test]
    fn timing_header_only_parses_valid_rtp_layouts() {
        let packet = timing_packet(u16::MAX, u32::MAX, 7, true);
        let header = timing_rtp_header(&packet).unwrap();
        assert_eq!(
            (
                header.pt,
                header.ssrc,
                header.timestamp,
                header.sequence,
                header.marker
            ),
            (98, 7, u32::MAX, u16::MAX, true)
        );
        let mut rtcp = packet.clone();
        rtcp[1] = 200;
        assert!(timing_rtp_header(&rtcp).is_none());
        let mut dtls = packet.clone();
        dtls[0] = 22;
        assert!(timing_rtp_header(&dtls).is_none());
        assert!(timing_rtp_header(&packet[..11]).is_none());
        let mut extension = packet;
        extension[0] = 0x91;
        extension[18] = 0;
        extension[19] = 2;
        assert!(timing_rtp_header(&extension).is_some());
        extension[19] = 100;
        assert!(timing_rtp_header(&extension).is_none());
    }

    #[test]
    fn reordering_experiment_preserves_default_and_rejects_invalid_deadlines() {
        assert_eq!(
            experimental_reordering(None).unwrap(),
            Duration::from_millis(500)
        );
        for ms in [20, 100, 500, 1000] {
            assert_eq!(
                experimental_reordering(Some(&ms.to_string())).unwrap(),
                Duration::from_millis(ms)
            );
        }
        for value in ["0", "19", "1001", "-1", "100.5", "", "fast"] {
            assert!(experimental_reordering(Some(value)).is_err());
        }
    }

    #[test]
    fn timing_separates_regular_socket_reads_from_bursty_assembly() {
        let start = Instant::now();
        let mut timing = TransportTiming::new(start);
        timing.received(&timing_packet(u16::MAX, 9000, 7, false), start);
        timing.received(
            &timing_packet(0, 9000, 7, true),
            start + Duration::from_millis(1),
        );
        timing.received(
            &timing_packet(1, 10500, 7, false),
            start + Duration::from_millis(16),
        );
        timing.received(
            &timing_packet(2, 10500, 7, true),
            start + Duration::from_millis(17),
        );
        let now = start + Duration::from_millis(35);
        timing.assembled(98, 9000, start, now, (65535, 65536), true);
        timing.assembled(
            98,
            10500,
            start + Duration::from_millis(16),
            now,
            (65537, 65538),
            true,
        );
        assert_eq!(timing.receive.streams[0].group_gaps.count, 1);
        assert_eq!(timing.receive.streams[0].group_gaps.sum_us, 16_000);
        assert_eq!(timing.assembly_gaps.count, 1);
        assert_eq!(timing.assembly_gaps.sum_us, 0);
        assert_eq!(timing.first_input_to_assembly.max_us, 35_000);
        assert_eq!(timing.last_read_to_assembly.max_us, 34_000);
        assert_eq!(timing.samples[0]["first_sequence_seen"], u16::MAX);
        assert_eq!(timing.samples[0]["last_sequence_seen"], 0);
        assert_eq!(timing.frames.len(), 0);
        assert_eq!(timing.slow_samples.len(), 1);
        assert_eq!(timing.slow_samples[0]["assembled_sequence_end"], 65536);
        assert_eq!(
            timing.slow_samples[0]["last_matching_read_to_assembly_us"],
            34000
        );
    }

    #[test]
    fn timing_accounts_for_padding_flag_without_inspecting_encrypted_payload() {
        let mut direction = TimingDirection::default();
        let now = Instant::now();
        let regular = timing_packet(1, 9000, 7, true);
        let mut padded = regular.clone();
        padded[0] |= 32;
        // The trailer is encrypted SRTP data, not an accessible padding count.
        padded.extend_from_slice(&[0xff; 200]);
        assert!(!timing_rtp_header(&regular).unwrap().padding_flag);
        assert!(timing_rtp_header(&padded).unwrap().padding_flag);
        direction.observe(&regular, now);
        direction.observe(&padded, now);
        let result = direction.take_json();
        let stream = &result["rtp_streams"][0];
        assert_eq!(stream["packets"], 2);
        assert_eq!(stream["udp_payload_bytes"], regular.len() + padded.len());
        assert_eq!(stream["padding_flag_packets"], 1);
        assert_eq!(stream["padding_flag_udp_payload_bytes"], padded.len());
        let next = direction.take_json();
        assert_eq!(next["rtp_streams"][0]["udp_payload_bytes"], 0);
        assert_eq!(next["rtp_streams"][0]["padding_flag_packets"], 0);
    }

    #[test]
    fn kernel_timing_separates_queue_delay_from_arrival_groups_and_missing_data() {
        let mut timing = KernelTiming::default();
        timing.observe(
            &timing_packet(1, 9000, 7, false),
            Some(1_000_000_000),
            Some(1_002_000_000),
        );
        timing.observe(
            &timing_packet(2, 9000, 7, true),
            Some(1_001_000_000),
            Some(1_003_000_000),
        );
        timing.observe(
            &timing_packet(3, 10500, 7, true),
            Some(1_016_000_000),
            Some(1_018_000_000),
        );
        timing.observe(&timing_packet(4, 12000, 7, true), None, Some(1_020_000_000));
        let result = timing.take_json();
        assert_eq!(result["timestamped_datagrams"], 3);
        assert_eq!(result["missing_timestamp_datagrams"], 1);
        assert_eq!(result["kernel_to_userspace"]["mean_ms"], 2.0);
        assert_eq!(
            result["rtp_streams"][0]["first_seen_rtp_group_gaps"]["count"],
            1
        );
        assert_eq!(
            result["rtp_streams"][0]["first_seen_rtp_group_gaps"]["max_ms"],
            16.0
        );
        assert_eq!(timing.take_json()["timestamped_datagrams"], 0);
    }

    #[test]
    fn kernel_timing_rejects_negative_queue_clock_and_bounds_stream_storage() {
        let mut timing = KernelTiming::default();
        timing.observe(
            &timing_packet(1, 9000, 7, true),
            Some(10_000_000),
            Some(9_000_000),
        );
        for ssrc in 0..64 {
            timing.observe(
                &timing_packet(1, 9000, ssrc, true),
                Some(11_000_000),
                Some(12_000_000),
            );
        }
        assert_eq!(timing.streams.len(), 8);
        let result = timing.take_json();
        assert_eq!(result["invalid_clock_samples"], 1);
        assert_eq!(result["kernel_to_userspace"]["count"], 64);
    }

    #[test]
    fn timing_retains_worst_holds_after_later_fast_groups() {
        let start = Instant::now();
        let mut timing = TransportTiming::new(start);
        for (index, delay) in [26, 31, 70, 40, 55, 80, 1, 1, 1, 1].into_iter().enumerate() {
            let timestamp = index as u32 * 1500;
            let at = start + Duration::from_millis(index as u64 * 100);
            timing.received(&timing_packet(index as u16, timestamp, 7, true), at);
            timing.assembled(
                98,
                timestamp,
                at,
                at + Duration::from_millis(delay),
                (index as u64, index as u64),
                index != 5,
            );
        }
        let holds: Vec<_> = timing
            .slow_samples
            .iter()
            .map(|s| s["last_matching_read_to_assembly_us"].as_u64().unwrap())
            .collect();
        assert_eq!(holds, vec![80_000, 70_000, 55_000, 40_000]);
        assert_eq!(timing.slow_samples[0]["contiguous_from_previous"], false);
        assert_eq!(timing.samples.len(), 4);
        assert!(timing
            .samples
            .iter()
            .all(|s| s["last_matching_read_to_assembly_us"] == 1000));
    }

    #[test]
    fn timing_bounds_streams_pending_groups_and_samples() {
        let start = Instant::now();
        let mut timing = TransportTiming::new(start);
        for timestamp in 0..200 {
            timing.received(&timing_packet(1, timestamp, 7, false), start);
        }
        assert_eq!(timing.frames.len(), 128);
        assert_eq!(timing.evicted_groups, 72);
        for ssrc in 0..100 {
            timing.received(&timing_packet(1, 201, ssrc, false), start);
        }
        assert_eq!(timing.receive.streams.len(), 8);
        assert!(timing.receive.untracked_stream_packets > 0);
        for timestamp in 300..310 {
            timing.received(&timing_packet(1, timestamp, 7, true), start);
            timing.assembled(98, timestamp, start, start, (1, 1), true);
        }
        assert_eq!(timing.samples.len(), 4);
        let packets = timing.receive.udp_packets;
        let window = timing.receive.take_json();
        assert_eq!(window["udp_packets"], packets);
        assert_eq!(timing.receive.udp_packets, 0);
    }

    #[test]
    fn cursor_rejects_out_of_bounds_and_oversized_messages() {
        assert!(
            validate_cursor(br#"{"revision":1,"sequence":1,"x":0.5,"y":0.2,"visible":true}"#)
                .is_ok()
        );
        assert!(
            validate_cursor(br#"{"revision":1,"sequence":1,"x":1.1,"y":0.2,"visible":true}"#)
                .is_err()
        );
        assert!(validate_cursor(&vec![0; 128 * 1024 + 1]).is_err());
        assert!(validate_cursor(
            br#"{"revision":1,"png":"","width":10000,"height":1,"hotspot_x":0,"hotspot_y":0}"#
        )
        .is_err());
    }
    #[test]
    fn hevc_and_cursor_state_cross_encrypted_transport_without_video_dependency() {
        let ip = "127.0.0.1".parse().unwrap();
        let mut sender = Transport::with_codec(ip, VideoCodec::Hevc).unwrap();
        let mut receiver = Transport::with_codec(ip, VideoCodec::Hevc).unwrap();
        let offer = sender.offer().unwrap();
        assert!(offer.contains("H265"));
        let answer = receiver.answer(&offer).unwrap();
        sender.accept_answer(&answer).unwrap();
        let position = br#"{"revision":1,"sequence":1,"x":0.5,"y":0.2,"visible":true}"#;
        let shape =
            br#"{"revision":1,"png":"AA==","width":16,"height":16,"hotspot_x":0,"hotspot_y":0}"#;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut position_sent = false;
        let mut shape_sent = false;
        let mut received_position = false;
        let mut received_shape = false;
        let mut sent_video = false;
        loop {
            sender.poll().unwrap();
            match receiver.poll().unwrap() {
                Some(DisplayEvent::Cursor(bytes)) if bytes == position => received_position = true,
                Some(DisplayEvent::Cursor(bytes)) if bytes == shape => received_shape = true,
                Some(DisplayEvent::Frame(frame)) => {
                    assert!(
                        received_position && received_shape,
                        "Cursor must arrive before video is sent"
                    );
                    assert!(frame.keyframe);
                    assert!(frame.bytes.starts_with(&[0, 0, 0, 1, 38, 1]));
                    return;
                }
                _ => {}
            }
            if !position_sent {
                position_sent = sender.write_cursor(position).unwrap();
            }
            if !shape_sent {
                shape_sent = sender.write_cursor(shape).unwrap();
            }
            if received_position && received_shape && !sent_video {
                let mut bytes = vec![0, 0, 0, 1, 38, 1];
                bytes.extend(vec![0x88; 32_000]);
                sent_video = sender
                    .write(Frame {
                        timestamp: 1_000_000_000,
                        keyframe: true,
                        bytes,
                    })
                    .unwrap();
            }
            assert!(Instant::now() < deadline, "HEVC/cursor transport timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn queued_events_do_not_starve_udp_receive() {
        let mut receiver = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
        let unrelated = UdpSocket::bind("127.0.0.1:0").unwrap();
        unrelated
            .send_to(&[1, 2, 3], receiver.socket.local_addr().unwrap())
            .unwrap();
        receiver.events.push_back(DisplayEvent::Connected);
        assert!(matches!(
            receiver.poll().unwrap(),
            Some(DisplayEvent::Connected)
        ));
        let error = receiver.socket.recv_from(&mut [0; 16]).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    }
    #[test]
    fn unrelated_udp_datagrams_do_not_end_display_transport() {
        let mut receiver = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
        let unrelated = UdpSocket::bind("127.0.0.1:0").unwrap();
        unrelated
            .send_to(&[1, 2, 3], receiver.socket.local_addr().unwrap())
            .unwrap();
        assert!(receiver.poll().unwrap().is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn kernel_preference_respects_explicit_override_and_keeps_existing_clock_state() {
        let mut transport = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
        // Inject override presence without changing the process environment or
        // interfering with other transport tests running in parallel.
        transport.kernel_arrival = None;
        transport.prefer_kernel_arrival_if_unset(false);
        assert!(transport.kernel_arrival.is_none());
        transport.prefer_kernel_arrival_if_unset(true);
        let arrival = transport.kernel_arrival.as_mut().unwrap();
        assert!(arrival.active());
        arrival.record(true, Instant::now());
        transport.prefer_kernel_arrival_if_unset(true);
        assert_eq!(transport.kernel_arrival.as_ref().unwrap().counts(), (1, 0));
    }

    #[test]
    fn missing_video_packet_requests_recovery_without_seconds_of_waiting() {
        check_missing_video_packet_recovery(VideoCodec::H264, 500, 750, None, true);
    }

    #[test]
    fn intact_video_resumes_after_idle_without_waiting_for_loss_deadline() {
        // Isolate transport behavior from capture, encoding, rendering and Wi-Fi.
        // A static desktop may produce no new frames for several seconds.
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let ip = "127.0.0.1".parse().unwrap();
            let mut sender = Transport::configured(ip, Duration::from_millis(500), codec).unwrap();
            let mut receiver =
                Transport::configured(ip, Duration::from_millis(500), codec).unwrap();
            #[cfg(target_os = "macos")]
            {
                let DatagramSocket::Udp(socket) = &receiver.socket else {
                    panic!("UDP timing test requires a UDP socket");
                };
                socket_timing::enable(socket).unwrap();
                receiver.kernel_arrival = Some(kernel_arrival::Receiver::new(Some(socket), false));
                receiver.kernel_timestamping = true;
                receiver.timing = Some(TransportTiming::new(Instant::now()));
            }
            let offer = sender.offer().unwrap();
            let answer = receiver.answer(&offer).unwrap();
            sender.accept_answer(&answer).unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut connected = false;
            let mut priming_sent = false;
            let mut idle_start: Option<Instant> = None;
            let mut resumed_at: Option<Instant> = None;
            let mut bytes = if codec == VideoCodec::Hevc {
                vec![0, 0, 0, 1, 38, 1]
            } else {
                vec![0, 0, 0, 1, 0x65]
            };
            bytes.extend(vec![0x88; 32_000]);
            loop {
                connected |= matches!(sender.poll().unwrap(), Some(DisplayEvent::Connected));
                match receiver.poll().unwrap() {
                    Some(DisplayEvent::Frame(frame)) if frame.timestamp == 1_000_000_000 => {
                        assert_eq!(frame.bytes, bytes);
                        idle_start = Some(Instant::now());
                    }
                    Some(DisplayEvent::Frame(frame)) if frame.timestamp == 3_000_000_000 => {
                        assert_eq!(frame.bytes, bytes);
                        let elapsed = resumed_at.unwrap().elapsed();
                        eprintln!(
                            "Intact idle resume: codec={} elapsed_ms={:.3}",
                            codec.name(),
                            elapsed.as_secs_f64() * 1000.0
                        );
                        assert!(
                            elapsed < Duration::from_millis(200),
                            "An intact resumed frame waited near the loss recovery deadline"
                        );
                        #[cfg(target_os = "macos")]
                        {
                            let observed = &receiver.timing.as_ref().unwrap().kernel_receive;
                            assert!(observed.timestamped > 0);
                            assert!(observed.queue_delay.count > 0);
                            assert_eq!(observed.missing, 0);
                            let (applied, fallback) =
                                receiver.kernel_arrival.as_ref().unwrap().counts();
                            assert!(applied > 0);
                            assert_eq!(fallback, 0);
                        }
                        break;
                    }
                    Some(DisplayEvent::Keyframe) => panic!("No media was intentionally lost"),
                    _ => {}
                }
                if connected && !priming_sent {
                    sender
                        .write(Frame {
                            timestamp: 1_000_000_000,
                            keyframe: true,
                            bytes: bytes.clone(),
                        })
                        .unwrap();
                    priming_sent = true;
                }
                if resumed_at.is_none()
                    && idle_start.is_some_and(|start| start.elapsed() >= Duration::from_secs(2))
                {
                    sender
                        .write(Frame {
                            timestamp: 3_000_000_000,
                            keyframe: true,
                            bytes: bytes.clone(),
                        })
                        .unwrap();
                    resumed_at = Some(Instant::now());
                    #[cfg(target_os = "macos")]
                    {
                        // The queued media arrived before this newer RTC tick.
                        // Kernel arrivals must remain usable without resetting
                        // or clamping the RTC timeout clock backwards.
                        std::thread::sleep(Duration::from_millis(25));
                        receiver
                            .rtc
                            .handle_input(Input::Timeout(Instant::now()))
                            .unwrap();
                    }
                }
                assert!(Instant::now() < deadline, "Idle resume timed out");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn missing_hevc_packet_requests_recovery_without_seconds_of_waiting() {
        check_missing_video_packet_recovery(VideoCodec::Hevc, 500, 750, None, true);
    }

    #[test]
    fn short_deadline_requests_h264_loss_recovery() {
        check_missing_video_packet_recovery(VideoCodec::H264, 100, 250, None, true);
    }

    #[test]
    fn short_deadline_requests_hevc_loss_recovery() {
        check_missing_video_packet_recovery(VideoCodec::Hevc, 100, 250, None, true);
    }

    #[test]
    fn reordered_video_within_short_deadline_still_delivers_intact() {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            check_missing_video_packet_recovery(codec, 100, 250, Some(40), false);
        }
    }

    #[test]
    fn long_deadline_preserves_late_reordered_video() {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            check_missing_video_packet_recovery(codec, 500, 350, Some(180), false);
        }
    }

    #[test]
    fn short_deadline_recovers_before_very_late_reordered_video() {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            check_missing_video_packet_recovery(codec, 100, 250, Some(180), true);
        }
    }

    fn check_missing_video_packet_recovery(
        codec: VideoCodec,
        deadline_ms: u64,
        limit_ms: u64,
        packet_delay_ms: Option<u64>,
        expect_recovery: bool,
    ) {
        let ip = "127.0.0.1".parse().unwrap();
        let mut sender =
            Transport::configured(ip, Duration::from_millis(deadline_ms), codec).unwrap();
        let mut receiver =
            Transport::configured(ip, Duration::from_millis(deadline_ms), codec).unwrap();
        let proxy = UdpSocket::bind((ip, 0)).unwrap();
        proxy.set_nonblocking(true).unwrap();
        let a = sender.socket.local_addr().unwrap();
        let b = receiver.socket.local_addr().unwrap();
        let port = proxy.local_addr().unwrap().port();
        let offer = sender.offer().unwrap().replace(
            &format!("{} typ host", a.port()),
            &format!("{port} typ host"),
        );
        let codec_name = if codec == VideoCodec::Hevc {
            "H265/"
        } else {
            "H264/"
        };
        let sdp = serde_json::from_str::<SdpOffer>(&offer)
            .unwrap()
            .to_sdp_string();
        let video_pt: u8 = sdp
            .lines()
            .find_map(|line| {
                let (pt, format) = line.strip_prefix("a=rtpmap:")?.split_once(' ')?;
                format.starts_with(codec_name).then(|| pt.parse().unwrap())
            })
            .expect("Offer must identify the media payload type");
        let answer = receiver.answer(&offer).unwrap().replace(
            &format!("{} typ host", b.port()),
            &format!("{port} typ host"),
        );
        sender.accept_answer(&answer).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut connected = false;
        let mut sent_at: Option<Instant> = None;
        let mut primer_sent = false;
        let mut primed = false;
        let mut dropped = false;
        let mut delayed: Option<(Vec<u8>, Instant)> = None;
        let mut released = false;
        let mut expected = if codec == VideoCodec::Hevc {
            vec![0, 0, 0, 1, 38, 1]
        } else {
            vec![0, 0, 0, 1, 0x65]
        };
        expected.extend(vec![0x88; 16_000]);
        loop {
            if delayed
                .as_ref()
                .is_some_and(|(_, due)| Instant::now() >= *due)
            {
                let (packet, _) = delayed.take().unwrap();
                proxy.send_to(&packet, b).unwrap();
                released = true;
            }
            let mut bytes = [0; 2048];
            for _ in 0..128 {
                match proxy.recv_from(&mut bytes) {
                    Ok((size, from)) => {
                        // RTP headers remain visible with SRTP. Drop one media
                        // packet, without decrypting or modifying its contents.
                        if sent_at.is_some()
                            && from == a
                            && !dropped
                            && size >= 12
                            && bytes[0] & 0xc0 == 0x80
                            && bytes[0] & 0x20 == 0
                            && bytes[1] & 0x7f == video_pt
                        {
                            dropped = true;
                            if let Some(ms) = packet_delay_ms {
                                delayed = Some((
                                    bytes[..size].to_vec(),
                                    Instant::now() + Duration::from_millis(ms),
                                ));
                            }
                            continue;
                        }
                        // Isolate unrepaired loss/reordering from RTX repair.
                        // Suppress RTX until a held packet is released (forever
                        // in the permanent-loss case), without inspecting SRTP.
                        if dropped
                            && !released
                            && from == a
                            && timing_rtp_header(&bytes[..size])
                                .is_some_and(|header| header.pt != video_pt)
                        {
                            continue;
                        }
                        proxy
                            .send_to(&bytes[..size], if from == a { b } else { a })
                            .unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("Proxy failed: {error}"),
                }
            }
            if matches!(sender.poll().unwrap(), Some(DisplayEvent::Connected)) {
                connected = true;
            }
            let received = receiver.poll().unwrap();
            if let Some(DisplayEvent::Frame(frame)) = &received {
                if sent_at.is_none() && frame.keyframe {
                    primed = true;
                } else if frame.keyframe {
                    assert!(
                        !expect_recovery,
                        "Incomplete frame must not bypass recovery deadline"
                    );
                    assert!(
                        dropped && released,
                        "Reordering case must release held video"
                    );
                    assert_eq!(
                        frame.bytes, expected,
                        "Reordered encrypted frame must remain intact"
                    );
                    assert!(sent_at.unwrap().elapsed() < Duration::from_millis(limit_ms));
                    eprintln!("Video reorder delivered: codec={} deadline_ms={} held_ms={} observed_ms={:.3}",
                        codec.name(), deadline_ms, packet_delay_ms.unwrap(), sent_at.unwrap().elapsed().as_secs_f64()*1000.0);
                    return;
                }
            }
            if matches!(received, Some(DisplayEvent::Keyframe)) {
                assert!(
                    expect_recovery,
                    "Recoverable reordered frame was abandoned too early"
                );
                assert!(dropped, "Test must lose an encrypted media packet");
                let recovery = sent_at.unwrap().elapsed();
                eprintln!(
                    "Video loss recovery: codec={} deadline_ms={} observed_ms={:.3}",
                    codec.name(),
                    deadline_ms,
                    recovery.as_secs_f64() * 1000.0
                );
                assert!(
                    recovery < Duration::from_millis(limit_ms),
                    "Recovery waited for the general streaming deadline"
                );
                return;
            }
            if connected && !primer_sent {
                let mut primer = expected[..if codec == VideoCodec::Hevc { 6 } else { 5 }].to_vec();
                primer.extend_from_slice(&[0x88; 100]);
                sender
                    .write(Frame {
                        timestamp: 500_000_000,
                        keyframe: true,
                        bytes: primer,
                    })
                    .unwrap();
                primer_sent = true;
            }
            if primed && sent_at.is_none() {
                sender
                    .write(Frame {
                        timestamp: 1_000_000_000,
                        keyframe: true,
                        bytes: expected.clone(),
                    })
                    .unwrap();
                sender
                    .write(Frame {
                        timestamp: 1_016_666_667,
                        keyframe: false,
                        bytes: if codec == VideoCodec::Hevc {
                            vec![0, 0, 0, 1, 2, 1, 0x88]
                        } else {
                            vec![0, 0, 0, 1, 0x41, 0x88]
                        },
                    })
                    .unwrap();
                sent_at = Some(Instant::now());
            }
            assert!(Instant::now() < deadline, "Packet-loss recovery timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn encrypted_video_survives_two_second_bidirectional_packet_blackout() {
        // This is an owned loopback transport test, not a network/panel
        // benchmark. Both codecs must accept fresh media after an outage
        // without replacing their DTLS/SRTP connection or modifying the OS.
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let ip = "127.0.0.1".parse().unwrap();
            let mut sender = Transport::configured(ip, Duration::from_millis(500), codec).unwrap();
            let mut receiver =
                Transport::configured(ip, Duration::from_millis(500), codec).unwrap();
            let proxy = UdpSocket::bind((ip, 0)).unwrap();
            proxy.set_nonblocking(true).unwrap();
            let a = sender.socket.local_addr().unwrap();
            let b = receiver.socket.local_addr().unwrap();
            let port = proxy.local_addr().unwrap().port();
            let offer = sender.offer().unwrap().replace(
                &format!("{} typ host", a.port()),
                &format!("{port} typ host"),
            );
            let answer = receiver.answer(&offer).unwrap().replace(
                &format!("{} typ host", b.port()),
                &format!("{port} typ host"),
            );
            sender.accept_answer(&answer).unwrap();
            let mut bytes = if codec == VideoCodec::Hevc {
                vec![0, 0, 0, 1, 38, 1]
            } else {
                vec![0, 0, 0, 1, 0x65]
            };
            bytes.extend_from_slice(&[0x88; 100]);
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut connected = false;
            let mut primer_sent = false;
            let mut blackout: Option<Instant> = None;
            let mut resumed: Option<Instant> = None;
            let mut next_frame = Instant::now();
            let mut sequence = 0_u64;
            let mut dropped = [0_u64; 2];
            loop {
                let dropping =
                    blackout.is_some_and(|start| start.elapsed() < Duration::from_secs(2));
                let mut packet = [0_u8; 2048];
                for _ in 0..128 {
                    match proxy.recv_from(&mut packet) {
                        Ok((size, from)) => {
                            assert!(from == a || from == b, "Unexpected proxy peer");
                            if dropping {
                                dropped[usize::from(from == b)] += 1;
                                continue;
                            }
                            proxy
                                .send_to(&packet[..size], if from == a { b } else { a })
                                .unwrap();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) => panic!("Proxy failed: {error}"),
                    }
                }
                if matches!(sender.poll().unwrap(), Some(DisplayEvent::Connected)) {
                    connected = true;
                }
                if let Some(DisplayEvent::Frame(frame)) = receiver.poll().unwrap() {
                    if blackout.is_none() {
                        assert_eq!(frame.bytes, bytes);
                        blackout = Some(Instant::now());
                    } else if resumed.is_some() && frame.timestamp >= 3_000_000_000 {
                        assert_eq!(
                            frame.bytes, bytes,
                            "Fresh encrypted access unit must remain intact"
                        );
                        assert!(
                            dropped.iter().all(|count| *count > 0),
                            "Blackout must drop both directions"
                        );
                        assert!(
                            resumed.unwrap().elapsed() < Duration::from_secs(1),
                            "Fresh media did not recover promptly"
                        );
                        break;
                    }
                }
                if connected && !primer_sent {
                    sender
                        .write(Frame {
                            timestamp: 500_000_000,
                            keyframe: true,
                            bytes: bytes.clone(),
                        })
                        .unwrap();
                    primer_sent = true;
                }
                if blackout.is_some() && Instant::now() >= next_frame {
                    sequence += 1;
                    // Receiving the primer can start the blackout after the
                    // proxy loop's earlier state sample. Re-read for this send.
                    let sending_during_blackout =
                        blackout.unwrap().elapsed() < Duration::from_secs(2);
                    let timestamp = if sending_during_blackout {
                        1_000_000_000 + sequence * 1_000_000
                    } else {
                        3_000_000_000 + sequence * 50_000_000
                    };
                    if !sending_during_blackout {
                        resumed.get_or_insert_with(Instant::now);
                    }
                    // An independently decodable access unit isolates transport
                    // survival from the application's keyframe scheduling.
                    sender
                        .write(Frame {
                            timestamp,
                            keyframe: true,
                            bytes: bytes.clone(),
                        })
                        .unwrap();
                    next_frame = Instant::now() + Duration::from_millis(50);
                }
                assert!(
                    Instant::now() < deadline,
                    "Bidirectional blackout recovery timed out"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn public_addresses_keep_packet_pacing() {
        for ip in [
            "127.0.0.1",
            "10.0.100.23",
            "169.254.1.2",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(
                local_address(ip.parse().unwrap()),
                "Expected local address: {ip}"
            );
        }
        for ip in ["8.8.8.8", "2001:4860:4860::8888"] {
            assert!(
                !local_address(ip.parse().unwrap()),
                "Expected public address: {ip}"
            );
        }
    }

    #[test]
    fn drained_packet_snapshot_does_not_pause_capture() {
        let ip = "127.0.0.1".parse().unwrap();
        let mut sender = Transport::new(ip).unwrap();
        let mut receiver = Transport::new(ip).unwrap();
        let offer = sender.offer().unwrap();
        let answer = receiver.answer(&offer).unwrap();
        sender.accept_answer(&answer).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let connected = matches!(sender.poll().unwrap(), Some(DisplayEvent::Connected));
            receiver.poll().unwrap();
            if connected {
                break;
            }
            assert!(Instant::now() < deadline, "Transport setup timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        let mid = sender.mid.unwrap();
        sender
            .rtc
            .direct_api()
            .stream_tx_by_mid(mid, None)
            .unwrap()
            .set_unpaced(true);
        assert!(sender
            .write(Frame {
                timestamp: 1_000_000_000,
                keyframe: true,
                bytes: vec![0, 0, 0, 1, 0x65, 0x88],
            })
            .unwrap());
        // Inject another frame through the library API, without Transport's
        // post-write refresh, to recreate the cached already-sent snapshot.
        let writer = sender.rtc.writer(mid).unwrap();
        let pt = writer
            .payload_params()
            .find(|p| p.spec().codec == Codec::H264)
            .unwrap()
            .pt();
        writer
            .write(
                pt,
                Instant::now(),
                MediaTime::from(Duration::from_nanos(1_016_666_667)),
                vec![0, 0, 0, 1, 0x41, 0x88],
            )
            .unwrap();
        sender
            .rtc
            .handle_input(Input::Timeout(Instant::now()))
            .unwrap();
        // Stop at the first timeout instead of using drain(), which now also
        // services due timeouts and would refresh away the stale snapshot.
        loop {
            match sender.rtc.poll_output().unwrap() {
                Output::Transmit(packet) => {
                    sender
                        .socket
                        .send_to(&packet.contents, packet.destination)
                        .unwrap();
                }
                Output::Timeout(_) => break,
                Output::Event(_) => {}
            }
        }
        // The cached snapshot still includes the packet just transmitted.
        assert!(
            sender
                .rtc
                .direct_api()
                .stream_tx_by_mid(mid, None)
                .unwrap()
                .queue_info()
                .unwrap()
                .packet_count()
                > 0
        );
        std::thread::sleep(Duration::from_millis(40));
        sender.lan_route = false;
        assert!(
            !sender.pause_capture(60).unwrap(),
            "Already-sent packets paused capture"
        );
        assert!(
            !sender.lan_route,
            "Regression must exercise the paced-route queue check"
        );
    }

    #[test]
    fn encrypted_udp_video_reassembles_fragmented_access_unit() {
        fragmented_access_unit(false);
    }
    struct TestPacketLink {
        address: SocketAddr,
        peer: SocketAddr,
        outgoing: std::sync::mpsc::SyncSender<Vec<u8>>,
        incoming: std::sync::Mutex<std::sync::mpsc::Receiver<Vec<u8>>>,
        backpressure: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl DatagramLink for TestPacketLink {
        fn local_addr(&self) -> std::io::Result<SocketAddr> {
            Ok(self.address)
        }
        fn send_to(&self, bytes: &[u8], destination: SocketAddr) -> std::io::Result<usize> {
            if destination != self.peer {
                return Err(std::io::ErrorKind::AddrNotAvailable.into());
            }
            // The carrier must only see encrypted media, never the repeated
            // encoded payload used by this test.
            assert!(!contains_test_plaintext(bytes));
            self.outgoing.try_send(bytes.to_vec()).map_err(|error| {
                std::io::Error::from(match error {
                    std::sync::mpsc::TrySendError::Full(_) => {
                        self.backpressure
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        std::io::ErrorKind::WouldBlock
                    }
                    std::sync::mpsc::TrySendError::Disconnected(_) => {
                        std::io::ErrorKind::BrokenPipe
                    }
                })
            })?;
            Ok(bytes.len())
        }
        fn recv_from(&self, bytes: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
            let packet = self.incoming.lock().unwrap().try_recv().map_err(|error| {
                std::io::Error::from(match error {
                    std::sync::mpsc::TryRecvError::Empty => std::io::ErrorKind::WouldBlock,
                    std::sync::mpsc::TryRecvError::Disconnected => std::io::ErrorKind::BrokenPipe,
                })
            })?;
            if packet.len() > bytes.len() {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            bytes[..packet.len()].copy_from_slice(&packet);
            Ok((packet.len(), self.peer))
        }
    }
    #[test]
    fn packet_link_reassembles_encrypted_video_without_udp_sockets() {
        packet_link_video(false);
    }
    #[test]
    fn link_local_ice_mapping_preserves_encrypted_video_and_physical_destinations() {
        packet_link_video(true);
    }
    fn packet_link_video(link_local: bool) {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let (a, b): (SocketAddr, SocketAddr) = if link_local {
                (
                    "169.254.226.32:48180".parse().unwrap(),
                    "169.254.173.134:48180".parse().unwrap(),
                )
            } else {
                (
                    "192.0.2.1:48180".parse().unwrap(),
                    "192.0.2.2:48180".parse().unwrap(),
                )
            };
            let (a_tx, a_rx) = std::sync::mpsc::sync_channel(1);
            let (b_tx, b_rx) = std::sync::mpsc::sync_channel(1);
            let backpressure = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let sender_link = Box::new(TestPacketLink {
                address: a,
                peer: b,
                outgoing: a_tx,
                incoming: std::sync::Mutex::new(b_rx),
                backpressure: backpressure.clone(),
            });
            let mut sender = if link_local {
                Transport::configured_link(
                    a.ip(),
                    Duration::from_millis(500),
                    codec,
                    Some(sender_link),
                    Some(b.ip()),
                )
            } else {
                Transport::with_packet_link(codec, sender_link)
            }
            .unwrap();
            let receiver_link = Box::new(TestPacketLink {
                address: b,
                peer: a,
                outgoing: b_tx,
                incoming: std::sync::Mutex::new(a_rx),
                backpressure: backpressure.clone(),
            });
            let mut receiver = if link_local {
                Transport::configured_link(
                    b.ip(),
                    Duration::from_millis(500),
                    codec,
                    Some(receiver_link),
                    Some(a.ip()),
                )
            } else {
                Transport::with_packet_link(codec, receiver_link)
            }
            .unwrap();
            #[cfg(target_os = "macos")]
            {
                sender.prefer_kernel_arrival_if_unset(true);
                receiver.prefer_kernel_arrival_if_unset(true);
                assert!(sender.kernel_arrival.is_none());
                assert!(receiver.kernel_arrival.is_none());
            }
            assert!(matches!(sender.socket, DatagramSocket::Link(_)));
            assert!(matches!(receiver.socket, DatagramSocket::Link(_)));
            let offer = sender.offer().unwrap();
            if link_local {
                assert!(offer.contains("10.254.226.32"));
                assert!(!offer.contains("169.254."));
            }
            let answer = receiver.answer(&offer).unwrap();
            if link_local {
                assert!(answer.contains("10.254.173.134"));
                assert!(!answer.contains("169.254."));
            }
            sender.accept_answer(&answer).unwrap();
            let mut bytes = if codec == VideoCodec::Hevc {
                vec![0, 0, 0, 1, 38, 1]
            } else {
                vec![0, 0, 0, 1, 0x65]
            };
            bytes.extend(vec![0x88; 400_000]);
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut sent = false;
            loop {
                if matches!(sender.poll().unwrap(), Some(DisplayEvent::Connected)) && !sent {
                    assert!(sender
                        .write(Frame {
                            timestamp: 1_000_000_000,
                            keyframe: true,
                            bytes: bytes.clone()
                        })
                        .unwrap());
                    sent = true;
                }
                if let Some(DisplayEvent::Frame(frame)) = receiver.poll().unwrap() {
                    assert_eq!(frame.bytes, bytes);
                    assert!(frame.keyframe);
                    assert!(!sender.lan_route);
                    assert!(
                        backpressure.load(std::sync::atomic::Ordering::Relaxed) > 0,
                        "Test must exercise a full bounded carrier queue"
                    );
                    break;
                }
                assert!(Instant::now() < deadline, "Encrypted packet link timed out: codec={} sent={} sender_connected={} receiver_connected={} pending={}/{}", codec.name(), sent, sender.connected, receiver.connected, sender.pending_transmit.is_some(), receiver.pending_transmit.is_some());
                std::thread::sleep(Duration::from_micros(100));
            }
        }
    }
    #[test]
    fn paced_lan_reassembles_large_encrypted_frame_without_per_packet_sleeps() {
        fragmented_access_unit(true);
    }
    fn fragmented_access_unit(paced_lan: bool) {
        let ip = "127.0.0.1".parse().unwrap();
        let mut sender = Transport::new(ip).unwrap();
        sender.paced_lan = paced_lan;
        let mut receiver = Transport::new(ip).unwrap();
        let offer = sender.offer().unwrap();
        let answer = receiver.answer(&offer).unwrap();
        sender.accept_answer(&answer).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut ready = false;
        let mut sent = false;
        let mut sent_at: Option<Instant> = None;
        let mut bytes = vec![0, 0, 0, 1, 0x65];
        bytes.extend(vec![0x88; 400_000]);
        loop {
            if matches!(sender.poll().unwrap(), Some(DisplayEvent::Connected)) {
                ready = true;
            }
            if let Some(DisplayEvent::Frame(frame)) = receiver.poll().unwrap() {
                assert_eq!(frame.bytes, bytes);
                assert!(frame.keyframe);
                assert!(
                    sent_at.unwrap().elapsed() < Duration::from_millis(400),
                    "Per-packet app sleeps delayed a complete LAN frame"
                );
                return;
            }
            if ready && !sent {
                sender.awaiting_keyframe = true;
                assert!(!sender
                    .write(Frame {
                        timestamp: 983_333_333,
                        keyframe: false,
                        bytes: vec![0, 0, 0, 1, 0x41, 0x88],
                    })
                    .unwrap());
                assert!(sender
                    .write(Frame {
                        timestamp: 1_000_000_000,
                        keyframe: true,
                        bytes: bytes.clone(),
                    })
                    .unwrap());
                sent = true;
                sent_at = Some(Instant::now());
            }
            assert!(
                Instant::now() < deadline,
                "Encrypted media round trip timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
