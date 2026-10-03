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
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_FRAME,
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
            Err(e) => return Err(TransportFailure(e).into()),
        }
    }
    Ok(())
}

pub fn read_frame_until(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>> {
    let mut size = [0; 2];
    read_exact_until(stream, &mut size, deadline)?;
    let size = u16::from_be_bytes(size) as usize;
    ensure!(size > 0 && size <= MAX_FRAME, "invalid frame length");
    let mut bytes = vec![0; size];
    read_exact_until(stream, &mut bytes, deadline)?;
    Ok(bytes)
}

pub struct Channel {
    stream: TcpStream,
    noise: TransportState,
    read_budget: Duration,
    protocol: u16,
    capabilities: Vec<String>,
}

impl Channel {
    pub fn new(stream: TcpStream, noise: TransportState) -> Self {
        Self {
            stream,
            noise,
            read_budget: Duration::from_secs(10),
            protocol: 0,
            capabilities: Vec::new(),
        }
    }
    pub(crate) fn set_protocol(&mut self, protocol: u16, capabilities: Vec<String>) {
        self.protocol = protocol;
        self.capabilities = capabilities;
    }
    pub fn require_capability(&self, capability: &str) -> Result<()> {
        ensure!(
            self.protocol > 0 && self.capabilities.iter().any(|c| c == capability),
            crate::error::EngineError::FeatureUnavailable
        );
        Ok(())
    }
    pub fn send<T: Serialize>(&mut self, message: &T) -> Result<()> {
        let plaintext = serde_json::to_vec(message)?;
        ensure!(plaintext.len() + 16 <= MAX_FRAME, "message too large");
        let mut ciphertext = [0; MAX_FRAME];
        let len = self.noise.write_message(&plaintext, &mut ciphertext)?;
        write_frame(&mut self.stream, &ciphertext[..len])
    }
    pub fn receive<T: DeserializeOwned>(&mut self) -> Result<T> {
        let ciphertext = read_frame_until(&mut self.stream, Instant::now() + self.read_budget)?;
        let mut plaintext = [0; MAX_FRAME];
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
    pub fn active_timeout(&mut self) -> Result<()> {
        self.read_budget = Duration::from_secs(10);
        Ok(())
    }
}
