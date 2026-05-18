// SPDX-License-Identifier: AGPL-3.0-or-later
//! MongoDB OP_MSG frame codec built on `cave_kernel::codec::FrameCodec`.
//!
//! OP_MSG framing on the wire is `[4B LE total_len][4B request_id][4B
//! response_to][4B opcode][payload]`. The first 4 bytes carry the total
//! length of the entire frame including the 16-byte header. This codec
//! handles only the framing (length-prefix split); per-section parsing
//! of the payload (body, document sequence) lives in
//! [`super::wire::decode_op_msg`].
//!
//! **Bug history.** Prior to sweep-004 the server's `handle_connection`
//! used a single `socket.read(&mut [u8; 16384])` call per iteration and
//! called `decode_op_msg` on whatever came back. Frames larger than
//! 16 KiB were truncated; frames split across two `read` calls were
//! assembled out of order. This codec routes the I/O through a
//! `BytesMut` accumulator + length-aware decoder, which is the correct
//! shape for any TCP-framed protocol.

use bytes::{Bytes, BytesMut};
use cave_kernel::codec::{FrameCodec, FrameError, LengthSpec, try_read_length_prefixed};

/// Maximum OP_MSG frame size. MongoDB's official limit is 48 MiB; we
/// match it. Configurable via [`OpMsgCodec::with_limit`].
pub const DEFAULT_MAX_FRAME_SIZE: usize = 48 * 1024 * 1024;

/// One full OP_MSG frame on the wire — header (16 bytes) + payload.
/// Caller passes `frame.bytes` to [`super::wire::decode_op_msg`] to
/// extract `request_id` and the section list.
#[derive(Debug, Clone)]
pub struct RawWireFrame {
    pub bytes: Bytes,
}

#[derive(Debug)]
pub struct OpMsgCodec {
    max_frame_size: usize,
}

impl OpMsgCodec {
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_MAX_FRAME_SIZE)
    }

    pub fn with_limit(max_frame_size: usize) -> Self {
        Self { max_frame_size }
    }
}

impl Default for OpMsgCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCodec<RawWireFrame> for OpMsgCodec {
    fn encode(&mut self, frame: RawWireFrame, buf: &mut BytesMut) -> Result<(), FrameError> {
        // Encoder is currently a passthrough: the caller composes frames
        // via `super::wire::encode_op_msg`, which already produces a
        // wire-ready `Vec<u8>` including the 16-byte header. The codec
        // exposes `encode` for trait completeness and to centralize the
        // size check before bytes hit the socket.
        if frame.bytes.len() > self.max_frame_size {
            return Err(FrameError::Limit { actual: frame.bytes.len(), max: self.max_frame_size });
        }
        if frame.bytes.len() < 16 {
            return Err(FrameError::invalid(format!(
                "encoded OP_MSG frame is {} bytes; minimum is 16 (header)",
                frame.bytes.len()
            )));
        }
        buf.extend_from_slice(&frame.bytes);
        Ok(())
    }

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<RawWireFrame>, FrameError> {
        let spec = LengthSpec::mongo_op_msg(self.max_frame_size);
        match try_read_length_prefixed(buf, &spec)? {
            Some(bytes) => {
                if bytes.len() < 16 {
                    return Err(FrameError::invalid(format!(
                        "OP_MSG frame {} bytes is shorter than header (16)",
                        bytes.len()
                    )));
                }
                Ok(Some(RawWireFrame { bytes }))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic OP_MSG frame with the given payload. The
    /// payload bytes are opaque to the codec (which only frames).
    fn synth_op_msg(payload: &[u8], request_id: i32) -> BytesMut {
        let mut buf = BytesMut::new();
        let total_len = (16 + payload.len()) as u32;
        buf.extend_from_slice(&total_len.to_le_bytes());
        buf.extend_from_slice(&request_id.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes()); // response_to
        buf.extend_from_slice(&2013i32.to_le_bytes()); // opcode = OP_MSG
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn decode_single_frame_clean() {
        let mut buf = synth_op_msg(b"\x00hello-payload-bytes", 42);
        let mut codec = OpMsgCodec::new();
        let frame = codec.decode(&mut buf).unwrap().unwrap();
        // First 4 bytes of the frame is total length.
        let length = u32::from_le_bytes([
            frame.bytes[0], frame.bytes[1], frame.bytes[2], frame.bytes[3],
        ]);
        assert_eq!(length as usize, frame.bytes.len());
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_split_across_three_reads_assembles() {
        // Simulate a peer flushing the same frame in three TCP segments.
        let full = synth_op_msg(&vec![0xAB; 4096], 7);
        let mut buf = BytesMut::new();
        let mut codec = OpMsgCodec::new();

        buf.extend_from_slice(&full[..2]);
        assert!(codec.decode(&mut buf).unwrap().is_none());
        buf.extend_from_slice(&full[2..1500]);
        assert!(codec.decode(&mut buf).unwrap().is_none());
        buf.extend_from_slice(&full[1500..]);
        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.bytes.len(), full.len());
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_rejects_frame_above_limit() {
        // Announce a 100 MiB frame against a 1 MiB limit.
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&(100u32 * 1024 * 1024).to_le_bytes());
        buf.extend_from_slice(&[0u8; 12]); // remaining header
        let mut codec = OpMsgCodec::with_limit(1024 * 1024);
        match codec.decode(&mut buf) {
            Err(FrameError::Limit { .. }) => {}
            other => panic!("expected Limit, got {other:?}"),
        }
    }

    #[test]
    fn decode_two_pipelined_frames_in_one_buffer() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&synth_op_msg(b"a", 1));
        buf.extend_from_slice(&synth_op_msg(b"bb", 2));
        let mut codec = OpMsgCodec::new();
        let f1 = codec.decode(&mut buf).unwrap().unwrap();
        let f2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1.bytes.len(), 17); // 16 + 1
        assert_eq!(f2.bytes.len(), 18); // 16 + 2
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_rejects_too_short_frame() {
        // total_len announces 8 bytes total — below the 16-byte header.
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(&[0u8; 4]);
        let mut codec = OpMsgCodec::new();
        // The length-prefix helper itself rejects: announced frame
        // shorter than the header it is meant to span.
        // (Helper considers header = `header_offset + length_size = 0 + 4 = 4`,
        // so 8 ≥ 4 passes there, but the codec then notices the frame
        // is < 16 and rejects.)
        // The buffer only has 8 bytes, so try_read returns None first.
        // Add the rest:
        buf.extend_from_slice(&[0u8; 4]);
        match codec.decode(&mut buf) {
            Err(FrameError::Invalid(msg)) => assert!(msg.contains("shorter than header")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn encode_passthrough_appends_bytes() {
        let mut codec = OpMsgCodec::new();
        let mut buf = BytesMut::new();
        let synth = synth_op_msg(b"payload", 9);
        let frame = RawWireFrame { bytes: synth.freeze() };
        codec.encode(frame, &mut buf).unwrap();
        assert_eq!(buf.len(), 16 + b"payload".len());
    }
}
