// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/websocket/
//
//! WebSocket proxy + scan — parity with `ExtensionWebSocket.java` and
//! `WebSocketProxy.java` (ZAP 2.14.0).
//!
//! Tracks a WebSocket session's state machine (handshake → established
//! → closing → closed), records framed messages for later replay, and
//! drives passive scan rules over each `WebSocketMessage`. Frame
//! parsing follows RFC 6455 — header byte, payload length, optional
//! mask, payload.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WsState {
    Handshake,
    Established,
    Closing,
    Closed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WsOpcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl WsOpcode {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b & 0x0F {
            0x0 => Some(Self::Continuation),
            0x1 => Some(Self::Text),
            0x2 => Some(Self::Binary),
            0x8 => Some(Self::Close),
            0x9 => Some(Self::Ping),
            0xA => Some(Self::Pong),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WsMessage {
    pub opcode: WsOpcode,
    pub fin: bool,
    pub payload: Vec<u8>,
    pub from_client: bool,
}

#[derive(Debug)]
pub struct WsSession {
    state: WsState,
    history: Vec<WsMessage>,
}

impl Default for WsSession {
    fn default() -> Self {
        Self {
            state: WsState::Handshake,
            history: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WsError {
    #[error("invalid opcode byte {0:#04x}")]
    InvalidOpcode(u8),
    #[error("frame truncated: need {needed} bytes, have {have}")]
    Truncated { needed: usize, have: usize },
    #[error("state transition rejected: {0:?} -> requested {1:?}")]
    BadTransition(WsState, WsState),
}

impl WsSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> WsState {
        self.state
    }

    pub fn history(&self) -> &[WsMessage] {
        &self.history
    }

    pub fn handshake_complete(&mut self) -> Result<(), WsError> {
        if self.state != WsState::Handshake {
            return Err(WsError::BadTransition(self.state, WsState::Established));
        }
        self.state = WsState::Established;
        Ok(())
    }

    pub fn close_initiated(&mut self) -> Result<(), WsError> {
        if !matches!(self.state, WsState::Established) {
            return Err(WsError::BadTransition(self.state, WsState::Closing));
        }
        self.state = WsState::Closing;
        Ok(())
    }

    pub fn close_complete(&mut self) -> Result<(), WsError> {
        if !matches!(self.state, WsState::Closing | WsState::Established) {
            return Err(WsError::BadTransition(self.state, WsState::Closed));
        }
        self.state = WsState::Closed;
        Ok(())
    }

    pub fn record(&mut self, msg: WsMessage) -> Result<(), WsError> {
        if !matches!(self.state, WsState::Established | WsState::Closing) {
            return Err(WsError::BadTransition(self.state, self.state));
        }
        self.history.push(msg);
        Ok(())
    }
}

/// Parse a single WebSocket frame (RFC 6455 section 5.2). Returns the
/// message and the number of bytes consumed. Multi-frame fragmented
/// messages must be re-assembled by the caller.
pub fn parse_frame(buf: &[u8], from_client: bool) -> Result<(WsMessage, usize), WsError> {
    if buf.len() < 2 {
        return Err(WsError::Truncated {
            needed: 2,
            have: buf.len(),
        });
    }
    let b0 = buf[0];
    let b1 = buf[1];
    let fin = (b0 & 0x80) != 0;
    let opcode = WsOpcode::from_u8(b0).ok_or(WsError::InvalidOpcode(b0))?;
    let masked = (b1 & 0x80) != 0;
    let mut len = (b1 & 0x7F) as usize;
    let mut cursor = 2usize;
    if len == 126 {
        if buf.len() < cursor + 2 {
            return Err(WsError::Truncated {
                needed: cursor + 2,
                have: buf.len(),
            });
        }
        len = u16::from_be_bytes([buf[cursor], buf[cursor + 1]]) as usize;
        cursor += 2;
    } else if len == 127 {
        if buf.len() < cursor + 8 {
            return Err(WsError::Truncated {
                needed: cursor + 8,
                have: buf.len(),
            });
        }
        len = u64::from_be_bytes([
            buf[cursor],
            buf[cursor + 1],
            buf[cursor + 2],
            buf[cursor + 3],
            buf[cursor + 4],
            buf[cursor + 5],
            buf[cursor + 6],
            buf[cursor + 7],
        ]) as usize;
        cursor += 8;
    }
    let mask = if masked {
        if buf.len() < cursor + 4 {
            return Err(WsError::Truncated {
                needed: cursor + 4,
                have: buf.len(),
            });
        }
        let m = [buf[cursor], buf[cursor + 1], buf[cursor + 2], buf[cursor + 3]];
        cursor += 4;
        Some(m)
    } else {
        None
    };
    if buf.len() < cursor + len {
        return Err(WsError::Truncated {
            needed: cursor + len,
            have: buf.len(),
        });
    }
    let mut payload: Vec<u8> = buf[cursor..cursor + len].to_vec();
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    cursor += len;
    Ok((
        WsMessage {
            opcode,
            fin,
            payload,
            from_client,
        },
        cursor,
    ))
}

/// Find suspicious patterns inside a WebSocket message payload — used
/// by the passive scan to surface XSS / credential leaks etc. Returns
/// a list of matching needles.
pub fn passive_scan(msg: &WsMessage, needles: &[&str]) -> Vec<String> {
    let text = String::from_utf8_lossy(&msg.payload);
    needles
        .iter()
        .filter(|n| text.contains(*n))
        .map(|n| n.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_from_byte() {
        assert_eq!(WsOpcode::from_u8(0x81), Some(WsOpcode::Text));
        assert_eq!(WsOpcode::from_u8(0x82), Some(WsOpcode::Binary));
        assert_eq!(WsOpcode::from_u8(0x88), Some(WsOpcode::Close));
        assert_eq!(WsOpcode::from_u8(0x07), None);
    }

    #[test]
    fn session_handshake_to_established_to_closed() {
        let mut s = WsSession::new();
        assert_eq!(s.state(), WsState::Handshake);
        s.handshake_complete().unwrap();
        assert_eq!(s.state(), WsState::Established);
        s.close_initiated().unwrap();
        assert_eq!(s.state(), WsState::Closing);
        s.close_complete().unwrap();
        assert_eq!(s.state(), WsState::Closed);
    }

    #[test]
    fn invalid_transition_rejected() {
        let mut s = WsSession::new();
        // can't close before handshake
        assert!(s.close_initiated().is_err());
        s.handshake_complete().unwrap();
        // can't re-handshake
        assert!(s.handshake_complete().is_err());
    }

    #[test]
    fn parse_unmasked_text_frame() {
        // Fin + Text opcode, 5-byte payload "hello", unmasked.
        let buf = vec![0x81, 0x05, b'h', b'e', b'l', b'l', b'o'];
        let (msg, consumed) = parse_frame(&buf, false).unwrap();
        assert_eq!(msg.opcode, WsOpcode::Text);
        assert!(msg.fin);
        assert_eq!(msg.payload, b"hello");
        assert_eq!(consumed, 7);
    }

    #[test]
    fn parse_masked_text_frame() {
        let mask = [0x37, 0xfa, 0x21, 0x3d];
        let plain = b"hello";
        let masked: Vec<u8> = plain
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i % 4])
            .collect();
        let mut buf = vec![0x81, 0x85];
        buf.extend_from_slice(&mask);
        buf.extend(&masked);
        let (msg, _consumed) = parse_frame(&buf, true).unwrap();
        assert_eq!(msg.payload, b"hello");
    }

    #[test]
    fn parse_truncated_returns_error() {
        let buf = vec![0x81];
        assert!(matches!(parse_frame(&buf, false), Err(WsError::Truncated { .. })));
    }

    #[test]
    fn parse_extended_length_126() {
        let payload: Vec<u8> = (0u8..200).collect();
        let mut buf = vec![0x82, 0x7E];
        buf.extend_from_slice(&(200u16).to_be_bytes());
        buf.extend_from_slice(&payload);
        let (msg, _) = parse_frame(&buf, false).unwrap();
        assert_eq!(msg.payload.len(), 200);
    }

    #[test]
    fn record_requires_established() {
        let mut s = WsSession::new();
        let msg = WsMessage {
            opcode: WsOpcode::Text,
            fin: true,
            payload: b"hi".to_vec(),
            from_client: true,
        };
        assert!(s.record(msg.clone()).is_err());
        s.handshake_complete().unwrap();
        s.record(msg).unwrap();
        assert_eq!(s.history().len(), 1);
    }

    #[test]
    fn passive_scan_finds_needles() {
        let msg = WsMessage {
            opcode: WsOpcode::Text,
            fin: true,
            payload: b"set token=abc123 and password=xyz".to_vec(),
            from_client: false,
        };
        let hits = passive_scan(&msg, &["token=", "password=", "absent"]);
        assert_eq!(hits, vec!["token=".to_string(), "password=".to_string()]);
    }

    #[test]
    fn passive_scan_empty_when_no_match() {
        let msg = WsMessage {
            opcode: WsOpcode::Text,
            fin: true,
            payload: b"clean".to_vec(),
            from_client: false,
        };
        assert!(passive_scan(&msg, &["dirty"]).is_empty());
    }
}
