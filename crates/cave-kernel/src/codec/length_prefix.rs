// SPDX-License-Identifier: AGPL-3.0-or-later
//! Length-prefix framing helper, parameterized over the small set of
//! choices that vary between PostgreSQL v3 and MongoDB OP_MSG.
//!
//! What's shared:
//! - Both protocols carry a 4-byte length field in the frame header.
//! - Both require the I/O loop to handle partial reads — the helper
//!   returns `Ok(None)` when the buffer has not yet received a full
//!   frame, leaving the buffer untouched.
//! - Both can be DoS'd by a malicious peer announcing a huge length.
//!   The helper enforces a max-frame ceiling **before** any allocation.
//!
//! What varies:
//! - Endianness: PostgreSQL uses big-endian; OP_MSG is little-endian.
//! - Whether the length field includes itself plus the payload (PG:
//!   `[1B type][4B BE length][payload]`, length covers itself + payload
//!   but not the type byte) or covers the entire frame from byte 0
//!   (OP_MSG: `[4B LE length][12B header_rest][payload]`, length covers
//!   the whole frame including itself).
//! - Number of bytes that sit in front of the length field
//!   (`header_offset`): 1 for PG (the type byte), 0 for OP_MSG.
//!
//! RESP3 cannot use this helper — its outer envelope is line-oriented,
//! not length-prefixed — and that is the honest reason the helper sits
//! at 2/3 reuse instead of 3/3.

use bytes::{Bytes, BytesMut};

use super::FrameError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Big,
    Little,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthIncludes {
    /// Length value covers the length field itself + the payload that
    /// follows it, but **not** any bytes ahead of the length field.
    /// Used by PostgreSQL v3 frontend/backend messages.
    SelfAndPayload,
    /// Length value covers the entire frame from byte 0 (including any
    /// `header_offset` bytes ahead of the length field).
    /// Used by MongoDB OP_MSG.
    Total,
}

#[derive(Debug, Clone, Copy)]
pub struct LengthSpec {
    /// Bytes that precede the length field. PG: 1 (type byte). Mongo: 0.
    pub header_offset: usize,
    /// Length field width. Both protocols use 4.
    pub length_size: usize,
    pub endian: Endian,
    pub length_includes: LengthIncludes,
    /// Maximum total frame size (including header). Defends against a
    /// peer that announces a length wide enough to OOM the process.
    pub max_frame_size: usize,
}

impl LengthSpec {
    /// PostgreSQL v3 message frame: `[1B type][4B BE length][payload]`,
    /// length covers itself plus payload. Caller picks `max_frame_size`.
    pub fn postgres_v3(max_frame_size: usize) -> Self {
        Self {
            header_offset: 1,
            length_size: 4,
            endian: Endian::Big,
            length_includes: LengthIncludes::SelfAndPayload,
            max_frame_size,
        }
    }

    /// PostgreSQL v3 startup message: `[4B BE length][payload]` — no
    /// type byte. Used for the first frame of a connection only.
    pub fn postgres_startup(max_frame_size: usize) -> Self {
        Self {
            header_offset: 0,
            length_size: 4,
            endian: Endian::Big,
            length_includes: LengthIncludes::SelfAndPayload,
            max_frame_size,
        }
    }

    /// MongoDB OP_MSG frame: `[4B LE total_len][...][payload]`,
    /// total_len covers the whole frame.
    pub fn mongo_op_msg(max_frame_size: usize) -> Self {
        Self {
            header_offset: 0,
            length_size: 4,
            endian: Endian::Little,
            length_includes: LengthIncludes::Total,
            max_frame_size,
        }
    }

    fn total_frame_len(&self, length_value: usize) -> usize {
        match self.length_includes {
            LengthIncludes::SelfAndPayload => self.header_offset + length_value,
            LengthIncludes::Total => length_value,
        }
    }
}

fn read_length(bytes: &[u8], spec: &LengthSpec) -> Result<usize, FrameError> {
    if bytes.len() < spec.length_size {
        return Err(FrameError::Incomplete { needed: spec.length_size - bytes.len() });
    }
    let value = match (spec.length_size, spec.endian) {
        (4, Endian::Big) => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
        (4, Endian::Little) => {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize
        }
        (n, _) => {
            return Err(FrameError::invalid(format!(
                "unsupported length_size {n}; only 4 is implemented"
            )));
        }
    };
    Ok(value)
}

/// Try to consume one length-prefixed frame off `src`.
///
/// On success, the entire frame (header + length field + payload) is
/// split off `src` and returned as a [`Bytes`]. The caller is
/// responsible for parsing inside that frame.
///
/// Returns:
/// - `Ok(Some(frame))` — a complete frame; `src` advanced past it.
/// - `Ok(None)` — buffer too short to know the length yet, or buffer
///   too short to contain the announced frame; `src` is **not**
///   modified.
/// - `Err(FrameError::Limit)` — announced length exceeds `max_frame_size`.
/// - `Err(FrameError::Invalid)` — announced length is shorter than the
///   header itself (a malformed peer).
pub fn try_read_length_prefixed(
    src: &mut BytesMut,
    spec: &LengthSpec,
) -> Result<Option<Bytes>, FrameError> {
    let header_total = spec.header_offset + spec.length_size;
    if src.len() < header_total {
        return Ok(None);
    }

    let length_value = read_length(&src[spec.header_offset..], spec)?;
    let frame_len = spec.total_frame_len(length_value);

    // Reject announced frames that don't even cover their own header —
    // this protects us from `frame_len < header_total` underflow further
    // down and from peers announcing 0-byte frames.
    if frame_len < header_total {
        return Err(FrameError::invalid(format!(
            "announced frame length {length_value} is shorter than header ({header_total})"
        )));
    }
    if frame_len > spec.max_frame_size {
        return Err(FrameError::Limit { actual: frame_len, max: spec.max_frame_size });
    }
    if src.len() < frame_len {
        return Ok(None);
    }

    let frame = src.split_to(frame_len).freeze();
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg_frame(type_byte: u8, payload: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[type_byte]);
        let length_value = (4 + payload.len()) as u32;
        buf.extend_from_slice(&length_value.to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    fn mongo_frame(payload: &[u8]) -> BytesMut {
        // Mongo: [4B LE total_len][12B header_rest][payload]
        let mut buf = BytesMut::new();
        let total_len = (16 + payload.len()) as u32;
        buf.extend_from_slice(&total_len.to_le_bytes());
        buf.extend_from_slice(&[0u8; 12]); // request_id + response_to + opcode placeholders
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn pg_frame_decodes_clean() {
        let mut buf = pg_frame(b'Q', b"SELECT 1\0");
        let spec = LengthSpec::postgres_v3(8 * 1024 * 1024);
        let frame = try_read_length_prefixed(&mut buf, &spec).unwrap().unwrap();
        // Frame includes the 1-byte type + 4-byte length + 9-byte payload = 14 bytes.
        assert_eq!(frame.len(), 14);
        assert_eq!(frame[0], b'Q');
        assert!(buf.is_empty());
    }

    #[test]
    fn pg_partial_header_returns_none() {
        let mut buf = BytesMut::from(&[b'Q', 0x00, 0x00][..]); // only 3 bytes
        let spec = LengthSpec::postgres_v3(64);
        assert!(try_read_length_prefixed(&mut buf, &spec).unwrap().is_none());
        assert_eq!(buf.len(), 3, "buffer must not be modified on Ok(None)");
    }

    #[test]
    fn pg_partial_payload_returns_none() {
        // Announce 9 bytes of payload but only deliver 3.
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[b'Q']);
        buf.extend_from_slice(&13u32.to_be_bytes()); // length = 4 + 9
        buf.extend_from_slice(b"SEL");
        let snapshot = buf.clone();
        let spec = LengthSpec::postgres_v3(64);
        assert!(try_read_length_prefixed(&mut buf, &spec).unwrap().is_none());
        assert_eq!(buf, snapshot, "buffer must not be modified on Ok(None)");
    }

    #[test]
    fn pg_two_frames_in_one_buffer() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&pg_frame(b'Q', b"a\0"));
        buf.extend_from_slice(&pg_frame(b'S', b""));
        let spec = LengthSpec::postgres_v3(64);
        let f1 = try_read_length_prefixed(&mut buf, &spec).unwrap().unwrap();
        let f2 = try_read_length_prefixed(&mut buf, &spec).unwrap().unwrap();
        assert_eq!(f1[0], b'Q');
        assert_eq!(f2[0], b'S');
        assert!(buf.is_empty());
    }

    #[test]
    fn mongo_frame_decodes_clean() {
        let mut buf = mongo_frame(&[0xAA; 32]);
        let spec = LengthSpec::mongo_op_msg(8 * 1024 * 1024);
        let frame = try_read_length_prefixed(&mut buf, &spec).unwrap().unwrap();
        assert_eq!(frame.len(), 16 + 32);
        let len_bytes = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
        assert_eq!(len_bytes as usize, frame.len());
        assert!(buf.is_empty());
    }

    #[test]
    fn mongo_partial_header_returns_none() {
        let mut buf = BytesMut::from(&[0x10, 0x00][..]); // only 2 bytes — length itself incomplete
        let spec = LengthSpec::mongo_op_msg(8 * 1024 * 1024);
        assert!(try_read_length_prefixed(&mut buf, &spec).unwrap().is_none());
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn mongo_split_frames_assemble() {
        // Simulate the read loop: deliver bytes in 3 chunks.
        let full = mongo_frame(&[0x55; 100]);
        let mut buf = BytesMut::new();
        let spec = LengthSpec::mongo_op_msg(8 * 1024 * 1024);

        buf.extend_from_slice(&full[..3]);
        assert!(try_read_length_prefixed(&mut buf, &spec).unwrap().is_none());

        buf.extend_from_slice(&full[3..50]);
        assert!(try_read_length_prefixed(&mut buf, &spec).unwrap().is_none());

        buf.extend_from_slice(&full[50..]);
        let frame = try_read_length_prefixed(&mut buf, &spec).unwrap().unwrap();
        assert_eq!(frame.len(), full.len());
        assert!(buf.is_empty());
    }

    #[test]
    fn frame_exceeding_max_returns_limit() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[b'Q']);
        buf.extend_from_slice(&1_000_000u32.to_be_bytes());
        let spec = LengthSpec::postgres_v3(1024);
        match try_read_length_prefixed(&mut buf, &spec) {
            Err(FrameError::Limit { actual, max: 1024 }) => {
                // Frame_len = header_offset(1) + length_value(1_000_000) = 1_000_001
                assert_eq!(actual, 1_000_001);
            }
            other => panic!("expected Limit, got {:?}", other),
        }
    }

    #[test]
    fn announced_length_below_header_is_invalid() {
        // PG: length must be at least 4 (length covers itself).
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[b'Q']);
        buf.extend_from_slice(&3u32.to_be_bytes()); // length=3 < length_size(4)
        let spec = LengthSpec::postgres_v3(64);
        assert!(matches!(
            try_read_length_prefixed(&mut buf, &spec),
            Err(FrameError::Invalid(_))
        ));
    }

    #[test]
    fn pg_startup_frame_no_type_byte() {
        // Startup message: [4B BE length][payload], length covers itself + payload.
        let payload = b"user\0alice\0\0";
        let length_value = 4 + payload.len();
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&(length_value as u32).to_be_bytes());
        buf.extend_from_slice(payload);
        let spec = LengthSpec::postgres_startup(64 * 1024);
        let frame = try_read_length_prefixed(&mut buf, &spec).unwrap().unwrap();
        assert_eq!(frame.len(), length_value);
        assert!(buf.is_empty());
    }

    #[test]
    fn empty_buffer_returns_none() {
        let mut buf = BytesMut::new();
        let spec = LengthSpec::postgres_v3(64);
        assert!(try_read_length_prefixed(&mut buf, &spec).unwrap().is_none());
    }
}
