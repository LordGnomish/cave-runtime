// SPDX-License-Identifier: AGPL-3.0-or-later
//! Streaming protocol — exec / attach / portforward multiplexing.
//!
//! Kubernetes wraps remote command execution in a multi-stream channel
//! protocol. Two transports exist:
//!
//! - **SPDY v4** (legacy) — multiple SPDY streams per connection, one per
//!   stdio channel.
//! - **WebSocket** (`channel.k8s.io.v5` subprotocol) — frames are
//!   `[channel_byte] [data_bytes...]`, with the channel byte indexing into
//!   the same stdio table.
//!
//! cave-cri implements the v5 channel encoding (it is what kubectl's
//! `exec` / `attach` / `port-forward` use today) and exposes the channel
//! layout for both upstream-named tests and the future SPDY shim.
//!
//! Channel layout (matches `kubernetes/pkg/kubelet/cri/streaming/remotecommand`
//! and `apimachinery/pkg/util/httpstream`):
//!
//! | byte | direction         | semantics                |
//! |------|-------------------|--------------------------|
//! | 0    | client → server   | stdin                    |
//! | 1    | server → client   | stdout                   |
//! | 2    | server → client   | stderr                   |
//! | 3    | server → client   | error / exit info        |
//! | 4    | client → server   | resize (TTY window size) |
//! | 5    | server → client   | close (port-forward)     |
//!
//! Port-forward uses an even/odd pairing where data streams live on even
//! channels and the matching error stream is the next odd channel.

use crate::error::{CriError, CriResult};
use serde::{Deserialize, Serialize};

/// Channel byte → semantic stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Channel {
    Stdin = 0,
    Stdout = 1,
    Stderr = 2,
    Error = 3,
    Resize = 4,
    Close = 5,
}

impl Channel {
    pub const fn as_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(b: u8) -> Option<Channel> {
        Some(match b {
            0 => Channel::Stdin,
            1 => Channel::Stdout,
            2 => Channel::Stderr,
            3 => Channel::Error,
            4 => Channel::Resize,
            5 => Channel::Close,
            _ => return None,
        })
    }

    /// Some channels flow only one way. Returns true for stdin / resize
    /// (client → server) and stdout / stderr / error / close
    /// (server → client).
    pub fn is_client_to_server(self) -> bool {
        matches!(self, Channel::Stdin | Channel::Resize)
    }
}

/// One frame on the multiplexed stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel: Channel,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn new(channel: Channel, data: impl Into<Vec<u8>>) -> Self {
        Self { channel, data: data.into() }
    }

    /// Encode as a `channel.k8s.io.v5` frame.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.data.len() + 1);
        buf.push(self.channel.as_byte());
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Decode a `channel.k8s.io.v5` frame.
    pub fn decode(bytes: &[u8]) -> CriResult<Frame> {
        if bytes.is_empty() {
            return Err(CriError::Runtime("empty stream frame".into()));
        }
        let channel = Channel::from_byte(bytes[0])
            .ok_or_else(|| CriError::Runtime(format!("unknown channel byte: {}", bytes[0])))?;
        Ok(Frame { channel, data: bytes[1..].to_vec() })
    }
}

/// Negotiated transport for the exec / attach session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamProtocol {
    /// `channel.k8s.io.v5` over WebSocket.
    WebSocketV5,
    /// SPDY v4 binary streams (legacy; wire format uses one SPDY stream
    /// per channel byte).
    SpdyV4,
}

impl StreamProtocol {
    /// Subprotocol string sent by the client during the WebSocket / SPDY
    /// upgrade.
    pub fn subprotocol(self) -> &'static str {
        match self {
            StreamProtocol::WebSocketV5 => "v5.channel.k8s.io",
            StreamProtocol::SpdyV4 => "v4.streamprotocol.k8s.io",
        }
    }

    /// Best-effort negotiation from a comma-separated `Sec-WebSocket-Protocol`
    /// or `X-Stream-Protocol-Version` header value. Returns the highest
    /// protocol the server supports that the client offered.
    pub fn negotiate(client_offer: &str) -> Option<StreamProtocol> {
        for proto in client_offer.split(',') {
            let proto = proto.trim();
            if proto == StreamProtocol::WebSocketV5.subprotocol() {
                return Some(StreamProtocol::WebSocketV5);
            }
            if proto == StreamProtocol::SpdyV4.subprotocol() {
                return Some(StreamProtocol::SpdyV4);
            }
        }
        None
    }
}

/// TTY window size signal sent on the resize channel as a JSON object:
/// `{"Width":80,"Height":24}` (matches client-go's TerminalSize).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TtyWindowSize {
    pub width: u16,
    pub height: u16,
}

impl TtyWindowSize {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> CriResult<TtyWindowSize> {
        serde_json::from_slice(bytes)
            .map_err(|e| CriError::Runtime(format!("invalid TTY resize payload: {}", e)))
    }
}

/// Exec session configuration carried in `ExecRequest`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecStreamSpec {
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
    pub tty: bool,
    /// Initial TTY size if a TTY is requested.
    pub initial_size: Option<TtyWindowSize>,
}

/// Port-forward channel allocation. Each forwarded port gets a
/// `(data, error)` stream pair. `data` carries the raw TCP bytes, `error`
/// carries human-readable error text and is closed when the port is done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForwardChannel {
    pub port: u16,
    pub data_channel: u8,
    pub error_channel: u8,
}

impl PortForwardChannel {
    /// Allocate channels for `port` given a 0-based `index` in the request.
    /// Channel layout matches kube's portforward proxy: data on `2*i`,
    /// error on `2*i + 1`.
    pub fn allocate(port: u16, index: usize) -> Self {
        Self {
            port,
            data_channel: (index * 2) as u8,
            error_channel: (index * 2 + 1) as u8,
        }
    }
}

/// Build the per-session response shape returned by the streaming
/// front-end (the actual upgrade is performed by the network layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingURL {
    pub url: String,
    pub protocols: Vec<String>,
    pub timeout_seconds: u32,
}

impl StreamingURL {
    pub fn for_exec(container_id: uuid::Uuid) -> Self {
        Self {
            url: format!("/api/cri/containers/{}/exec/ws", container_id),
            protocols: vec![
                StreamProtocol::WebSocketV5.subprotocol().to_string(),
                StreamProtocol::SpdyV4.subprotocol().to_string(),
            ],
            timeout_seconds: 30,
        }
    }

    pub fn for_attach(container_id: uuid::Uuid) -> Self {
        Self {
            url: format!("/api/cri/containers/{}/attach/ws", container_id),
            protocols: vec![
                StreamProtocol::WebSocketV5.subprotocol().to_string(),
                StreamProtocol::SpdyV4.subprotocol().to_string(),
            ],
            timeout_seconds: 30,
        }
    }

    pub fn for_portforward(sandbox_id: uuid::Uuid) -> Self {
        Self {
            url: format!("/api/cri/sandboxes/{}/portforward/ws", sandbox_id),
            protocols: vec!["portforward.k8s.io".into()],
            timeout_seconds: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Channel ───────────────────────────────────────────────────────────────

    #[test]
    fn channel_byte_values_are_stable() {
        assert_eq!(Channel::Stdin.as_byte(), 0);
        assert_eq!(Channel::Stdout.as_byte(), 1);
        assert_eq!(Channel::Stderr.as_byte(), 2);
        assert_eq!(Channel::Error.as_byte(), 3);
        assert_eq!(Channel::Resize.as_byte(), 4);
        assert_eq!(Channel::Close.as_byte(), 5);
    }

    #[test]
    fn channel_from_byte_known() {
        for c in [Channel::Stdin, Channel::Stdout, Channel::Stderr,
                  Channel::Error, Channel::Resize, Channel::Close] {
            assert_eq!(Channel::from_byte(c.as_byte()), Some(c));
        }
    }

    #[test]
    fn channel_from_byte_unknown_returns_none() {
        assert!(Channel::from_byte(99).is_none());
        assert!(Channel::from_byte(7).is_none());
    }

    #[test]
    fn channel_direction_matches_spec() {
        assert!(Channel::Stdin.is_client_to_server());
        assert!(Channel::Resize.is_client_to_server());
        assert!(!Channel::Stdout.is_client_to_server());
        assert!(!Channel::Stderr.is_client_to_server());
        assert!(!Channel::Error.is_client_to_server());
        assert!(!Channel::Close.is_client_to_server());
    }

    // ── Frame encode/decode ───────────────────────────────────────────────────

    #[test]
    fn frame_encode_prepends_channel_byte() {
        let f = Frame::new(Channel::Stdout, b"hello".to_vec());
        let encoded = f.encode();
        assert_eq!(encoded[0], Channel::Stdout.as_byte());
        assert_eq!(&encoded[1..], b"hello");
    }

    #[test]
    fn frame_decode_recovers_channel_and_data() {
        let bytes = vec![Channel::Stderr.as_byte(), b'!', b'!'];
        let f = Frame::decode(&bytes).unwrap();
        assert_eq!(f.channel, Channel::Stderr);
        assert_eq!(f.data, b"!!".to_vec());
    }

    #[test]
    fn frame_roundtrip_preserves_payload() {
        let original = Frame::new(Channel::Stdin, vec![1, 2, 3, 4, 5]);
        let decoded = Frame::decode(&original.encode()).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn frame_decode_empty_errors() {
        assert!(Frame::decode(&[]).is_err());
    }

    #[test]
    fn frame_decode_unknown_channel_errors() {
        let bytes = vec![88, b'x'];
        let err = Frame::decode(&bytes).unwrap_err();
        assert!(err.to_string().contains("88"));
    }

    #[test]
    fn frame_decode_zero_payload_returns_empty_data() {
        let bytes = vec![Channel::Stdout.as_byte()];
        let f = Frame::decode(&bytes).unwrap();
        assert_eq!(f.channel, Channel::Stdout);
        assert!(f.data.is_empty());
    }

    // ── StreamProtocol negotiation ────────────────────────────────────────────

    #[test]
    fn negotiate_picks_websocket_v5_when_offered_first() {
        let p = StreamProtocol::negotiate("v5.channel.k8s.io,v4.streamprotocol.k8s.io");
        assert_eq!(p, Some(StreamProtocol::WebSocketV5));
    }

    #[test]
    fn negotiate_falls_back_to_spdy_v4() {
        let p = StreamProtocol::negotiate("v4.streamprotocol.k8s.io");
        assert_eq!(p, Some(StreamProtocol::SpdyV4));
    }

    #[test]
    fn negotiate_unknown_returns_none() {
        assert!(StreamProtocol::negotiate("base64.binary.k8s.io").is_none());
    }

    #[test]
    fn negotiate_handles_whitespace() {
        let p = StreamProtocol::negotiate("  v5.channel.k8s.io  ");
        assert_eq!(p, Some(StreamProtocol::WebSocketV5));
    }

    #[test]
    fn subprotocol_strings_are_official() {
        assert_eq!(StreamProtocol::WebSocketV5.subprotocol(), "v5.channel.k8s.io");
        assert_eq!(StreamProtocol::SpdyV4.subprotocol(), "v4.streamprotocol.k8s.io");
    }

    // ── TtyWindowSize ─────────────────────────────────────────────────────────

    #[test]
    fn tty_window_size_encodes_to_pascal_case_json() {
        let size = TtyWindowSize { width: 120, height: 40 };
        let json = String::from_utf8(size.encode()).unwrap();
        assert!(json.contains("\"Width\":120"));
        assert!(json.contains("\"Height\":40"));
    }

    #[test]
    fn tty_window_size_decode_roundtrip() {
        let size = TtyWindowSize { width: 80, height: 24 };
        let bytes = size.encode();
        let back = TtyWindowSize::decode(&bytes).unwrap();
        assert_eq!(size, back);
    }

    #[test]
    fn tty_window_size_decode_invalid_errors() {
        assert!(TtyWindowSize::decode(b"not json").is_err());
    }

    // ── ExecStreamSpec ────────────────────────────────────────────────────────

    #[test]
    fn exec_stream_spec_default_is_no_streams() {
        let s = ExecStreamSpec::default();
        assert!(!s.stdin);
        assert!(!s.stdout);
        assert!(!s.stderr);
        assert!(!s.tty);
    }

    #[test]
    fn exec_stream_spec_serializes() {
        let s = ExecStreamSpec {
            stdin: true,
            stdout: true,
            stderr: false,
            tty: true,
            initial_size: Some(TtyWindowSize { width: 200, height: 60 }),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ExecStreamSpec = serde_json::from_str(&json).unwrap();
        assert!(back.stdin);
        assert!(back.tty);
        assert_eq!(back.initial_size.unwrap().width, 200);
    }

    // ── PortForwardChannel ────────────────────────────────────────────────────

    #[test]
    fn port_forward_channels_zero_index_uses_0_and_1() {
        let p = PortForwardChannel::allocate(8080, 0);
        assert_eq!(p.data_channel, 0);
        assert_eq!(p.error_channel, 1);
    }

    #[test]
    fn port_forward_channels_subsequent_indices_step_by_two() {
        let p1 = PortForwardChannel::allocate(8080, 1);
        assert_eq!(p1.data_channel, 2);
        assert_eq!(p1.error_channel, 3);
        let p2 = PortForwardChannel::allocate(443, 2);
        assert_eq!(p2.data_channel, 4);
        assert_eq!(p2.error_channel, 5);
    }

    #[test]
    fn port_forward_channels_remember_port() {
        let p = PortForwardChannel::allocate(9090, 3);
        assert_eq!(p.port, 9090);
    }

    // ── StreamingURL ─────────────────────────────────────────────────────────

    #[test]
    fn streaming_url_for_exec_includes_container_id() {
        let id = uuid::Uuid::new_v4();
        let u = StreamingURL::for_exec(id);
        assert!(u.url.contains(&id.to_string()));
        assert!(u.url.ends_with("/exec/ws"));
        assert!(u.protocols.contains(&"v5.channel.k8s.io".to_string()));
    }

    #[test]
    fn streaming_url_for_attach_includes_container_id() {
        let id = uuid::Uuid::new_v4();
        let u = StreamingURL::for_attach(id);
        assert!(u.url.ends_with("/attach/ws"));
        assert!(u.protocols.iter().any(|p| p.contains("k8s.io")));
    }

    #[test]
    fn streaming_url_for_portforward_uses_sandbox_id() {
        let id = uuid::Uuid::new_v4();
        let u = StreamingURL::for_portforward(id);
        assert!(u.url.contains("/sandboxes/"));
        assert!(u.url.contains("/portforward/ws"));
        assert_eq!(u.protocols, vec!["portforward.k8s.io".to_string()]);
    }
}
