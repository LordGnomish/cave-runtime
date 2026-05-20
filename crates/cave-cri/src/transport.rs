// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SPDY/WebSocket byte transport for `exec` and `attach`.
//!
//! Cite: containerd v2.2.3 `pkg/cri/streaming/portforward/portforward.go`,
//! `pkg/cri/streaming/server.go` (delegates to upstream Kubernetes
//! `apimachinery/pkg/util/httpstream/spdy/{connection,upgrade}.go` and
//! `apimachinery/pkg/util/httpstream/wsstream/conn.go`). cave implements the
//! channel-byte framing used by both SPDY/3.1 data frames and WebSocket
//! binary frames so a single demultiplexer feeds the runtime exec pipes.
//!
//! Channel byte assignment matches kubectl exec / kubectl attach:
//!
//! | Channel | Direction      | Purpose                            |
//! |---------|----------------|------------------------------------|
//! | 0       | client→runtime | stdin                              |
//! | 1       | runtime→client | stdout                             |
//! | 2       | runtime→client | stderr                             |
//! | 3       | runtime→client | error (terminal status JSON)       |
//! | 4       | client→runtime | resize (TIOCSWINSZ JSON payload)   |

use crate::error::{CriError, CriResult};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const CHANNEL_STDIN: u8 = 0;
pub const CHANNEL_STDOUT: u8 = 1;
pub const CHANNEL_STDERR: u8 = 2;
pub const CHANNEL_ERROR: u8 = 3;
pub const CHANNEL_RESIZE: u8 = 4;

/// Cite: SPDY/3.1 "Data frame" structure
/// (`apimachinery/pkg/util/httpstream/spdy/connection.go`):
///
/// ```text
///  0                   1                   2                   3
///  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///  |C|       Stream-ID (31 bits)                                   |
///  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///  | Flags (8) |  Length (24 bits)                                 |
///  +---------------------------------------------------------------+
///  | Payload (Length bytes, with channel byte as payload[0])       |
///  +---------------------------------------------------------------+
/// ```
///
/// `C == 0` ⇒ data frame (we never emit control frames here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpdyFrame {
    pub stream_id: u32,
    pub flags: u8,
    pub channel: u8,
    pub data: Vec<u8>,
}

impl SpdyFrame {
    pub fn data(stream_id: u32, channel: u8, data: impl Into<Vec<u8>>) -> Self {
        Self {
            stream_id,
            flags: 0,
            channel,
            data: data.into(),
        }
    }

    /// Frame with the FIN flag set (last frame on a stream — channel close).
    /// See SPDY/3.1 §2.6.2 (`FLAG_FIN = 0x01`).
    pub fn fin(stream_id: u32, channel: u8) -> Self {
        Self {
            stream_id,
            flags: 0x01,
            channel,
            data: Vec::new(),
        }
    }

    /// Encode into the on-wire byte sequence. The `channel` byte is
    /// prepended to `data` per the Kubernetes/kubectl convention.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 1 + self.data.len());
        // Bit 31 = control flag (always 0 for data frames). Stream-ID is 31-bit.
        let sid = self.stream_id & 0x7FFF_FFFF;
        out.extend_from_slice(&sid.to_be_bytes());
        let len = (1 + self.data.len()) as u32;
        let flags_len = (self.flags as u32) << 24 | (len & 0x00FF_FFFF);
        out.extend_from_slice(&flags_len.to_be_bytes());
        out.push(self.channel);
        out.extend_from_slice(&self.data);
        out
    }

    /// Decode the next frame from a byte stream. Returns the frame and the
    /// number of bytes consumed.
    pub fn decode(buf: &[u8]) -> CriResult<(Self, usize)> {
        if buf.len() < 8 {
            return Err(CriError::Exec("spdy frame: header truncated".into()));
        }
        let sid_word = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if sid_word & 0x8000_0000 != 0 {
            return Err(CriError::Exec(
                "spdy frame: control frame not supported".into(),
            ));
        }
        let stream_id = sid_word & 0x7FFF_FFFF;
        let flags_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let flags = (flags_len >> 24) as u8;
        let payload_len = (flags_len & 0x00FF_FFFF) as usize;
        if buf.len() < 8 + payload_len {
            return Err(CriError::Exec("spdy frame: payload truncated".into()));
        }
        if payload_len == 0 {
            // FIN frame with no payload — treat as a synthetic close on
            // channel 0 (caller usually inspects flags directly).
            return Ok((
                Self {
                    stream_id,
                    flags,
                    channel: 0,
                    data: Vec::new(),
                },
                8,
            ));
        }
        let channel = buf[8];
        let data = buf[9..8 + payload_len].to_vec();
        Ok((
            Self {
                stream_id,
                flags,
                channel,
                data,
            },
            8 + payload_len,
        ))
    }
}

/// WebSocket binary-frame helper. cave does NOT speak the full WebSocket
/// protocol here — that lives in axum's tungstenite layer. We just encode
/// the per-message payload (channel byte + data) which axum then wraps in
/// a binary frame.
///
/// Cite: `apimachinery/pkg/util/httpstream/wsstream/conn.go` v1.36.0 — the
/// channel-byte convention is identical to SPDY.
pub fn ws_encode(channel: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + data.len());
    out.push(channel);
    out.extend_from_slice(data);
    out
}

pub fn ws_decode(payload: &[u8]) -> CriResult<(u8, &[u8])> {
    payload
        .split_first()
        .map(|(c, rest)| (*c, rest))
        .ok_or_else(|| CriError::Exec("ws frame: empty payload".into()))
}

/// Demultiplexer: stitches frames arriving on a single transport into
/// per-channel buffers. Used by the exec/attach session manager.
#[derive(Debug, Default, Clone)]
pub struct ChannelDemux {
    stdin: Vec<u8>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    error: Vec<u8>,
    resize: Vec<u8>,
    closed: [bool; 5],
}

impl ChannelDemux {
    pub fn feed(&mut self, channel: u8, data: &[u8]) -> CriResult<()> {
        match channel {
            CHANNEL_STDIN => self.stdin.extend_from_slice(data),
            CHANNEL_STDOUT => self.stdout.extend_from_slice(data),
            CHANNEL_STDERR => self.stderr.extend_from_slice(data),
            CHANNEL_ERROR => self.error.extend_from_slice(data),
            CHANNEL_RESIZE => self.resize.extend_from_slice(data),
            other => return Err(CriError::Exec(format!("unknown channel {}", other))),
        }
        Ok(())
    }

    pub fn close(&mut self, channel: u8) -> CriResult<()> {
        if (channel as usize) >= self.closed.len() {
            return Err(CriError::Exec(format!("unknown channel {}", channel)));
        }
        self.closed[channel as usize] = true;
        Ok(())
    }

    pub fn is_closed(&self, channel: u8) -> bool {
        self.closed.get(channel as usize).copied().unwrap_or(false)
    }

    pub fn stdin(&self) -> &[u8] {
        &self.stdin
    }
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
    pub fn error(&self) -> &[u8] {
        &self.error
    }
    pub fn resize(&self) -> &[u8] {
        &self.resize
    }
}

/// Tenant-scoped exec/attach session. Each container gets at most one
/// active session; cross-tenant lookups return an error.
///
/// Cite: containerd `pkg/cri/streaming/server.go` v2.2.3 — `Exec`/`Attach`
/// HTTP handlers delegate to a per-container session keyed by the
/// container ID + a request-scoped session ID.
#[derive(Debug)]
pub struct ExecSession {
    pub tenant_id: String,
    pub container_id: String,
    pub session_id: String,
    inbox: Mutex<VecDeque<SpdyFrame>>,
    demux: Mutex<ChannelDemux>,
    closed: Mutex<bool>,
}

impl ExecSession {
    pub fn new(
        tenant_id: impl Into<String>,
        container_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            tenant_id: tenant_id.into(),
            container_id: container_id.into(),
            session_id: session_id.into(),
            inbox: Mutex::new(VecDeque::new()),
            demux: Mutex::new(ChannelDemux::default()),
            closed: Mutex::new(false),
        })
    }

    /// Push a frame received from the client (stdin / resize).
    /// Cross-tenant pushes return `CriError::Exec`.
    pub fn push_from_client(&self, requesting_tenant: &str, frame: SpdyFrame) -> CriResult<()> {
        if requesting_tenant != self.tenant_id {
            return Err(CriError::Exec(format!(
                "cross-tenant push denied: session tenant '{}', request '{}'",
                self.tenant_id, requesting_tenant,
            )));
        }
        if *self.closed.lock().unwrap() {
            return Err(CriError::Exec("session closed".into()));
        }
        if frame.channel != CHANNEL_STDIN && frame.channel != CHANNEL_RESIZE {
            return Err(CriError::Exec(format!(
                "client may only write to channels stdin(0) and resize(4); got {}",
                frame.channel
            )));
        }
        let mut demux = self.demux.lock().unwrap();
        if frame.flags & 0x01 != 0 {
            demux.close(frame.channel)?;
        } else {
            demux.feed(frame.channel, &frame.data)?;
        }
        self.inbox.lock().unwrap().push_back(frame);
        Ok(())
    }

    /// Emit a frame to the client (stdout / stderr / error) — accepts a
    /// channel + bytes and queues the encoded SpdyFrame for the writer.
    pub fn emit_to_client(&self, channel: u8, data: impl Into<Vec<u8>>) -> CriResult<SpdyFrame> {
        if channel != CHANNEL_STDOUT && channel != CHANNEL_STDERR && channel != CHANNEL_ERROR {
            return Err(CriError::Exec(format!(
                "runtime may only write to channels stdout(1), stderr(2), error(3); got {}",
                channel
            )));
        }
        if *self.closed.lock().unwrap() {
            return Err(CriError::Exec("session closed".into()));
        }
        let frame = SpdyFrame::data(stream_id_for(&self.session_id, channel), channel, data);
        Ok(frame)
    }

    pub fn close(&self) {
        *self.closed.lock().unwrap() = true;
    }

    pub fn is_closed(&self) -> bool {
        *self.closed.lock().unwrap()
    }

    /// Read the bytes accumulated on a given channel by the demuxer.
    pub fn buffered(&self, channel: u8) -> Vec<u8> {
        let demux = self.demux.lock().unwrap();
        match channel {
            CHANNEL_STDIN => demux.stdin().to_vec(),
            CHANNEL_STDOUT => demux.stdout().to_vec(),
            CHANNEL_STDERR => demux.stderr().to_vec(),
            CHANNEL_ERROR => demux.error().to_vec(),
            CHANNEL_RESIZE => demux.resize().to_vec(),
            _ => Vec::new(),
        }
    }

    pub fn channel_closed(&self, channel: u8) -> bool {
        self.demux.lock().unwrap().is_closed(channel)
    }
}

/// Stable per-channel stream id derived from session id + channel byte.
/// Mirrors the kubectl convention of one SPDY stream per channel.
fn stream_id_for(session_id: &str, channel: u8) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in session_id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h ^= (channel as u64).wrapping_mul(0x9E37_79B9);
    // 31-bit positive stream id (high bit reserved for control frames)
    ((h as u32) & 0x7FFF_FFFF).max(1)
}
