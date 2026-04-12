//! Tokio codec for framing PostgreSQL wire protocol v3 messages.
//!
//! The codec handles both the startup (length-prefixed only) phase and the
//! regular message phase (type byte + length prefix).

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use crate::error::{Error, Result};
use crate::protocol::message::{BackendMessage, FrontendMessage, StartupMessage};

// ─────────────────────────────────────────────────────────────────────────────
// Startup codec — reads the initial length-prefixed startup message
// ─────────────────────────────────────────────────────────────────────────────

/// Codec for the startup phase — expects a 4-byte length followed by that many
/// bytes of payload (length is inclusive of itself).
pub struct StartupCodec;

impl Decoder for StartupCodec {
    type Item = StartupMessage;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> std::result::Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if len < 8 {
            // Minimum valid startup: 4 bytes length + 4 bytes protocol version
            return Err(Error::Protocol(format!("startup message length {len} is too short")));
        }
        if src.len() < len {
            src.reserve(len - src.len());
            return Ok(None);
        }
        src.advance(4); // consume length
        let payload = src.split_to(len - 4).freeze();
        let msg = StartupMessage::parse(&payload)?;
        Ok(Some(msg))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Regular message codec — type byte + 4-byte length + body
// ─────────────────────────────────────────────────────────────────────────────

/// Codec for regular (post-startup) client messages.
pub struct PgCodec;

impl Decoder for PgCodec {
    type Item = FrontendMessage;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> std::result::Result<Option<Self::Item>, Self::Error> {
        // Need at least 5 bytes: 1 type + 4 length
        if src.len() < 5 {
            return Ok(None);
        }
        let msg_type = src[0];
        let msg_len = i32::from_be_bytes([src[1], src[2], src[3], src[4]]) as usize;
        // msg_len includes the 4-byte length field but NOT the type byte
        if msg_len < 4 {
            return Err(Error::Protocol(format!(
                "message length {msg_len} is too short for type '{}'",
                msg_type as char
            )));
        }
        let total = 1 + msg_len; // type byte + length field + body
        if src.len() < total {
            src.reserve(total - src.len());
            return Ok(None);
        }
        src.advance(5); // consume type + length
        let body_len = msg_len - 4;
        let body = src.split_to(body_len).freeze();
        let msg = FrontendMessage::parse(msg_type, &body)?;
        Ok(Some(msg))
    }
}

impl Encoder<BackendMessage> for PgCodec {
    type Error = Error;

    fn encode(&mut self, msg: BackendMessage, dst: &mut BytesMut) -> std::result::Result<(), Self::Error> {
        let bytes = msg.encode();
        dst.extend_from_slice(&bytes);
        Ok(())
    }
}

/// Encoder for backend messages only — used when we hold the write half.
pub struct BackendEncoder;

impl Encoder<BackendMessage> for BackendEncoder {
    type Error = Error;

    fn encode(&mut self, msg: BackendMessage, dst: &mut BytesMut) -> std::result::Result<(), Self::Error> {
        let bytes = msg.encode();
        dst.extend_from_slice(&bytes);
        Ok(())
    }
}

/// Encoder for raw bytes (for SSL negotiation responses).
pub struct RawEncoder;

impl Encoder<u8> for RawEncoder {
    type Error = Error;

    fn encode(&mut self, byte: u8, dst: &mut BytesMut) -> std::result::Result<(), Self::Error> {
        dst.put_u8(byte);
        Ok(())
    }
}
