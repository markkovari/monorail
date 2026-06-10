//! CSAFE framing: flags, byte stuffing, XOR checksum.
//!
//! A standard CSAFE frame is:
//!
//! ```text
//! 0xF1 <stuffed: contents + checksum> 0xF2
//! ```
//!
//! where `checksum` is the XOR of the unstuffed contents, and the byte
//! values 0xF0–0xF3 inside the frame body are escaped as `0xF3 (value & 0x03)`.
//! Concept2 PM-proprietary commands ride inside standard CSAFE frames
//! wrapped in `CSAFE_SETUSERCFG1_CMD`; command enums land here as the
//! protocol surface grows.

use thiserror::Error;

/// Standard frame start flag.
pub const START_FLAG: u8 = 0xF1;
/// Extended frame start flag (carries source/destination addresses).
pub const EXT_START_FLAG: u8 = 0xF0;
/// Frame stop flag.
pub const STOP_FLAG: u8 = 0xF2;
/// Byte-stuffing escape flag.
pub const STUFF_FLAG: u8 = 0xF3;

/// Short (status request) command: get machine status.
pub const CSAFE_GETSTATUS_CMD: u8 = 0x80;
/// Wrapper for Concept2 PM-proprietary commands.
pub const CSAFE_SETUSERCFG1_CMD: u8 = 0x1A;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame too short ({0} bytes)")]
    TooShort(usize),
    #[error("missing start flag, got {0:#04x}")]
    BadStart(u8),
    #[error("missing stop flag, got {0:#04x}")]
    BadStop(u8),
    #[error("invalid stuffed byte {0:#04x} after stuff flag")]
    BadStuffing(u8),
    #[error("checksum mismatch: computed {computed:#04x}, frame carries {carried:#04x}")]
    Checksum { computed: u8, carried: u8 },
    #[error("empty frame body")]
    Empty,
}

fn checksum(contents: &[u8]) -> u8 {
    contents.iter().fold(0u8, |acc, b| acc ^ b)
}

fn stuff_into(out: &mut Vec<u8>, byte: u8) {
    if (EXT_START_FLAG..=STUFF_FLAG).contains(&byte) {
        out.push(STUFF_FLAG);
        out.push(byte & 0x03);
    } else {
        out.push(byte);
    }
}

/// Build a standard CSAFE frame around `contents` (commands + arguments).
pub fn build_frame(contents: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(contents.len() + 4);
    out.push(START_FLAG);
    for &b in contents {
        stuff_into(&mut out, b);
    }
    stuff_into(&mut out, checksum(contents));
    out.push(STOP_FLAG);
    out
}

/// Parse a standard CSAFE frame, returning the unstuffed, checksum-verified
/// contents.
pub fn parse_frame(raw: &[u8]) -> Result<Vec<u8>, FrameError> {
    if raw.len() < 3 {
        return Err(FrameError::TooShort(raw.len()));
    }
    if raw[0] != START_FLAG {
        return Err(FrameError::BadStart(raw[0]));
    }
    let last = *raw.last().expect("len checked");
    if last != STOP_FLAG {
        return Err(FrameError::BadStop(last));
    }

    let mut body = Vec::with_capacity(raw.len() - 2);
    let mut bytes = raw[1..raw.len() - 1].iter();
    while let Some(&b) = bytes.next() {
        if b == STUFF_FLAG {
            match bytes.next() {
                Some(&v @ 0x00..=0x03) => body.push(EXT_START_FLAG + v),
                Some(&v) => return Err(FrameError::BadStuffing(v)),
                None => return Err(FrameError::TooShort(raw.len())),
            }
        } else {
            body.push(b);
        }
    }

    let carried = body.pop().ok_or(FrameError::Empty)?;
    let computed = checksum(&body);
    if computed != carried {
        return Err(FrameError::Checksum { computed, carried });
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_status_golden_vector() {
        // Single-command frame: checksum of [0x80] is 0x80.
        assert_eq!(
            build_frame(&[CSAFE_GETSTATUS_CMD]),
            vec![0xF1, 0x80, 0x80, 0xF2]
        );
    }

    #[test]
    fn flag_bytes_are_stuffed() {
        // 0xF0..=0xF3 in contents must be escaped as 0xF3 0x00..=0x03.
        let frame = build_frame(&[0xF0, 0xF1, 0xF2, 0xF3]);
        assert_eq!(
            frame,
            vec![
                0xF1, // start
                0xF3, 0x00, // 0xF0
                0xF3, 0x01, // 0xF1
                0xF3, 0x02, // 0xF2
                0xF3, 0x03, // 0xF3
                0x00, // checksum: F0^F1^F2^F3 = 0x00
                0xF2, // stop
            ]
        );
    }

    #[test]
    fn stuffed_checksum_round_trips() {
        // Contents whose checksum itself lands in the flag range.
        let contents = [0x71, 0x81]; // 0x71 ^ 0x81 = 0xF0 → stuffed checksum
        let frame = build_frame(&contents);
        assert_eq!(parse_frame(&frame).unwrap(), contents);
    }

    #[test]
    fn round_trip_arbitrary_contents() {
        let contents: Vec<u8> = (0u8..=255).collect();
        assert_eq!(parse_frame(&build_frame(&contents)).unwrap(), contents);
    }

    #[test]
    fn corrupted_byte_fails_checksum() {
        let mut frame = build_frame(&[CSAFE_GETSTATUS_CMD, 0x01, 0x02]);
        frame[1] ^= 0x10;
        assert!(matches!(
            parse_frame(&frame),
            Err(FrameError::Checksum { .. })
        ));
    }

    #[test]
    fn rejects_malformed_frames() {
        assert_eq!(parse_frame(&[0xF1]), Err(FrameError::TooShort(1)));
        assert_eq!(
            parse_frame(&[0x00, 0x80, 0x80, 0xF2]),
            Err(FrameError::BadStart(0x00))
        );
        assert_eq!(
            parse_frame(&[0xF1, 0x80, 0x80, 0x00]),
            Err(FrameError::BadStop(0x00))
        );
        assert_eq!(
            parse_frame(&[0xF1, 0xF3, 0x07, 0xF2]),
            Err(FrameError::BadStuffing(0x07))
        );
    }
}
