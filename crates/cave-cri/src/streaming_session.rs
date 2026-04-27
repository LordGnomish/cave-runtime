//! Streaming session — SPDY/3.1 framing, stream lifecycle, multiplex
//! demultiplexer, TTY (pty + winsize), and the CRI error stream JSON.
//!
//! Sits on top of `crate::streaming` (channel-byte framing) and
//! `crate::transport` (raw byte transport). This module owns:
//!
//! - `SpdyFrame` — header + payload encode/decode (SYN_STREAM,
//!   SYN_REPLY, DATA, RST_STREAM, GOAWAY).
//! - `StreamState` lifecycle: Idle → Open → HalfClosed{Local,Remote}
//!   → Closed. Mirrors the SPDY/3.1 state machine.
//! - `StreamSession` — multiplexed N-stream session over one transport,
//!   with monotonic odd-numbered stream IDs (client-initiated) and a
//!   per-stream flow-control window.
//! - `Pty` — winsize record + control char map; `ResizeMessage` JSON
//!   payload kubelet sends on the resize channel.
//! - `StreamErrorStatus` — kubelet error-channel JSON
//!   (`{"status":"Failure",…}`) following Kubernetes' meta/v1.Status.
//!
//! Upstream:
//! - SPDY/3.1: <https://www.chromium.org/spdy/spdy-protocol/spdy-protocol-draft3-1/>
//! - kubernetes/staging/src/k8s.io/apimachinery/pkg/util/httpstream/spdy
//! - kubernetes/staging/src/k8s.io/client-go/tools/remotecommand

use crate::error::{CriError, CriResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

// ── SPDY frame format ────────────────────────────────────────────────────────

/// Frame type — SPDY/3.1 §2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpdyFrameKind {
    Data,
    SynStream,
    SynReply,
    RstStream,
    Settings,
    Ping,
    GoAway,
    Headers,
    WindowUpdate,
}

impl SpdyFrameKind {
    pub fn control_type(self) -> Option<u16> {
        Some(match self {
            SpdyFrameKind::Data => return None,
            SpdyFrameKind::SynStream    => 0x0001,
            SpdyFrameKind::SynReply     => 0x0002,
            SpdyFrameKind::RstStream    => 0x0003,
            SpdyFrameKind::Settings     => 0x0004,
            SpdyFrameKind::Ping         => 0x0006,
            SpdyFrameKind::GoAway       => 0x0007,
            SpdyFrameKind::Headers      => 0x0008,
            SpdyFrameKind::WindowUpdate => 0x0009,
        })
    }

    pub fn from_control_type(ty: u16) -> Option<SpdyFrameKind> {
        Some(match ty {
            0x0001 => SpdyFrameKind::SynStream,
            0x0002 => SpdyFrameKind::SynReply,
            0x0003 => SpdyFrameKind::RstStream,
            0x0004 => SpdyFrameKind::Settings,
            0x0006 => SpdyFrameKind::Ping,
            0x0007 => SpdyFrameKind::GoAway,
            0x0008 => SpdyFrameKind::Headers,
            0x0009 => SpdyFrameKind::WindowUpdate,
            _ => return None,
        })
    }
}

/// `FLAG_FIN` — half-closes the stream after this frame. SPDY §2.6.1.
pub const FLAG_FIN: u8 = 0x01;
/// `FLAG_UNIDIRECTIONAL` on SYN_STREAM. SPDY §2.6.1.
pub const FLAG_UNIDIRECTIONAL: u8 = 0x02;

/// SPDY frame — control or data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpdyFrame {
    pub kind: SpdyFrameKind,
    pub flags: u8,
    pub stream_id: u32,
    pub payload: Vec<u8>,
}

impl SpdyFrame {
    /// Encode a SPDY/3.1 frame.
    ///
    /// Data frame layout (8-byte header):
    /// ```text
    /// +----------------------------------+
    /// | 0|        Stream-ID (31 bits)    |
    /// +----------------------------------+
    /// | Flags (8) |  Length (24 bits)    |
    /// +----------------------------------+
    /// |               Data               |
    /// +----------------------------------+
    /// ```
    ///
    /// Control frame layout:
    /// ```text
    /// +----------------------------------+
    /// | 1| Version(15) |     Type(16)    |
    /// +----------------------------------+
    /// | Flags (8) |  Length (24 bits)    |
    /// +----------------------------------+
    /// |               Data               |
    /// +----------------------------------+
    /// ```
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + self.payload.len());
        if self.kind == SpdyFrameKind::Data {
            // High bit clear → data frame; stream ID in low 31 bits.
            let sid = self.stream_id & 0x7FFF_FFFF;
            buf.extend_from_slice(&sid.to_be_bytes());
        } else {
            // High bit set → control frame; version 3 (0x8003).
            let header: u32 = 0x8003_0000 | self.kind.control_type().unwrap_or(0) as u32;
            buf.extend_from_slice(&header.to_be_bytes());
        }
        let length = self.payload.len() as u32; // 24-bit length
        let flags_and_length = ((self.flags as u32) << 24) | (length & 0x00FF_FFFF);
        buf.extend_from_slice(&flags_and_length.to_be_bytes());
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Decode a SPDY/3.1 frame from `bytes`. Returns the frame and the
    /// number of bytes consumed (8 + payload_length).
    pub fn decode(bytes: &[u8]) -> CriResult<(SpdyFrame, usize)> {
        if bytes.len() < 8 {
            return Err(CriError::Runtime("SPDY frame shorter than 8-byte header".into()));
        }
        let word0 = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let word1 = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let flags = (word1 >> 24) as u8;
        let length = (word1 & 0x00FF_FFFF) as usize;

        let total = 8 + length;
        if bytes.len() < total {
            return Err(CriError::Runtime(format!(
                "SPDY frame truncated: expected {} bytes, have {}",
                total, bytes.len()
            )));
        }
        let payload = bytes[8..total].to_vec();

        let frame = if word0 & 0x8000_0000 != 0 {
            // Control frame
            let version = ((word0 >> 16) & 0x7FFF) as u16;
            if version != 3 {
                return Err(CriError::Runtime(format!("unsupported SPDY version: {}", version)));
            }
            let ty = (word0 & 0x0000_FFFF) as u16;
            let kind = SpdyFrameKind::from_control_type(ty)
                .ok_or_else(|| CriError::Runtime(format!("unknown control frame type: {}", ty)))?;
            // For control frames, stream_id is encoded in the first 4 bytes
            // of the payload for SYN_STREAM / SYN_REPLY / RST_STREAM /
            // HEADERS / WINDOW_UPDATE.
            let stream_id = if matches!(
                kind,
                SpdyFrameKind::SynStream
                    | SpdyFrameKind::SynReply
                    | SpdyFrameKind::RstStream
                    | SpdyFrameKind::Headers
                    | SpdyFrameKind::WindowUpdate
            ) && payload.len() >= 4
            {
                u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) & 0x7FFF_FFFF
            } else {
                0
            };
            SpdyFrame { kind, flags, stream_id, payload }
        } else {
            let stream_id = word0 & 0x7FFF_FFFF;
            SpdyFrame { kind: SpdyFrameKind::Data, flags, stream_id, payload }
        };
        Ok((frame, total))
    }

    /// True if `FLAG_FIN` is set.
    pub fn is_fin(&self) -> bool {
        self.flags & FLAG_FIN != 0
    }

    pub fn data(stream_id: u32, payload: impl Into<Vec<u8>>, fin: bool) -> Self {
        Self {
            kind: SpdyFrameKind::Data,
            flags: if fin { FLAG_FIN } else { 0 },
            stream_id,
            payload: payload.into(),
        }
    }

    pub fn syn_stream(stream_id: u32, fin: bool, unidirectional: bool) -> Self {
        let mut payload = Vec::with_capacity(10);
        payload.extend_from_slice(&(stream_id & 0x7FFF_FFFF).to_be_bytes());
        payload.extend_from_slice(&[0, 0, 0, 0]); // associated-stream-id = 0
        payload.extend_from_slice(&[0, 0]);       // priority + slot
        let mut flags = 0;
        if fin { flags |= FLAG_FIN; }
        if unidirectional { flags |= FLAG_UNIDIRECTIONAL; }
        Self { kind: SpdyFrameKind::SynStream, flags, stream_id, payload }
    }

    pub fn rst_stream(stream_id: u32, status_code: u32) -> Self {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&(stream_id & 0x7FFF_FFFF).to_be_bytes());
        payload.extend_from_slice(&status_code.to_be_bytes());
        Self { kind: SpdyFrameKind::RstStream, flags: 0, stream_id, payload }
    }

    pub fn goaway(last_good_stream_id: u32, status_code: u32) -> Self {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&(last_good_stream_id & 0x7FFF_FFFF).to_be_bytes());
        payload.extend_from_slice(&status_code.to_be_bytes());
        Self { kind: SpdyFrameKind::GoAway, flags: 0, stream_id: 0, payload }
    }
}

// ── Stream state machine ─────────────────────────────────────────────────────

/// SPDY/3.1 stream state — §2.3.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamState {
    Idle,
    Open,
    HalfClosedLocal,
    HalfClosedRemote,
    Closed,
}

impl StreamState {
    /// Apply a sent frame with optional FIN. Returns the next state, or
    /// an error if the transition is illegal.
    pub fn after_send(self, fin: bool) -> CriResult<StreamState> {
        match self {
            StreamState::Idle if fin            => Ok(StreamState::HalfClosedLocal),
            StreamState::Idle                   => Ok(StreamState::Open),
            StreamState::Open if fin            => Ok(StreamState::HalfClosedLocal),
            StreamState::Open                   => Ok(StreamState::Open),
            StreamState::HalfClosedRemote if fin => Ok(StreamState::Closed),
            StreamState::HalfClosedRemote       => Ok(StreamState::HalfClosedRemote),
            StreamState::HalfClosedLocal => Err(CriError::Runtime(
                "cannot send on half-closed-local stream".into(),
            )),
            StreamState::Closed => Err(CriError::Runtime(
                "cannot send on closed stream".into(),
            )),
        }
    }

    pub fn after_recv(self, fin: bool) -> CriResult<StreamState> {
        match self {
            StreamState::Idle if fin             => Ok(StreamState::HalfClosedRemote),
            StreamState::Idle                    => Ok(StreamState::Open),
            StreamState::Open if fin             => Ok(StreamState::HalfClosedRemote),
            StreamState::Open                    => Ok(StreamState::Open),
            StreamState::HalfClosedLocal if fin  => Ok(StreamState::Closed),
            StreamState::HalfClosedLocal         => Ok(StreamState::HalfClosedLocal),
            StreamState::HalfClosedRemote => Err(CriError::Runtime(
                "cannot receive on half-closed-remote stream".into(),
            )),
            StreamState::Closed => Err(CriError::Runtime(
                "cannot receive on closed stream".into(),
            )),
        }
    }

    /// Apply RST_STREAM — always transitions to Closed.
    pub fn after_rst(self) -> StreamState {
        StreamState::Closed
    }
}

// ── Multiplexed stream session ───────────────────────────────────────────────

/// Initial flow-control window (SPDY §2.6.8 — 64 KiB).
pub const INITIAL_WINDOW_SIZE: i32 = 64 * 1024;

/// One half-duplex stream tracked by `StreamSession`.
#[derive(Debug, Clone)]
pub struct StreamRecord {
    pub id: u32,
    pub state: StreamState,
    pub send_window: i32,
    pub recv_window: i32,
    pub label: String,
}

impl StreamRecord {
    fn new(id: u32, label: String) -> Self {
        Self {
            id,
            state: StreamState::Idle,
            send_window: INITIAL_WINDOW_SIZE,
            recv_window: INITIAL_WINDOW_SIZE,
            label,
        }
    }
}

/// Session-level demultiplexer. Tracks in-flight streams, allocates the
/// next client-initiated stream ID (odd, monotonic), enforces the
/// SPDY/3.1 state machine, and maintains per-stream flow-control
/// windows.
#[derive(Debug)]
pub struct StreamSession {
    next_stream_id: Mutex<u32>,
    streams: Mutex<BTreeMap<u32, StreamRecord>>,
    goaway_sent: Mutex<bool>,
}

impl Default for StreamSession {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamSession {
    pub fn new() -> Self {
        Self {
            // SPDY/3.1: client streams use odd IDs starting at 1.
            next_stream_id: Mutex::new(1),
            streams: Mutex::new(BTreeMap::new()),
            goaway_sent: Mutex::new(false),
        }
    }

    /// Allocate the next odd client stream ID and register it with the
    /// supplied label (e.g. "stdin", "stdout"). Returns the allocated ID.
    pub fn open_stream(&self, label: &str) -> CriResult<u32> {
        if *self.goaway_sent.lock().unwrap() {
            return Err(CriError::Runtime("session is going away".into()));
        }
        let mut next = self.next_stream_id.lock().unwrap();
        let id = *next;
        *next = next.checked_add(2).ok_or_else(|| CriError::Runtime("stream id space exhausted".into()))?;
        self.streams
            .lock()
            .unwrap()
            .insert(id, StreamRecord::new(id, label.to_string()));
        Ok(id)
    }

    /// Apply an outgoing frame to its stream — updates state, returns
    /// an error if the transition is illegal or the flow-control window
    /// would underflow.
    pub fn send(&self, frame: &SpdyFrame) -> CriResult<()> {
        let mut streams = self.streams.lock().unwrap();
        let record = streams
            .get_mut(&frame.stream_id)
            .ok_or_else(|| CriError::Runtime(format!("unknown stream id: {}", frame.stream_id)))?;

        if frame.kind == SpdyFrameKind::RstStream {
            record.state = record.state.after_rst();
            return Ok(());
        }

        if frame.kind == SpdyFrameKind::Data {
            let len = frame.payload.len() as i32;
            if len > record.send_window {
                return Err(CriError::Runtime(format!(
                    "send window underflow on stream {}: payload {}, window {}",
                    frame.stream_id, len, record.send_window,
                )));
            }
            record.send_window -= len;
        }

        record.state = record.state.after_send(frame.is_fin())?;
        Ok(())
    }

    /// Apply an incoming frame to its stream.
    pub fn recv(&self, frame: &SpdyFrame) -> CriResult<()> {
        let mut streams = self.streams.lock().unwrap();
        // Server-initiated streams (even IDs) may auto-register on first
        // receipt — same approach kube uses for SYN_STREAM with a fresh ID.
        let record = streams
            .entry(frame.stream_id)
            .or_insert_with(|| StreamRecord::new(frame.stream_id, "remote".into()));

        if frame.kind == SpdyFrameKind::RstStream {
            record.state = record.state.after_rst();
            return Ok(());
        }

        if frame.kind == SpdyFrameKind::Data {
            let len = frame.payload.len() as i32;
            if len > record.recv_window {
                return Err(CriError::Runtime(format!(
                    "recv window underflow on stream {}: payload {}, window {}",
                    frame.stream_id, len, record.recv_window,
                )));
            }
            record.recv_window -= len;
        }

        record.state = record.state.after_recv(frame.is_fin())?;
        Ok(())
    }

    /// Increase the receive window after the consumer has drained
    /// `delta` bytes. Returns the WINDOW_UPDATE frame to send.
    pub fn ack_recv(&self, stream_id: u32, delta: u32) -> CriResult<SpdyFrame> {
        let mut streams = self.streams.lock().unwrap();
        let record = streams
            .get_mut(&stream_id)
            .ok_or_else(|| CriError::Runtime(format!("unknown stream id: {}", stream_id)))?;
        record.recv_window = record.recv_window.saturating_add(delta as i32);
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&(stream_id & 0x7FFF_FFFF).to_be_bytes());
        payload.extend_from_slice(&delta.to_be_bytes());
        Ok(SpdyFrame {
            kind: SpdyFrameKind::WindowUpdate,
            flags: 0,
            stream_id,
            payload,
        })
    }

    pub fn state(&self, stream_id: u32) -> Option<StreamState> {
        self.streams.lock().unwrap().get(&stream_id).map(|r| r.state)
    }

    pub fn label(&self, stream_id: u32) -> Option<String> {
        self.streams.lock().unwrap().get(&stream_id).map(|r| r.label.clone())
    }

    pub fn open_count(&self) -> usize {
        self.streams
            .lock()
            .unwrap()
            .values()
            .filter(|r| !matches!(r.state, StreamState::Closed))
            .count()
    }

    /// Send GOAWAY — no further streams can be opened on this session.
    pub fn go_away(&self, status_code: u32) -> SpdyFrame {
        *self.goaway_sent.lock().unwrap() = true;
        let last_good = self
            .streams
            .lock()
            .unwrap()
            .keys()
            .max()
            .copied()
            .unwrap_or(0);
        SpdyFrame::goaway(last_good, status_code)
    }
}

// ── Pty / TTY ────────────────────────────────────────────────────────────────

/// Window size (struct winsize from `termios.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyWinsize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

impl Default for PtyWinsize {
    fn default() -> Self {
        // 80x24 with no pixel dimensions — kubectl exec defaults.
        Self { rows: 24, cols: 80, xpixel: 0, ypixel: 0 }
    }
}

/// Resize message kubelet sends on the resize channel — JSON encoded
/// `{"Width":<cols>,"Height":<rows>}`. Same on-the-wire shape as
/// `crate::streaming::TtyWindowSize` but kept here so PtyWinsize can
/// translate cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ResizeMessage {
    pub width: u16,
    pub height: u16,
}

impl From<PtyWinsize> for ResizeMessage {
    fn from(w: PtyWinsize) -> Self {
        Self { width: w.cols, height: w.rows }
    }
}

impl From<ResizeMessage> for PtyWinsize {
    fn from(r: ResizeMessage) -> Self {
        PtyWinsize { rows: r.height, cols: r.width, xpixel: 0, ypixel: 0 }
    }
}

impl ResizeMessage {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> CriResult<ResizeMessage> {
        serde_json::from_slice(bytes)
            .map_err(|e| CriError::Runtime(format!("invalid resize message: {}", e)))
    }
}

/// Pty session record — tracks the negotiated winsize.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pty {
    pub winsize: PtyWinsize,
}

impl Pty {
    pub fn new(initial: PtyWinsize) -> Self {
        Self { winsize: initial }
    }

    /// Apply a resize message, returning the previous size.
    pub fn resize(&mut self, msg: ResizeMessage) -> PtyWinsize {
        let prev = self.winsize;
        self.winsize = PtyWinsize::from(msg);
        prev
    }
}

// ── Error stream JSON ────────────────────────────────────────────────────────

/// Kubernetes meta/v1.Status sent on channel 3 when exec/attach fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamErrorStatus {
    pub status: String,
    pub message: String,
    pub reason: String,
    pub details: StreamErrorDetails,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamErrorDetails {
    #[serde(default)]
    pub causes: Vec<StreamErrorCause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamErrorCause {
    pub reason: String,
    pub message: String,
}

impl StreamErrorStatus {
    /// Build the standard "non-zero exit" status sent at the end of a
    /// successful exec when the command returned non-zero.
    pub fn non_zero_exit(exit_code: i32) -> Self {
        Self {
            status: "Failure".into(),
            message: format!("command terminated with non-zero exit code: error executing command, exit code {}", exit_code),
            reason: "NonZeroExitCode".into(),
            details: StreamErrorDetails {
                causes: vec![StreamErrorCause {
                    reason: "ExitCode".into(),
                    message: exit_code.to_string(),
                }],
                exit_code: Some(exit_code),
            },
        }
    }

    /// Build the standard "success" status sent at end-of-stream.
    pub fn success() -> Self {
        Self {
            status: "Success".into(),
            message: String::new(),
            reason: String::new(),
            details: StreamErrorDetails::default(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> CriResult<StreamErrorStatus> {
        serde_json::from_slice(bytes)
            .map_err(|e| CriError::Runtime(format!("invalid stream status: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SpdyFrame encode/decode ──────────────────────────────────────────────

    #[test]
    fn data_frame_roundtrip_preserves_stream_id_and_payload() {
        let f = SpdyFrame::data(7, b"hello".to_vec(), false);
        let bytes = f.encode();
        let (decoded, n) = SpdyFrame::decode(&bytes).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(decoded.kind, SpdyFrameKind::Data);
        assert_eq!(decoded.stream_id, 7);
        assert_eq!(decoded.payload, b"hello".to_vec());
        assert!(!decoded.is_fin());
    }

    #[test]
    fn data_frame_with_fin_flag_roundtrips() {
        let f = SpdyFrame::data(11, b"end".to_vec(), true);
        let (decoded, _) = SpdyFrame::decode(&f.encode()).unwrap();
        assert!(decoded.is_fin());
    }

    #[test]
    fn syn_stream_carries_stream_id_in_payload() {
        let f = SpdyFrame::syn_stream(13, false, true);
        let (decoded, _) = SpdyFrame::decode(&f.encode()).unwrap();
        assert_eq!(decoded.kind, SpdyFrameKind::SynStream);
        assert_eq!(decoded.stream_id, 13);
        assert_eq!(decoded.flags & FLAG_UNIDIRECTIONAL, FLAG_UNIDIRECTIONAL);
    }

    #[test]
    fn rst_stream_carries_status_code_in_payload() {
        let f = SpdyFrame::rst_stream(5, 6);
        let (decoded, _) = SpdyFrame::decode(&f.encode()).unwrap();
        assert_eq!(decoded.kind, SpdyFrameKind::RstStream);
        assert_eq!(decoded.stream_id, 5);
        let status = u32::from_be_bytes([decoded.payload[4], decoded.payload[5], decoded.payload[6], decoded.payload[7]]);
        assert_eq!(status, 6);
    }

    #[test]
    fn goaway_carries_last_good_stream_id() {
        let f = SpdyFrame::goaway(99, 0);
        let (decoded, _) = SpdyFrame::decode(&f.encode()).unwrap();
        assert_eq!(decoded.kind, SpdyFrameKind::GoAway);
        let last = u32::from_be_bytes([decoded.payload[0], decoded.payload[1], decoded.payload[2], decoded.payload[3]]);
        assert_eq!(last, 99);
    }

    #[test]
    fn decode_short_buffer_errors() {
        assert!(SpdyFrame::decode(&[0u8; 4]).is_err());
    }

    #[test]
    fn decode_truncated_payload_errors() {
        let f = SpdyFrame::data(1, vec![0u8; 100], false);
        let bytes = f.encode();
        let err = SpdyFrame::decode(&bytes[..bytes.len() - 5]).unwrap_err();
        assert!(err.to_string().contains("truncated"));
    }

    #[test]
    fn decode_unknown_control_type_errors() {
        // Manually craft a control frame with type 0xFFFF.
        let mut buf = Vec::new();
        let header: u32 = 0x8003_FFFF;
        buf.extend_from_slice(&header.to_be_bytes());
        buf.extend_from_slice(&[0u8, 0, 0, 0]); // flags=0, length=0
        let err = SpdyFrame::decode(&buf).unwrap_err();
        assert!(err.to_string().contains("unknown control"));
    }

    #[test]
    fn decode_wrong_version_errors() {
        let mut buf = Vec::new();
        let header: u32 = 0x8002_0001; // version 2, type SYN_STREAM
        buf.extend_from_slice(&header.to_be_bytes());
        buf.extend_from_slice(&[0u8, 0, 0, 0]);
        let err = SpdyFrame::decode(&buf).unwrap_err();
        assert!(err.to_string().contains("unsupported SPDY version"));
    }

    #[test]
    fn data_frame_high_bit_clear_in_encoding() {
        let f = SpdyFrame::data(0x1234, vec![], false);
        let bytes = f.encode();
        // First byte high bit must be 0 for data frames.
        assert_eq!(bytes[0] & 0x80, 0);
    }

    #[test]
    fn control_frame_high_bit_set_in_encoding() {
        let f = SpdyFrame::syn_stream(1, false, false);
        let bytes = f.encode();
        // First byte high bit must be 1 for control frames.
        assert_eq!(bytes[0] & 0x80, 0x80);
    }

    #[test]
    fn frame_kind_control_type_matches_spec_constants() {
        assert_eq!(SpdyFrameKind::SynStream.control_type(),    Some(0x0001));
        assert_eq!(SpdyFrameKind::SynReply.control_type(),     Some(0x0002));
        assert_eq!(SpdyFrameKind::RstStream.control_type(),    Some(0x0003));
        assert_eq!(SpdyFrameKind::Settings.control_type(),     Some(0x0004));
        assert_eq!(SpdyFrameKind::Ping.control_type(),         Some(0x0006));
        assert_eq!(SpdyFrameKind::GoAway.control_type(),       Some(0x0007));
        assert_eq!(SpdyFrameKind::Headers.control_type(),      Some(0x0008));
        assert_eq!(SpdyFrameKind::WindowUpdate.control_type(), Some(0x0009));
        assert_eq!(SpdyFrameKind::Data.control_type(),         None);
    }

    // ── StreamState transitions ──────────────────────────────────────────────

    #[test]
    fn state_idle_send_no_fin_becomes_open() {
        assert_eq!(StreamState::Idle.after_send(false).unwrap(), StreamState::Open);
    }

    #[test]
    fn state_idle_send_fin_becomes_half_closed_local() {
        assert_eq!(StreamState::Idle.after_send(true).unwrap(), StreamState::HalfClosedLocal);
    }

    #[test]
    fn state_open_recv_fin_becomes_half_closed_remote() {
        assert_eq!(StreamState::Open.after_recv(true).unwrap(), StreamState::HalfClosedRemote);
    }

    #[test]
    fn state_half_closed_local_send_errors() {
        assert!(StreamState::HalfClosedLocal.after_send(false).is_err());
    }

    #[test]
    fn state_half_closed_remote_recv_errors() {
        assert!(StreamState::HalfClosedRemote.after_recv(false).is_err());
    }

    #[test]
    fn state_closed_send_or_recv_errors() {
        assert!(StreamState::Closed.after_send(false).is_err());
        assert!(StreamState::Closed.after_recv(false).is_err());
    }

    #[test]
    fn rst_always_closes() {
        for s in [StreamState::Idle, StreamState::Open, StreamState::HalfClosedLocal, StreamState::HalfClosedRemote] {
            assert_eq!(s.after_rst(), StreamState::Closed);
        }
    }

    // ── StreamSession ────────────────────────────────────────────────────────

    #[test]
    fn open_stream_returns_odd_monotonic_ids() {
        let s = StreamSession::new();
        let a = s.open_stream("stdin").unwrap();
        let b = s.open_stream("stdout").unwrap();
        let c = s.open_stream("stderr").unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 3);
        assert_eq!(c, 5);
    }

    #[test]
    fn open_stream_after_goaway_errors() {
        let s = StreamSession::new();
        s.go_away(0);
        assert!(s.open_stream("x").is_err());
    }

    #[test]
    fn send_data_decrements_send_window() {
        let s = StreamSession::new();
        let id = s.open_stream("stdin").unwrap();
        let f = SpdyFrame::data(id, vec![0u8; 100], false);
        s.send(&f).unwrap();
        // Window starts at 64 KiB → 100 bytes consumed → 65436 remaining.
        // We can verify by sending a too-big follow-up.
        let too_big = SpdyFrame::data(id, vec![0u8; 100_000], false);
        let err = s.send(&too_big).unwrap_err();
        assert!(err.to_string().contains("send window underflow"));
    }

    #[test]
    fn send_with_fin_advances_state_to_half_closed_local() {
        let s = StreamSession::new();
        let id = s.open_stream("stdin").unwrap();
        s.send(&SpdyFrame::data(id, vec![], true)).unwrap();
        assert_eq!(s.state(id), Some(StreamState::HalfClosedLocal));
    }

    #[test]
    fn recv_on_unknown_stream_auto_registers() {
        let s = StreamSession::new();
        s.recv(&SpdyFrame::data(2, vec![1, 2, 3], false)).unwrap();
        assert_eq!(s.state(2), Some(StreamState::Open));
        assert_eq!(s.label(2).unwrap(), "remote");
    }

    #[test]
    fn rst_via_send_closes_stream() {
        let s = StreamSession::new();
        let id = s.open_stream("x").unwrap();
        s.send(&SpdyFrame::rst_stream(id, 0)).unwrap();
        assert_eq!(s.state(id), Some(StreamState::Closed));
    }

    #[test]
    fn ack_recv_emits_window_update_frame() {
        let s = StreamSession::new();
        let id = s.open_stream("stdout").unwrap();
        let frame = s.ack_recv(id, 1024).unwrap();
        assert_eq!(frame.kind, SpdyFrameKind::WindowUpdate);
        let delta = u32::from_be_bytes([frame.payload[4], frame.payload[5], frame.payload[6], frame.payload[7]]);
        assert_eq!(delta, 1024);
    }

    #[test]
    fn open_count_drops_when_streams_close() {
        let s = StreamSession::new();
        let a = s.open_stream("a").unwrap();
        let _b = s.open_stream("b").unwrap();
        assert_eq!(s.open_count(), 2);
        s.send(&SpdyFrame::rst_stream(a, 0)).unwrap();
        assert_eq!(s.open_count(), 1);
    }

    #[test]
    fn go_away_includes_largest_assigned_stream_id() {
        let s = StreamSession::new();
        s.open_stream("a").unwrap();
        s.open_stream("b").unwrap();
        let f = s.go_away(0);
        let last = u32::from_be_bytes([f.payload[0], f.payload[1], f.payload[2], f.payload[3]]);
        assert_eq!(last, 3);
    }

    #[test]
    fn send_to_unknown_stream_errors() {
        let s = StreamSession::new();
        let err = s.send(&SpdyFrame::data(99, vec![], false)).unwrap_err();
        assert!(err.to_string().contains("unknown stream"));
    }

    // ── Pty / Resize ─────────────────────────────────────────────────────────

    #[test]
    fn pty_winsize_default_is_80_by_24() {
        let w = PtyWinsize::default();
        assert_eq!(w.cols, 80);
        assert_eq!(w.rows, 24);
    }

    #[test]
    fn resize_message_pascal_case_serialization() {
        let m = ResizeMessage { width: 200, height: 60 };
        let bytes = m.encode();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("\"Width\":200"));
        assert!(s.contains("\"Height\":60"));
    }

    #[test]
    fn resize_message_decode_roundtrip() {
        let m = ResizeMessage { width: 100, height: 30 };
        let back = ResizeMessage::decode(&m.encode()).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn resize_message_decode_invalid_errors() {
        assert!(ResizeMessage::decode(b"garbage").is_err());
    }

    #[test]
    fn pty_to_resize_roundtrip() {
        let w = PtyWinsize { rows: 50, cols: 120, xpixel: 0, ypixel: 0 };
        let r: ResizeMessage = w.into();
        let back: PtyWinsize = r.into();
        assert_eq!(back.rows, w.rows);
        assert_eq!(back.cols, w.cols);
    }

    #[test]
    fn pty_resize_returns_previous_size() {
        let mut p = Pty::new(PtyWinsize::default());
        let prev = p.resize(ResizeMessage { width: 200, height: 60 });
        assert_eq!(prev, PtyWinsize::default());
        assert_eq!(p.winsize.cols, 200);
        assert_eq!(p.winsize.rows, 60);
    }

    // ── StreamErrorStatus ────────────────────────────────────────────────────

    #[test]
    fn non_zero_exit_status_carries_exit_code_and_reason() {
        let s = StreamErrorStatus::non_zero_exit(137);
        assert_eq!(s.status, "Failure");
        assert_eq!(s.reason, "NonZeroExitCode");
        assert_eq!(s.details.exit_code, Some(137));
        assert_eq!(s.details.causes[0].message, "137");
    }

    #[test]
    fn success_status_is_empty() {
        let s = StreamErrorStatus::success();
        assert_eq!(s.status, "Success");
        assert!(s.message.is_empty());
        assert!(s.details.causes.is_empty());
    }

    #[test]
    fn stream_error_status_roundtrips_through_json() {
        let s = StreamErrorStatus::non_zero_exit(2);
        let bytes = s.encode();
        let back = StreamErrorStatus::decode(&bytes).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn stream_error_status_decode_invalid_errors() {
        assert!(StreamErrorStatus::decode(b"not json").is_err());
    }

    #[test]
    fn stream_error_status_includes_status_field_in_json() {
        let s = StreamErrorStatus::non_zero_exit(1);
        let bytes = s.encode();
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(json.contains("\"status\":\"Failure\""));
        assert!(json.contains("\"reason\":\"NonZeroExitCode\""));
    }
}
