use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use snow::TransportState;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Distinguish socket I/O from protocol/cryptographic/policy failures. macOS
/// can return EINVAL from timeout configuration after a socket is shut down.
#[derive(Debug)]
pub struct TransportFailure(pub std::io::Error);
impl std::fmt::Display for TransportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transport: {}", self.0)
    }
}
impl std::error::Error for TransportFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

pub const MAX_FRAME: usize = 4096;

pub fn configure(stream: &TcpStream) -> Result<()> {
    stream.set_nodelay(true).map_err(TransportFailure)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(TransportFailure)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(TransportFailure)?;
    Ok(())
}

pub fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut size = [0; 2];
    reader.read_exact(&mut size)?;
    let size = u16::from_be_bytes(size) as usize;
    ensure!(size > 0 && size <= MAX_FRAME, "invalid frame length");
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Result<()> {
    write_frame_limited(writer, bytes, MAX_FRAME)
}

fn write_frame_limited(writer: &mut impl Write, bytes: &[u8], limit: usize) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= limit,
        "invalid frame length"
    );
    writer
        .write_all(&(bytes.len() as u16).to_be_bytes())
        .map_err(TransportFailure)?;
    writer.write_all(bytes).map_err(TransportFailure)?;
    writer.flush().map_err(TransportFailure)?;
    Ok(())
}

/// Absolute deadline, so a peer cannot extend a read forever by dribbling bytes.
pub fn read_exact_until(stream: &mut TcpStream, bytes: &mut [u8], deadline: Instant) -> Result<()> {
    let mut filled = 0;
    while filled < bytes.len() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| anyhow::anyhow!("read deadline exceeded"))?;
        stream
            .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))
            .map_err(TransportFailure)?;
        match stream.read(&mut bytes[filled..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer disconnected",
                )
                .into())
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // A kernel timeout can return before the absolute deadline.
                // Keep the bytes already read and finish this same frame rather
                // than abandoning framing/Noise state after a transient EAGAIN.
                if Instant::now() >= deadline {
                    return Err(TransportFailure(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "read deadline exceeded",
                    ))
                    .into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(TransportFailure(e).into()),
        }
    }
    Ok(())
}

pub fn read_frame_until(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>> {
    read_frame_until_limited(stream, deadline, MAX_FRAME)
}

fn read_frame_until_limited(
    stream: &mut TcpStream,
    deadline: Instant,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut size = [0; 2];
    read_exact_until(stream, &mut size, deadline)?;
    let size = u16::from_be_bytes(size) as usize;
    ensure!(size > 0 && size <= limit, "invalid frame length");
    let mut bytes = vec![0; size];
    read_exact_until(stream, &mut bytes, deadline)?;
    Ok(bytes)
}

pub struct Channel {
    stream: TcpStream,
    noise: TransportState,
    read_budget: Duration,
    frame_limit: usize,
    protocol: u16,
    capabilities: Vec<String>,
    remote_device_info: Option<crate::device_info::DeviceInfo>,
    remote_link: Option<crate::link::Link>,
}

impl Channel {
    pub fn new(stream: TcpStream, noise: TransportState) -> Self {
        Self {
            stream,
            noise,
            read_budget: Duration::from_secs(10),
            frame_limit: MAX_FRAME,
            protocol: 0,
            capabilities: Vec::new(),
            remote_device_info: None,
            remote_link: None,
        }
    }
    pub(crate) fn set_protocol(&mut self, protocol: u16, capabilities: Vec<String>) {
        self.protocol = protocol;
        self.capabilities = capabilities;
    }
    pub(crate) fn set_remote_device_info(&mut self, info: Option<crate::device_info::DeviceInfo>) {
        self.remote_device_info = info;
    }
    /// Remote presentation metadata from the authenticated compatibility exchange.
    pub fn remote_device_info(&self) -> Option<&crate::device_info::DeviceInfo> {
        self.remote_device_info.as_ref()
    }
    pub(crate) fn local_link(&self) -> Option<crate::link::Link> {
        crate::link::of(&self.stream)
    }
    pub(crate) fn set_remote_link(&mut self, link: Option<crate::link::Link>) {
        self.remote_link = link;
    }
    /// The peer's own physical link for this connection, if it reported one.
    pub fn remote_link(&self) -> Option<crate::link::Link> {
        self.remote_link
    }
    pub fn require_capability(&self, capability: &str) -> Result<()> {
        ensure!(
            self.supports_capability(capability),
            crate::error::EngineError::FeatureUnavailable
        );
        Ok(())
    }
    /// Only capabilities agreed inside the authenticated compatibility exchange.
    pub fn supports_capability(&self, capability: &str) -> bool {
        self.protocol > 0 && self.capabilities.iter().any(|c| c == capability)
    }
    /// SDP with media and data channels needs a larger authenticated envelope.
    pub(crate) fn display_signaling(&mut self) {
        self.frame_limit = 32 * 1024;
    }
    pub fn send<T: Serialize>(&mut self, message: &T) -> Result<()> {
        let plaintext = serde_json::to_vec(message)?;
        ensure!(
            plaintext.len() + 16 <= self.frame_limit,
            "message too large"
        );
        let mut ciphertext = vec![0; self.frame_limit];
        let len = self.noise.write_message(&plaintext, &mut ciphertext)?;
        write_frame_limited(&mut self.stream, &ciphertext[..len], self.frame_limit)
    }
    pub fn receive<T: DeserializeOwned>(&mut self) -> Result<T> {
        let ciphertext = read_frame_until_limited(
            &mut self.stream,
            Instant::now() + self.read_budget,
            self.frame_limit,
        )?;
        let mut plaintext = vec![0; self.frame_limit];
        let len = self
            .noise
            .read_message(&ciphertext, &mut plaintext)
            .context(crate::error::EngineError::AuthenticationFailed)?;
        Ok(serde_json::from_slice(&plaintext[..len])?)
    }
    pub fn consent_timeout(&mut self) -> Result<()> {
        self.read_budget = Duration::from_secs(120);
        Ok(())
    }
    /// Half-close, then discard input until the peer closes or the limit passes.
    pub fn close_after_peer(&mut self, limit: Duration) {
        let _ = self.stream.shutdown(std::net::Shutdown::Write);
        let deadline = Instant::now() + limit;
        let mut discard = [0; 4096];
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if self.stream.set_read_timeout(Some(remaining)).is_err() {
                return;
            }
            match self.stream.read(&mut discard) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    }
    pub fn active_timeout(&mut self) -> Result<()> {
        self.read_budget = Duration::from_secs(10);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_would_block_preserves_partial_frame_until_absolute_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut writer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut reader = listener.accept().unwrap().0;
        reader.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let mut bytes = [0; 4];
            read_exact_until(
                &mut reader,
                &mut bytes,
                Instant::now() + Duration::from_millis(500),
            )
            .unwrap();
            bytes
        });
        writer.write_all(&[1, 2]).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        writer.write_all(&[3, 4]).unwrap();
        assert_eq!(server.join().unwrap(), [1, 2, 3, 4]);
    }
    #[test]
    fn repeated_would_block_does_not_extend_absolute_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let _writer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut reader = listener.accept().unwrap().0;
        reader.set_nonblocking(true).unwrap();
        let start = Instant::now();
        assert!(
            read_exact_until(&mut reader, &mut [0; 4], start + Duration::from_millis(40)).is_err()
        );
        assert!(start.elapsed() < Duration::from_millis(200));
    }
}
