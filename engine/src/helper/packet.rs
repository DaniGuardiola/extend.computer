//! Display helper framing, `[kind u8][length u32 BE][payload]`, on a helper's
//! stdout and on a viewer's stdin.
use crate::display::Frame;
use anyhow::{bail, ensure, Result};
use std::io::{self, Read, Write};

pub const MAX_PACKET: usize = 16 * 1024 * 1024;
pub const MAX_VIEWER_INPUT: usize = crate::display::input::MAX_MESSAGE_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// JSON [`super::display::Startup`].
    Startup = 0,
    /// An encoded frame; see [`frame`].
    Frame = 1,
    /// JSON control: [`super::display::HelperMessage`] from a helper,
    /// [`super::display::ViewerCommand`] to a viewer.
    Control = 2,
    /// JSON [`super::display::CursorPacket`].
    Cursor = 3,
    /// JSON [`super::display::NativeEvent`] from a viewer.
    ViewerInput = 4,
}
impl TryFrom<u8> for Kind {
    type Error = anyhow::Error;
    fn try_from(kind: u8) -> Result<Self> {
        Ok(match kind {
            0 => Self::Startup,
            1 => Self::Frame,
            2 => Self::Control,
            3 => Self::Cursor,
            4 => Self::ViewerInput,
            _ => bail!("Unexpected display helper packet"),
        })
    }
}

/// The next packet, or `None` on a clean end of stream. A truncated packet is
/// an `UnexpectedEof` I/O error.
pub fn read_packet<R: Read>(input: &mut R) -> Result<Option<(Kind, Vec<u8>)>> {
    let mut header = [0u8; 5];
    let mut filled = 0;
    while filled < header.len() {
        match input.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
    ensure!(length <= MAX_PACKET, "Invalid helper packet length");
    let kind = Kind::try_from(header[0])?;
    if kind == Kind::ViewerInput {
        ensure!(
            length > 0 && length <= MAX_VIEWER_INPUT,
            "Invalid native viewer input packet length"
        );
    }
    let mut payload = vec![0; length];
    input.read_exact(&mut payload)?;
    Ok(Some((kind, payload)))
}

pub fn write_packet<W: Write>(output: &mut W, kind: Kind, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut header = [kind as u8, 0, 0, 0, 0];
    header[1..].copy_from_slice(&length.to_be_bytes());
    output.write_all(&header)?;
    output.write_all(payload)
}

/// Kind 1 payload: `[timestamp ns u64 BE][keyframe 0|1][Annex B]`.
pub mod frame {
    use super::*;

    pub fn encode(frame: &Frame) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(frame.bytes.len() + 9);
        bytes.extend_from_slice(&frame.timestamp.to_be_bytes());
        bytes.push(u8::from(frame.keyframe));
        bytes.extend_from_slice(&frame.bytes);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame> {
        ensure!(bytes.len() > 9 && bytes[8] < 2, "Invalid helper frame");
        Ok(Frame {
            timestamp: u64::from_be_bytes(bytes[..8].try_into().unwrap()),
            keyframe: bytes[8] == 1,
            bytes: bytes[9..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![kind];
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn reads_written_packets_until_a_clean_end() {
        let mut stream = Vec::new();
        write_packet(&mut stream, Kind::Control, br#"{"type":"keyframe"}"#).unwrap();
        write_packet(&mut stream, Kind::Startup, br#"{"ready":true}"#).unwrap();
        assert_eq!(&stream[..5], &[2, 0, 0, 0, 19]);
        let mut input = stream.as_slice();
        assert_eq!(
            read_packet(&mut input).unwrap(),
            Some((Kind::Control, br#"{"type":"keyframe"}"#.to_vec()))
        );
        assert_eq!(read_packet(&mut input).unwrap().unwrap().0, Kind::Startup);
        assert_eq!(read_packet(&mut input).unwrap(), None);
    }

    #[test]
    fn truncation_is_an_unexpected_eof_and_limits_hold_before_reading() {
        for truncated in [&packet(2, b"{}")[..3], &packet(2, b"{}")[..6]] {
            let error = read_packet(&mut &truncated[..]).unwrap_err();
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
        let mut oversized = vec![1];
        oversized.extend_from_slice(&(MAX_PACKET as u32 + 1).to_be_bytes());
        assert!(read_packet(&mut oversized.as_slice()).is_err());
        assert!(read_packet(&mut packet(5, b"{}").as_slice()).is_err());
        assert!(read_packet(&mut packet(4, b"").as_slice()).is_err());
        let largest = vec![b' '; MAX_VIEWER_INPUT];
        assert!(read_packet(&mut packet(4, &largest).as_slice()).is_ok());
        let too_large = vec![b' '; MAX_VIEWER_INPUT + 1];
        assert!(read_packet(&mut packet(4, &too_large).as_slice()).is_err());
    }

    #[test]
    fn frames_round_trip_and_reject_missing_data_or_bad_flags() {
        let frame = Frame {
            timestamp: 0x0102_0304_0506_0708,
            keyframe: true,
            bytes: vec![0, 0, 0, 1, 0x67],
        };
        let bytes = frame::encode(&frame);
        assert_eq!(&bytes[..9], &[1, 2, 3, 4, 5, 6, 7, 8, 1]);
        let decoded = frame::decode(&bytes).unwrap();
        assert_eq!(
            (decoded.timestamp, decoded.keyframe, decoded.bytes),
            (frame.timestamp, true, frame.bytes)
        );
        assert!(frame::decode(&bytes[..9]).is_err());
        let mut flag = bytes;
        flag[8] = 2;
        assert!(frame::decode(&flag).is_err());
    }
}
