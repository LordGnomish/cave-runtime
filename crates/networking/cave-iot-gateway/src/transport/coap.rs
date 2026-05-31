// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CoAP transport codec (RFC 7252 message format).
//!
//! Ports the `transport/coap` `CoapTransportResource` wire format: the
//! 4-byte fixed header (ver/type/TKL, code, message-id), variable-length
//! token, the delta/length option encoding (RFC 7252 §3.1 incl. the 13/14
//! extended nibbles) and the `0xFF` payload marker. No DTLS socket.

use crate::{IotError, Result};

/// CoAP message type (RFC 7252 §3, the 2-bit T field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoapType {
    Confirmable = 0,
    NonConfirmable = 1,
    Acknowledgement = 2,
    Reset = 3,
}

impl CoapType {
    pub fn from_bits(b: u8) -> Option<CoapType> {
        Some(match b {
            0 => CoapType::Confirmable,
            1 => CoapType::NonConfirmable,
            2 => CoapType::Acknowledgement,
            3 => CoapType::Reset,
            _ => return None,
        })
    }
}

/// CoAP code byte: 3-bit class + 5-bit detail (`c.dd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoapCode(u8);

impl CoapCode {
    pub fn new(class: u8, detail: u8) -> CoapCode {
        CoapCode(((class & 0x07) << 5) | (detail & 0x1F))
    }
    pub fn from_u8(b: u8) -> CoapCode {
        CoapCode(b)
    }
    pub fn as_u8(&self) -> u8 {
        self.0
    }
    pub fn class(&self) -> u8 {
        self.0 >> 5
    }
    pub fn detail(&self) -> u8 {
        self.0 & 0x1F
    }
}

/// A CoAP option (number + opaque value). Options are stored sorted; the
/// codec computes the delta encoding on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoapOption {
    pub number: u16,
    pub value: Vec<u8>,
}

/// A decoded CoAP message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoapMessage {
    pub version: u8,
    pub msg_type: CoapType,
    pub code: CoapCode,
    pub message_id: u16,
    pub token: Vec<u8>,
    pub options: Vec<CoapOption>,
    pub payload: Vec<u8>,
}

/// Encode a 4-bit nibble + its extended bytes per RFC 7252 §3.1.
fn encode_ext(value: u16) -> (u8, Vec<u8>) {
    if value < 13 {
        (value as u8, vec![])
    } else if value < 269 {
        (13, vec![(value - 13) as u8])
    } else {
        let v = value - 269;
        (14, vec![(v >> 8) as u8, (v & 0xFF) as u8])
    }
}

/// Decode an extended nibble, reading from `buf` at `*pos`.
fn decode_ext(nibble: u8, buf: &[u8], pos: &mut usize) -> Result<u16> {
    match nibble {
        0..=12 => Ok(nibble as u16),
        13 => {
            let b = *buf
                .get(*pos)
                .ok_or_else(|| IotError::Codec("truncated 1-byte ext".into()))?;
            *pos += 1;
            Ok(b as u16 + 13)
        }
        14 => {
            let hi = *buf
                .get(*pos)
                .ok_or_else(|| IotError::Codec("truncated 2-byte ext".into()))?;
            let lo = *buf
                .get(*pos + 1)
                .ok_or_else(|| IotError::Codec("truncated 2-byte ext".into()))?;
            *pos += 2;
            Ok(((hi as u16) << 8 | lo as u16) + 269)
        }
        _ => Err(IotError::Codec("reserved option nibble 15".into())),
    }
}

impl CoapMessage {
    pub fn encode(&self) -> Vec<u8> {
        let tkl = self.token.len().min(8) as u8;
        let mut out = Vec::new();
        out.push((self.version << 6) | ((self.msg_type as u8) << 4) | tkl);
        out.push(self.code.as_u8());
        out.extend_from_slice(&self.message_id.to_be_bytes());
        out.extend_from_slice(&self.token[..tkl as usize]);

        // Options must be emitted in ascending option number; deltas are
        // relative to the previous option number.
        let mut opts = self.options.clone();
        opts.sort_by_key(|o| o.number);
        let mut last = 0u16;
        for o in &opts {
            let delta = o.number - last;
            last = o.number;
            let (dn, dext) = encode_ext(delta);
            let (ln, lext) = encode_ext(o.value.len() as u16);
            out.push((dn << 4) | ln);
            out.extend_from_slice(&dext);
            out.extend_from_slice(&lext);
            out.extend_from_slice(&o.value);
        }
        if !self.payload.is_empty() {
            out.push(0xFF);
            out.extend_from_slice(&self.payload);
        }
        out
    }

    pub fn decode(buf: &[u8]) -> Result<CoapMessage> {
        if buf.len() < 4 {
            return Err(IotError::Codec("CoAP message shorter than 4 bytes".into()));
        }
        let version = buf[0] >> 6;
        if version != 1 {
            return Err(IotError::Codec(format!("unsupported CoAP version {version}")));
        }
        let msg_type = CoapType::from_bits((buf[0] >> 4) & 0x03)
            .ok_or_else(|| IotError::Codec("bad CoAP type".into()))?;
        let tkl = (buf[0] & 0x0F) as usize;
        if tkl > 8 {
            return Err(IotError::Codec("token length > 8".into()));
        }
        let code = CoapCode::from_u8(buf[1]);
        let message_id = u16::from_be_bytes([buf[2], buf[3]]);
        let mut pos = 4;
        let token = buf
            .get(pos..pos + tkl)
            .ok_or_else(|| IotError::Codec("truncated token".into()))?
            .to_vec();
        pos += tkl;

        let mut options = Vec::new();
        let mut current = 0u16;
        let mut payload = Vec::new();
        while pos < buf.len() {
            let b = buf[pos];
            if b == 0xFF {
                payload = buf[pos + 1..].to_vec();
                break;
            }
            pos += 1;
            let delta = decode_ext(b >> 4, buf, &mut pos)?;
            let length = decode_ext(b & 0x0F, buf, &mut pos)? as usize;
            current += delta;
            let value = buf
                .get(pos..pos + length)
                .ok_or_else(|| IotError::Codec("truncated option value".into()))?
                .to_vec();
            pos += length;
            options.push(CoapOption { number: current, value });
        }
        Ok(CoapMessage {
            version,
            msg_type,
            code,
            message_id,
            token,
            options,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_byte_roundtrip() {
        for t in [
            CoapType::Confirmable,
            CoapType::NonConfirmable,
            CoapType::Acknowledgement,
            CoapType::Reset,
        ] {
            assert_eq!(CoapType::from_bits(t as u8), Some(t));
        }
    }

    #[test]
    fn code_class_detail() {
        // GET = 0.01, POST = 0.02, 2.05 Content, 4.04 Not Found.
        assert_eq!(CoapCode::new(0, 1).as_u8(), 0x01);
        assert_eq!(CoapCode::new(0, 2).as_u8(), 0x02);
        assert_eq!(CoapCode::new(2, 5).as_u8(), 0x45);
        let nf = CoapCode::from_u8(0x84);
        assert_eq!(nf.class(), 4);
        assert_eq!(nf.detail(), 4);
    }

    #[test]
    fn message_roundtrip_with_token_and_payload() {
        let msg = CoapMessage {
            version: 1,
            msg_type: CoapType::Confirmable,
            code: CoapCode::new(0, 2), // POST
            message_id: 0xBEEF,
            token: vec![0x11, 0x22],
            options: vec![],
            payload: b"{\"t\":21}".to_vec(),
        };
        let bytes = msg.encode();
        let back = CoapMessage::decode(&bytes).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn message_roundtrip_with_options() {
        // Uri-Path (11) "telemetry", Content-Format (12) [0x32]
        let msg = CoapMessage {
            version: 1,
            msg_type: CoapType::NonConfirmable,
            code: CoapCode::new(0, 2),
            message_id: 1,
            token: vec![],
            options: vec![
                CoapOption { number: 11, value: b"telemetry".to_vec() },
                CoapOption { number: 12, value: vec![0x32] },
            ],
            payload: vec![],
        };
        let back = CoapMessage::decode(&msg.encode()).unwrap();
        assert_eq!(back.options.len(), 2);
        assert_eq!(back.options[0].number, 11);
        assert_eq!(back.options[0].value, b"telemetry");
        assert_eq!(back.options[1].number, 12);
    }

    #[test]
    fn option_with_large_delta_uses_extended_nibble() {
        // Option number 270 forces the 2-byte extended delta (>268).
        let msg = CoapMessage {
            version: 1,
            msg_type: CoapType::Confirmable,
            code: CoapCode::new(0, 1),
            message_id: 5,
            token: vec![],
            options: vec![CoapOption { number: 270, value: b"x".to_vec() }],
            payload: vec![],
        };
        let back = CoapMessage::decode(&msg.encode()).unwrap();
        assert_eq!(back.options[0].number, 270);
    }

    #[test]
    fn decode_rejects_bad_version_and_truncation() {
        // version 2 (top two bits = 10) is invalid.
        let bad = vec![0x80, 0x01, 0x00, 0x00];
        assert!(CoapMessage::decode(&bad).is_err());
        assert!(CoapMessage::decode(&[]).is_err());
    }
}
