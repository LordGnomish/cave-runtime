// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PostgreSQL v3 frame codec built on `cave_kernel::codec::FrameCodec`.
//!
//! PostgreSQL's wire protocol has two framing modes that share the same
//! big-endian 4-byte length-prefix shape:
//!
//! - **Startup phase** — `[4B BE length_inc_self][payload]`. No type
//!   byte. Used for the first frame on a new connection (StartupMessage,
//!   SSLRequest, CancelRequest).
//! - **Regular phase** — `[1B type][4B BE length_inc_self][payload]`.
//!   All post-startup messages.
//!
//! The codec exposes both modes by holding a [`PgPhase`] and switching
//! after the application has handled the startup. The framing primitive
//! itself comes from `cave_kernel::codec::length_prefix` so the codec is
//! a thin wrapper that knows about PG's two phases.
//!
//! Per-message body parsing (e.g. interpreting a `Q` body as a query
//! string, a `B` body as a Bind) lives in [`super::messages`] and is
//! unchanged — this module owns framing only.

use bytes::{Bytes, BytesMut};
use cave_kernel::codec::{
    FrameCodec, FrameError, LengthSpec, try_read_length_prefixed,
};

/// One PostgreSQL frame as carried on the wire.
#[derive(Debug, Clone)]
pub struct PgFrame {
    /// `None` while the codec is in [`PgPhase::Startup`] (the wire frame
    /// has no type byte). `Some(t)` in [`PgPhase::Regular`].
    pub type_byte: Option<u8>,
    /// Payload bytes — type byte and length prefix already stripped.
    pub body: Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgPhase {
    /// First frame of a connection: SSLRequest, CancelRequest, or
    /// StartupMessage. No type byte on the wire.
    Startup,
    /// Post-startup steady state. Every frame begins with a type byte.
    Regular,
}

/// Default ceiling on a single PG frame: 16 MiB. PG's protocol allows
/// up to 2 GiB but real workloads stay far below this; capping protects
/// the process from a malicious peer announcing a huge length.
pub const DEFAULT_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct PgWireCodec {
    phase: PgPhase,
    max_frame_size: usize,
}

impl PgWireCodec {
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_MAX_FRAME_SIZE)
    }

    pub fn with_limit(max_frame_size: usize) -> Self {
        Self { phase: PgPhase::Startup, max_frame_size }
    }

    pub fn phase(&self) -> PgPhase {
        self.phase
    }

    /// Switch the codec into the post-startup phase. Call this after the
    /// caller has handled a real `StartupMessage` (i.e. not an SSLRequest
    /// or a CancelRequest, which must keep the codec in `Startup`).
    pub fn advance_to_regular(&mut self) {
        self.phase = PgPhase::Regular;
    }
}

impl Default for PgWireCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCodec<PgFrame> for PgWireCodec {
    fn encode(&mut self, frame: PgFrame, buf: &mut BytesMut) -> Result<(), FrameError> {
        if let Some(t) = frame.type_byte {
            buf.extend_from_slice(&[t]);
        }
        let length_value = 4 + frame.body.len();
        if length_value > u32::MAX as usize {
            return Err(FrameError::Limit { actual: length_value, max: u32::MAX as usize });
        }
        buf.extend_from_slice(&(length_value as u32).to_be_bytes());
        buf.extend_from_slice(&frame.body);
        Ok(())
    }

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<PgFrame>, FrameError> {
        let spec = match self.phase {
            PgPhase::Startup => LengthSpec::postgres_startup(self.max_frame_size),
            PgPhase::Regular => LengthSpec::postgres_v3(self.max_frame_size),
        };
        let raw = match try_read_length_prefixed(buf, &spec)? {
            Some(b) => b,
            None => return Ok(None),
        };
        let frame = match self.phase {
            PgPhase::Startup => {
                // raw = [4B length][payload]
                PgFrame { type_byte: None, body: raw.slice(4..) }
            }
            PgPhase::Regular => {
                // raw = [1B type][4B length][payload]
                PgFrame { type_byte: Some(raw[0]), body: raw.slice(5..) }
            }
        };
        Ok(Some(frame))
    }
}

/// SSLRequest magic constant: `1234.5679` packed as 4-byte BE int. PG
/// peers send this as the first frame to ask for TLS; cave-rdbms
/// declines (writes `b"N"`) and stays in startup phase.
pub const SSL_REQUEST_CODE: u32 = 80877103;

/// CancelRequest magic constant: `1234.5678`. PG peers open a fresh
/// connection and send this to abort an in-flight query on another
/// connection. Currently a no-op in cave-rdbms.
pub const CANCEL_REQUEST_CODE: u32 = 80877102;

/// Inspect the first 4 bytes of a startup frame body to tell apart the
/// three startup variants. Returns `None` if body is shorter than 4
/// bytes (caller should treat as malformed).
pub fn classify_startup(body: &[u8]) -> Option<StartupKind> {
    if body.len() < 4 {
        return None;
    }
    let code = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
    Some(match code {
        SSL_REQUEST_CODE => StartupKind::SslRequest,
        CANCEL_REQUEST_CODE => StartupKind::CancelRequest,
        _ => StartupKind::Startup,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupKind {
    /// Real StartupMessage; caller should send AuthOk + ParameterStatus
    /// + ReadyForQuery and call [`PgWireCodec::advance_to_regular`].
    Startup,
    /// SSLRequest; caller writes a single byte (`N` to decline, `S` to
    /// accept) and keeps the codec in startup.
    SslRequest,
    /// CancelRequest; caller closes the connection without further reply.
    CancelRequest,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg_regular_frame(type_byte: u8, payload: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[type_byte]);
        let length_value = (4 + payload.len()) as u32;
        buf.extend_from_slice(&length_value.to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn decode_startup_frame_no_type_byte() {
        let payload = b"\x00\x03\x00\x00user\x00alice\x00\x00";
        let length_value = (4 + payload.len()) as u32;
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&length_value.to_be_bytes());
        buf.extend_from_slice(payload);

        let mut codec = PgWireCodec::new();
        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.type_byte, None);
        assert_eq!(&frame.body[..], payload);
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_regular_frame_after_advance() {
        let mut codec = PgWireCodec::new();
        codec.advance_to_regular();
        assert_eq!(codec.phase(), PgPhase::Regular);

        let mut buf = pg_regular_frame(b'Q', b"SELECT 1\0");
        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.type_byte, Some(b'Q'));
        assert_eq!(&frame.body[..], b"SELECT 1\0");
    }

    #[test]
    fn decode_partial_startup_returns_none() {
        let mut buf = BytesMut::from(&[0u8, 0, 0, 8, 0xDE][..]); // length=8, only 1 of 4 payload bytes
        let mut codec = PgWireCodec::new();
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!(buf.len(), 5, "buffer untouched");
    }

    #[test]
    fn encode_then_decode_regular_roundtrip() {
        let mut codec = PgWireCodec::new();
        codec.advance_to_regular();
        let mut buf = BytesMut::new();
        let frame = PgFrame { type_byte: Some(b'C'), body: Bytes::from_static(b"SELECT 3\0") };
        codec.encode(frame, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.type_byte, Some(b'C'));
        assert_eq!(&decoded.body[..], b"SELECT 3\0");
        assert!(buf.is_empty());
    }

    #[test]
    fn encode_then_decode_startup_roundtrip() {
        let mut codec = PgWireCodec::new();
        let mut buf = BytesMut::new();
        let frame = PgFrame { type_byte: None, body: Bytes::from_static(b"\x00\x03\x00\x00user\x00x\x00\x00") };
        codec.encode(frame, &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.type_byte, None);
    }

    #[test]
    fn classify_startup_recognises_three_variants() {
        let mut ssl_body = Vec::from([0u8; 4]);
        ssl_body[..4].copy_from_slice(&SSL_REQUEST_CODE.to_be_bytes());
        assert_eq!(classify_startup(&ssl_body), Some(StartupKind::SslRequest));

        let mut cancel_body = Vec::from([0u8; 4]);
        cancel_body[..4].copy_from_slice(&CANCEL_REQUEST_CODE.to_be_bytes());
        assert_eq!(classify_startup(&cancel_body), Some(StartupKind::CancelRequest));

        let mut startup_body = vec![0u8; 8];
        startup_body[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        assert_eq!(classify_startup(&startup_body), Some(StartupKind::Startup));

        assert_eq!(classify_startup(&[0, 0]), None);
    }

    #[test]
    fn limit_error_on_oversized_announcement() {
        let mut codec = PgWireCodec::with_limit(64);
        codec.advance_to_regular();
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[b'Q']);
        buf.extend_from_slice(&1_000_000u32.to_be_bytes());
        match codec.decode(&mut buf) {
            Err(FrameError::Limit { .. }) => {}
            other => panic!("expected Limit, got {other:?}"),
        }
    }
}
