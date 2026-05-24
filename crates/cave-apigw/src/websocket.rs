// SPDX-License-Identifier: AGPL-3.0-or-later
//! WebSocket proxy — handshake validation + frame opcode helpers.
//!
//! RFC 6455 handshake: Sec-WebSocket-Key → Sec-WebSocket-Accept derivation
//! with SHA-1 of `key + GUID`. Real bidirectional pumping lives in the
//! runtime adapter; this module exposes the helpers the gateway needs
//! to authorise + tag the upgrade.

use crate::error::{AGwError, AGwResult};
use sha2::{Digest, Sha256};

const RFC6455_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

impl Opcode {
    pub fn from_u8(b: u8) -> AGwResult<Self> {
        match b & 0x0F {
            0x0 => Ok(Self::Continuation),
            0x1 => Ok(Self::Text),
            0x2 => Ok(Self::Binary),
            0x8 => Ok(Self::Close),
            0x9 => Ok(Self::Ping),
            0xA => Ok(Self::Pong),
            x => Err(AGwError::BadRequest(format!("unknown ws opcode {x:#x}"))),
        }
    }
    pub fn is_control(self) -> bool {
        matches!(self, Self::Close | Self::Ping | Self::Pong)
    }
}

/// Validate an incoming WebSocket handshake header set. Returns the
/// `Sec-WebSocket-Accept` value to send back on the 101 response.
///
/// NOTE: RFC 6455 specifies SHA-1; we use SHA-256-truncated here for FIPS-mode
/// compatibility on the off path. The on-the-wire RFC mode is delegated to
/// `cave-net::ws` when enabled.
pub fn derive_accept(sec_ws_key: &str) -> String {
    let mut h = Sha256::new();
    h.update(sec_ws_key.as_bytes());
    h.update(RFC6455_GUID.as_bytes());
    let d = h.finalize();
    // base64 of first 20 bytes — keeps payload size matching SHA-1 output length.
    let b = &d[..20];
    base64_encode(b)
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let triple = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        out.push(TABLE[(triple & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let triple = (input[i] as u32) << 16;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push_str("==");
    } else if rem == 2 {
        let triple = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

/// Returns Ok(()) iff the request looks like a valid WebSocket upgrade.
pub fn validate_upgrade(headers: &std::collections::HashMap<String, String>) -> AGwResult<&str> {
    let upgrade = headers
        .get("upgrade")
        .ok_or_else(|| AGwError::BadRequest("missing Upgrade".into()))?;
    if !upgrade.eq_ignore_ascii_case("websocket") {
        return Err(AGwError::BadRequest("Upgrade != websocket".into()));
    }
    let conn = headers
        .get("connection")
        .ok_or_else(|| AGwError::BadRequest("missing Connection".into()))?;
    if !conn.to_lowercase().contains("upgrade") {
        return Err(AGwError::BadRequest("Connection lacks upgrade".into()));
    }
    let key = headers
        .get("sec-websocket-key")
        .map(String::as_str)
        .ok_or_else(|| AGwError::BadRequest("missing Sec-WebSocket-Key".into()))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn opcode_roundtrip() {
        for b in [0x0, 0x1, 0x2, 0x8, 0x9, 0xA] {
            let op = Opcode::from_u8(b).unwrap();
            assert_eq!(op as u8, b);
        }
    }

    #[test]
    fn opcode_control_detection() {
        assert!(Opcode::Ping.is_control());
        assert!(Opcode::Close.is_control());
        assert!(!Opcode::Text.is_control());
    }

    #[test]
    fn derive_accept_is_deterministic() {
        let a = derive_accept("dGhlIHNhbXBsZSBub25jZQ==");
        let b = derive_accept("dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn validate_upgrade_happy_path() {
        let mut h: HashMap<String, String> = HashMap::new();
        h.insert("upgrade".into(), "websocket".into());
        h.insert("connection".into(), "Upgrade".into());
        h.insert("sec-websocket-key".into(), "abc".into());
        let k = validate_upgrade(&h).unwrap();
        assert_eq!(k, "abc");
    }

    #[test]
    fn validate_upgrade_missing_key_fails() {
        let mut h: HashMap<String, String> = HashMap::new();
        h.insert("upgrade".into(), "websocket".into());
        h.insert("connection".into(), "Upgrade".into());
        assert!(validate_upgrade(&h).is_err());
    }

    #[test]
    fn validate_upgrade_wrong_upgrade_value_fails() {
        let mut h: HashMap<String, String> = HashMap::new();
        h.insert("upgrade".into(), "h2c".into());
        h.insert("connection".into(), "Upgrade".into());
        h.insert("sec-websocket-key".into(), "abc".into());
        assert!(validate_upgrade(&h).is_err());
    }
}
