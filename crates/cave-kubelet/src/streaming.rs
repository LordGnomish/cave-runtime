// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Streaming endpoints: exec, attach, port-forward.
//!
//! Mirrors `pkg/kubelet/cri/streaming` and the upstream streaming protocol
//! semantics: SPDY/3.1 and WebSocket-channel (v1–v5) upgrade negotiation,
//! multiplexed stream IDs (stdin=0, stdout=1, stderr=2, error=3, resize=4
//! plus v5 close=255), TTY resize message format, port-forward stream
//! pair-per-port, and request-level validation (stdin/stdout/stderr/tty
//! combinations, command presence, container name resolution).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamProtocol {
    /// SPDY/3.1 — historical default (`portforward.k8s.io`, `v4.channel.k8s.io`...).
    Spdy,
    /// WebSocket — modern path (`v4.channel.k8s.io`, `v5.channel.k8s.io`).
    WebSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WebSocketChannelVersion {
    V1,
    V2,
    V3,
    V4,
    V5,
}

impl WebSocketChannelVersion {
    pub fn subprotocol(self) -> &'static str {
        match self {
            WebSocketChannelVersion::V1 => "channel.k8s.io",
            WebSocketChannelVersion::V2 => "base64.channel.k8s.io",
            WebSocketChannelVersion::V3 => "v3.channel.k8s.io",
            WebSocketChannelVersion::V4 => "v4.channel.k8s.io",
            WebSocketChannelVersion::V5 => "v5.channel.k8s.io",
        }
    }

    pub fn from_subprotocol(s: &str) -> Option<Self> {
        match s {
            "channel.k8s.io" => Some(Self::V1),
            "base64.channel.k8s.io" => Some(Self::V2),
            "v3.channel.k8s.io" => Some(Self::V3),
            "v4.channel.k8s.io" => Some(Self::V4),
            "v5.channel.k8s.io" => Some(Self::V5),
            _ => None,
        }
    }

    /// Whether this version supports out-of-band TTY resize messages on stream 4.
    pub fn supports_resize(self) -> bool {
        // Resize stream introduced in v3 (legacy) and refined in v4.
        matches!(self, Self::V3 | Self::V4 | Self::V5)
    }

    /// Whether this version supports explicit close-stream messages
    /// (CLOSE channel). Only v5.
    pub fn supports_close(self) -> bool {
        matches!(self, Self::V5)
    }

    pub fn is_base64(self) -> bool {
        matches!(self, Self::V2)
    }
}

/// Subprotocol negotiation: select the best server-preferred protocol that
/// also appears in the client's offered list. K8s prefers v5 → v4 → v3 →
/// base64 → channel.
pub fn negotiate_subprotocol(client_offered: &[&str]) -> Option<WebSocketChannelVersion> {
    const PREFERENCE: &[WebSocketChannelVersion] = &[
        WebSocketChannelVersion::V5,
        WebSocketChannelVersion::V4,
        WebSocketChannelVersion::V3,
        WebSocketChannelVersion::V2,
        WebSocketChannelVersion::V1,
    ];
    for v in PREFERENCE {
        if client_offered.iter().any(|s| *s == v.subprotocol()) {
            return Some(*v);
        }
    }
    None
}

/// Stream IDs — match upstream kubelet's docs/proxy/streaming/portforward
/// channel layout. SPDY uses stream headers; WebSocket-channel uses a 1-byte
/// channel prefix.
pub mod channel_ids {
    pub const STDIN: u8 = 0;
    pub const STDOUT: u8 = 1;
    pub const STDERR: u8 = 2;
    /// Out-of-band error reporting (process-level errors, status messages).
    pub const ERROR: u8 = 3;
    /// TTY resize messages (introduced in v3+).
    pub const RESIZE: u8 = 4;
    /// V5: close-stream sentinel.
    pub const CLOSE: u8 = 255;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecRequest {
    pub container: String,
    pub command: Vec<String>,
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
    pub tty: bool,
}

impl ExecRequest {
    /// Upstream `pkg/kubelet/cri/streaming/remotecommand.options.fromQuery`
    /// validation: command required for exec; stdin/stdout/stderr — at least
    /// one true; tty requires stdout (else terminal is meaningless); tty
    /// disables stderr (output is multiplexed onto stdout PTY).
    pub fn validate(&self) -> Result<(), String> {
        if self.command.is_empty() {
            return Err("exec command required".into());
        }
        if !(self.stdin || self.stdout || self.stderr) {
            return Err("at least one of stdin/stdout/stderr required".into());
        }
        if self.tty && !self.stdout {
            return Err("tty requires stdout".into());
        }
        if self.tty && self.stderr {
            return Err("tty cannot be combined with stderr".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachRequest {
    pub container: String,
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
    pub tty: bool,
}

impl AttachRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.stdin || self.stdout || self.stderr) {
            return Err("at least one of stdin/stdout/stderr required".into());
        }
        if self.tty && !self.stdout {
            return Err("tty requires stdout".into());
        }
        if self.tty && self.stderr {
            return Err("tty cannot be combined with stderr".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForwardRequest {
    pub pod_uid: String,
    pub ports: Vec<u16>,
}

impl PortForwardRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.ports.is_empty() {
            return Err("at least one port required".into());
        }
        if self.ports.iter().any(|p| *p == 0) {
            return Err("port 0 is invalid".into());
        }
        let mut sorted = self.ports.clone();
        sorted.sort();
        for w in sorted.windows(2) {
            if w[0] == w[1] {
                return Err(format!("duplicate port: {}", w[0]));
            }
        }
        Ok(())
    }
}

/// Resize message body — JSON `{"Width": <cols>, "Height": <rows>}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    #[serde(rename = "Width")]
    pub width: u16,
    #[serde(rename = "Height")]
    pub height: u16,
}

impl TerminalSize {
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("infallible serialize")
    }
}

/// Per-port port-forward stream layout (SPDY): one DATA stream + one ERROR
/// stream paired by `requestID` and `port` headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForwardStreamPair {
    pub request_id: String,
    pub port: u16,
    pub data_stream_id: Option<u64>,
    pub error_stream_id: Option<u64>,
}

impl PortForwardStreamPair {
    pub fn new(request_id: &str, port: u16) -> Self {
        Self {
            request_id: request_id.to_string(),
            port,
            data_stream_id: None,
            error_stream_id: None,
        }
    }

    pub fn record_stream(&mut self, stream_id: u64, kind: PortForwardStreamKind) {
        match kind {
            PortForwardStreamKind::Data => self.data_stream_id = Some(stream_id),
            PortForwardStreamKind::Error => self.error_stream_id = Some(stream_id),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.data_stream_id.is_some() && self.error_stream_id.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardStreamKind {
    Data,
    Error,
}

impl PortForwardStreamKind {
    pub fn from_header(s: &str) -> Option<Self> {
        match s {
            "data" => Some(Self::Data),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn as_header(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Error => "error",
        }
    }
}

/// Encode a frame for v1/v3-v5 channel-prefixed protocols: `[channel_id][payload]`.
pub fn encode_channel_frame(channel: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + payload.len());
    out.push(channel);
    out.extend_from_slice(payload);
    out
}

/// Decode a frame; returns (channel, payload). Empty input → error.
pub fn decode_channel_frame(frame: &[u8]) -> Result<(u8, &[u8]), String> {
    if frame.is_empty() {
        return Err("empty frame".into());
    }
    Ok((frame[0], &frame[1..]))
}

/// V2 (base64) variant: payload is base64-encoded, but the channel prefix is
/// the *ASCII digit* of the channel ID (`'0'`, `'1'`, etc.) per upstream's
/// `wsstream/conn.go` Base64Codec.
pub fn encode_base64_frame(channel: u8, payload_b64: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + payload_b64.len());
    out.push(b'0' + channel);
    out.extend_from_slice(payload_b64.as_bytes());
    out
}

pub fn decode_base64_frame(frame: &[u8]) -> Result<(u8, &str), String> {
    if frame.is_empty() {
        return Err("empty frame".into());
    }
    let ch = frame[0];
    if !(b'0'..=b'9').contains(&ch) {
        return Err("invalid channel digit".into());
    }
    let payload = std::str::from_utf8(&frame[1..]).map_err(|e| e.to_string())?;
    Ok((ch - b'0', payload))
}

#[derive(Debug, Default)]
pub struct StreamingSession {
    pub protocol: Option<StreamProtocol>,
    pub ws_version: Option<WebSocketChannelVersion>,
    pub stdin_open: bool,
    pub stdout_open: bool,
    pub stderr_open: bool,
    pub error_open: bool,
    pub resize_open: bool,
    pub closed: bool,
}

impl StreamingSession {
    pub fn open_for_exec(req: &ExecRequest, version: WebSocketChannelVersion) -> Self {
        Self {
            protocol: Some(StreamProtocol::WebSocket),
            ws_version: Some(version),
            stdin_open: req.stdin,
            stdout_open: req.stdout,
            stderr_open: req.stderr && !req.tty,
            error_open: true,
            resize_open: req.tty && version.supports_resize(),
            closed: false,
        }
    }

    pub fn open_for_attach(req: &AttachRequest, version: WebSocketChannelVersion) -> Self {
        Self {
            protocol: Some(StreamProtocol::WebSocket),
            ws_version: Some(version),
            stdin_open: req.stdin,
            stdout_open: req.stdout,
            stderr_open: req.stderr && !req.tty,
            error_open: true,
            resize_open: req.tty && version.supports_resize(),
            closed: false,
        }
    }

    pub fn open_for_portforward() -> Self {
        Self {
            protocol: Some(StreamProtocol::Spdy),
            ws_version: None,
            stdin_open: false,
            stdout_open: false,
            stderr_open: false,
            error_open: true,
            resize_open: false,
            closed: false,
        }
    }

    pub fn close_stream(&mut self, channel: u8) {
        match channel {
            channel_ids::STDIN => self.stdin_open = false,
            channel_ids::STDOUT => self.stdout_open = false,
            channel_ids::STDERR => self.stderr_open = false,
            channel_ids::ERROR => self.error_open = false,
            channel_ids::RESIZE => self.resize_open = false,
            _ => {}
        }
        if !(self.stdin_open || self.stdout_open || self.stderr_open || self.error_open) {
            self.closed = true;
        }
    }

    pub fn handle_close_message(&mut self, channel: u8) -> Result<(), String> {
        if !self.ws_version.map(|v| v.supports_close()).unwrap_or(false) {
            return Err("close stream messages require v5.channel.k8s.io".into());
        }
        self.close_stream(channel);
        Ok(())
    }
}

/// SPDY upgrade detection: an HTTP `Upgrade` header equal to one of the
/// streaming protocol identifiers.
pub fn parse_upgrade_header(headers: &[(&str, &str)]) -> Option<StreamProtocol> {
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("Upgrade") {
            for token in v.split(',').map(|s| s.trim()) {
                if token.starts_with("SPDY/") || token == "SPDY/3.1" {
                    return Some(StreamProtocol::Spdy);
                }
                if token.eq_ignore_ascii_case("websocket") {
                    return Some(StreamProtocol::WebSocket);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use channel_ids as ch;

    #[test]
    fn ws_subprotocol_round_trip() {
        for v in [
            WebSocketChannelVersion::V1,
            WebSocketChannelVersion::V2,
            WebSocketChannelVersion::V3,
            WebSocketChannelVersion::V4,
            WebSocketChannelVersion::V5,
        ] {
            assert_eq!(WebSocketChannelVersion::from_subprotocol(v.subprotocol()), Some(v));
        }
    }

    #[test]
    fn ws_subprotocol_unknown_returns_none() {
        assert_eq!(WebSocketChannelVersion::from_subprotocol("foo"), None);
    }

    #[test]
    fn negotiate_prefers_v5() {
        let v = negotiate_subprotocol(&["v4.channel.k8s.io", "v5.channel.k8s.io"]);
        assert_eq!(v, Some(WebSocketChannelVersion::V5));
    }

    #[test]
    fn negotiate_falls_back_to_v4_when_v5_missing() {
        let v = negotiate_subprotocol(&["v4.channel.k8s.io", "channel.k8s.io"]);
        assert_eq!(v, Some(WebSocketChannelVersion::V4));
    }

    #[test]
    fn negotiate_unknown_returns_none() {
        assert_eq!(negotiate_subprotocol(&["websocket-unknown"]), None);
    }

    #[test]
    fn negotiate_picks_v1_only_when_alone() {
        let v = negotiate_subprotocol(&["channel.k8s.io"]);
        assert_eq!(v, Some(WebSocketChannelVersion::V1));
    }

    #[test]
    fn ws_resize_supported_v3_v4_v5() {
        assert!(WebSocketChannelVersion::V3.supports_resize());
        assert!(WebSocketChannelVersion::V4.supports_resize());
        assert!(WebSocketChannelVersion::V5.supports_resize());
        assert!(!WebSocketChannelVersion::V2.supports_resize());
        assert!(!WebSocketChannelVersion::V1.supports_resize());
    }

    #[test]
    fn ws_close_supported_only_v5() {
        assert!(WebSocketChannelVersion::V5.supports_close());
        for v in [
            WebSocketChannelVersion::V1,
            WebSocketChannelVersion::V2,
            WebSocketChannelVersion::V3,
            WebSocketChannelVersion::V4,
        ] {
            assert!(!v.supports_close());
        }
    }

    #[test]
    fn ws_v2_is_base64() {
        assert!(WebSocketChannelVersion::V2.is_base64());
        assert!(!WebSocketChannelVersion::V4.is_base64());
    }

    #[test]
    fn channel_id_constants_match_upstream() {
        assert_eq!(ch::STDIN, 0);
        assert_eq!(ch::STDOUT, 1);
        assert_eq!(ch::STDERR, 2);
        assert_eq!(ch::ERROR, 3);
        assert_eq!(ch::RESIZE, 4);
        assert_eq!(ch::CLOSE, 255);
    }

    #[test]
    fn exec_validate_requires_command() {
        let mut r = ExecRequest {
            container: "c".into(),
            command: vec![],
            stdin: false,
            stdout: true,
            stderr: false,
            tty: false,
        };
        assert!(r.validate().is_err());
        r.command = vec!["sh".into()];
        assert!(r.validate().is_ok());
    }

    #[test]
    fn exec_validate_requires_at_least_one_stream() {
        let r = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into()],
            stdin: false,
            stdout: false,
            stderr: false,
            tty: false,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn exec_validate_tty_requires_stdout() {
        let r = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into()],
            stdin: true,
            stdout: false,
            stderr: false,
            tty: true,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn exec_validate_tty_excludes_stderr() {
        let r = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into()],
            stdin: false,
            stdout: true,
            stderr: true,
            tty: true,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn exec_validate_happy_path() {
        let r = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into(), "-c".into(), "echo".into()],
            stdin: false,
            stdout: true,
            stderr: true,
            tty: false,
        };
        assert!(r.validate().is_ok());
    }

    #[test]
    fn attach_validate_rejects_no_streams() {
        let r = AttachRequest {
            container: "c".into(),
            stdin: false,
            stdout: false,
            stderr: false,
            tty: false,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn attach_validate_tty_rules() {
        let r = AttachRequest {
            container: "c".into(),
            stdin: false,
            stdout: false,
            stderr: false,
            tty: true,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn portforward_validate_requires_ports() {
        let r = PortForwardRequest { pod_uid: "p".into(), ports: vec![] };
        assert!(r.validate().is_err());
    }

    #[test]
    fn portforward_validate_rejects_zero_port() {
        let r = PortForwardRequest { pod_uid: "p".into(), ports: vec![0, 80] };
        assert!(r.validate().is_err());
    }

    #[test]
    fn portforward_validate_rejects_duplicates() {
        let r = PortForwardRequest { pod_uid: "p".into(), ports: vec![80, 80] };
        assert!(r.validate().is_err());
    }

    #[test]
    fn portforward_validate_accepts_unique_ports() {
        let r = PortForwardRequest { pod_uid: "p".into(), ports: vec![80, 443, 8080] };
        assert!(r.validate().is_ok());
    }

    #[test]
    fn terminal_size_round_trip() {
        let s = TerminalSize { width: 120, height: 40 };
        let json = s.encode();
        assert!(json.contains("\"Width\":120"));
        let s2 = TerminalSize::parse(&json).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn terminal_size_parse_invalid_json_errs() {
        assert!(TerminalSize::parse("{not json").is_err());
    }

    #[test]
    fn channel_frame_encode_decode_round_trip() {
        let frame = encode_channel_frame(ch::STDOUT, b"hello");
        let (c, p) = decode_channel_frame(&frame).unwrap();
        assert_eq!(c, ch::STDOUT);
        assert_eq!(p, b"hello");
    }

    #[test]
    fn decode_channel_frame_empty_errs() {
        assert!(decode_channel_frame(&[]).is_err());
    }

    #[test]
    fn channel_frame_encode_zero_payload() {
        let f = encode_channel_frame(ch::STDIN, b"");
        let (c, p) = decode_channel_frame(&f).unwrap();
        assert_eq!(c, ch::STDIN);
        assert_eq!(p, b"");
    }

    #[test]
    fn base64_frame_uses_ascii_digit_prefix() {
        let f = encode_base64_frame(ch::STDOUT, "aGVsbG8=");
        assert_eq!(f[0], b'1');
    }

    #[test]
    fn base64_frame_decode_round_trip() {
        let f = encode_base64_frame(ch::STDERR, "ZXJyb3I=");
        let (c, p) = decode_base64_frame(&f).unwrap();
        assert_eq!(c, ch::STDERR);
        assert_eq!(p, "ZXJyb3I=");
    }

    #[test]
    fn base64_frame_decode_rejects_non_digit_prefix() {
        let f: Vec<u8> = b"x...".to_vec();
        assert!(decode_base64_frame(&f).is_err());
    }

    #[test]
    fn portforward_stream_pair_completes_with_two_streams() {
        let mut p = PortForwardStreamPair::new("rid-1", 8080);
        assert!(!p.is_complete());
        p.record_stream(1, PortForwardStreamKind::Data);
        assert!(!p.is_complete());
        p.record_stream(2, PortForwardStreamKind::Error);
        assert!(p.is_complete());
    }

    #[test]
    fn portforward_stream_kind_header_round_trip() {
        assert_eq!(PortForwardStreamKind::from_header("data"), Some(PortForwardStreamKind::Data));
        assert_eq!(PortForwardStreamKind::from_header("error"), Some(PortForwardStreamKind::Error));
        assert_eq!(PortForwardStreamKind::from_header("nope"), None);
        assert_eq!(PortForwardStreamKind::Data.as_header(), "data");
        assert_eq!(PortForwardStreamKind::Error.as_header(), "error");
    }

    #[test]
    fn streaming_session_open_for_exec_no_tty_opens_stderr() {
        let req = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into()],
            stdin: true,
            stdout: true,
            stderr: true,
            tty: false,
        };
        let s = StreamingSession::open_for_exec(&req, WebSocketChannelVersion::V4);
        assert!(s.stdin_open);
        assert!(s.stdout_open);
        assert!(s.stderr_open);
        assert!(s.error_open);
        assert!(!s.resize_open);
    }

    #[test]
    fn streaming_session_open_for_exec_tty_disables_stderr_enables_resize() {
        let req = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into()],
            stdin: true,
            stdout: true,
            stderr: false,
            tty: true,
        };
        let s = StreamingSession::open_for_exec(&req, WebSocketChannelVersion::V5);
        assert!(s.stdout_open);
        assert!(!s.stderr_open);
        assert!(s.resize_open);
    }

    #[test]
    fn streaming_session_close_stream_closes_individually() {
        let mut s = StreamingSession::open_for_attach(
            &AttachRequest {
                container: "c".into(),
                stdin: true,
                stdout: true,
                stderr: true,
                tty: false,
            },
            WebSocketChannelVersion::V4,
        );
        s.close_stream(ch::STDIN);
        assert!(!s.stdin_open);
        assert!(s.stdout_open);
        assert!(!s.closed);
    }

    #[test]
    fn streaming_session_closed_when_all_streams_closed() {
        let mut s = StreamingSession::open_for_attach(
            &AttachRequest {
                container: "c".into(),
                stdin: true,
                stdout: true,
                stderr: false,
                tty: false,
            },
            WebSocketChannelVersion::V4,
        );
        s.close_stream(ch::STDIN);
        s.close_stream(ch::STDOUT);
        s.close_stream(ch::ERROR);
        assert!(s.closed);
    }

    #[test]
    fn streaming_session_handle_close_v4_errors() {
        let mut s = StreamingSession::open_for_exec(
            &ExecRequest {
                container: "c".into(),
                command: vec!["sh".into()],
                stdin: true,
                stdout: true,
                stderr: false,
                tty: false,
            },
            WebSocketChannelVersion::V4,
        );
        assert!(s.handle_close_message(ch::STDIN).is_err());
    }

    #[test]
    fn streaming_session_handle_close_v5_works() {
        let mut s = StreamingSession::open_for_exec(
            &ExecRequest {
                container: "c".into(),
                command: vec!["sh".into()],
                stdin: true,
                stdout: true,
                stderr: false,
                tty: false,
            },
            WebSocketChannelVersion::V5,
        );
        s.handle_close_message(ch::STDIN).unwrap();
        assert!(!s.stdin_open);
    }

    #[test]
    fn streaming_session_open_for_portforward_uses_spdy() {
        let s = StreamingSession::open_for_portforward();
        assert_eq!(s.protocol, Some(StreamProtocol::Spdy));
    }

    #[test]
    fn parse_upgrade_header_spdy() {
        assert_eq!(
            parse_upgrade_header(&[("Upgrade", "SPDY/3.1")]),
            Some(StreamProtocol::Spdy)
        );
    }

    #[test]
    fn parse_upgrade_header_websocket() {
        assert_eq!(
            parse_upgrade_header(&[("Upgrade", "websocket")]),
            Some(StreamProtocol::WebSocket)
        );
    }

    #[test]
    fn parse_upgrade_header_case_insensitive() {
        assert_eq!(
            parse_upgrade_header(&[("upgrade", "WEBSOCKET")]),
            Some(StreamProtocol::WebSocket)
        );
    }

    #[test]
    fn parse_upgrade_header_returns_none_when_absent() {
        assert!(parse_upgrade_header(&[("Connection", "keep-alive")]).is_none());
    }

    #[test]
    fn parse_upgrade_header_handles_multiple_tokens() {
        assert_eq!(
            parse_upgrade_header(&[("Upgrade", "websocket, h2c")]),
            Some(StreamProtocol::WebSocket)
        );
    }

    #[test]
    fn channel_frame_high_channel_value() {
        let f = encode_channel_frame(ch::CLOSE, b"x");
        let (c, _) = decode_channel_frame(&f).unwrap();
        assert_eq!(c, 255);
    }

    #[test]
    fn portforward_stream_pair_overwrite() {
        let mut p = PortForwardStreamPair::new("r", 80);
        p.record_stream(10, PortForwardStreamKind::Data);
        p.record_stream(20, PortForwardStreamKind::Data);
        assert_eq!(p.data_stream_id, Some(20));
    }

    #[test]
    fn exec_v3_supports_resize_with_tty() {
        let req = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into()],
            stdin: false,
            stdout: true,
            stderr: false,
            tty: true,
        };
        let s = StreamingSession::open_for_exec(&req, WebSocketChannelVersion::V3);
        assert!(s.resize_open);
    }

    #[test]
    fn exec_v1_no_resize_even_with_tty() {
        let req = ExecRequest {
            container: "c".into(),
            command: vec!["sh".into()],
            stdin: false,
            stdout: true,
            stderr: false,
            tty: true,
        };
        let s = StreamingSession::open_for_exec(&req, WebSocketChannelVersion::V1);
        assert!(!s.resize_open);
    }

    #[test]
    fn streaming_session_default_protocol_none() {
        let s = StreamingSession::default();
        assert!(s.protocol.is_none());
    }
}
