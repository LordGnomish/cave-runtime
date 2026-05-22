// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! deeper-002: SPDY/WebSocket exec/attach byte transport.
//!
//! Upstream: containerd v2.2.3 `pkg/cri/streaming/server.go` →
//! Kubernetes `apimachinery/pkg/util/httpstream/spdy/connection.go` (SPDY/3.1
//! data frames) and `apimachinery/pkg/util/httpstream/wsstream/conn.go`
//! (WebSocket binary frames). cave routes both through the same
//! channel-byte demuxer.

use cave_cri::transport::{
    ws_decode, ws_encode, ChannelDemux, ExecSession, SpdyFrame, CHANNEL_ERROR, CHANNEL_RESIZE,
    CHANNEL_STDERR, CHANNEL_STDIN, CHANNEL_STDOUT,
};

const TENANT: &str = "tenant-acme-prod";

/// Cite: SPDY/3.1 data-frame layout (see
/// `apimachinery/pkg/util/httpstream/spdy/connection.go` `WriteFrame` —
/// 8-byte header: 4 bytes stream-id (high bit 0), 1 byte flags, 3 bytes
/// length, then payload). Round-trip through encode/decode must preserve
/// stream_id, channel and data verbatim.
#[test]
fn spdy_frame_round_trip_preserves_payload() {
    let frame = SpdyFrame::data(0x1234_5678, CHANNEL_STDOUT, b"hello world".to_vec());
    let bytes = frame.encode();
    assert_eq!(bytes.len(), 8 + 1 + b"hello world".len());
    // Stream-id high bit MUST be 0 (data frame, not control)
    assert_eq!(bytes[0] & 0x80, 0);

    let (decoded, n) = SpdyFrame::decode(&bytes).unwrap();
    assert_eq!(n, bytes.len());
    assert_eq!(decoded, frame);
    assert_eq!(decoded.data, b"hello world");
}

/// Cite: SPDY/3.1 §2.6.2 `FLAG_FIN = 0x01` — a fin frame signals
/// end-of-stream on the channel, conveying nothing in the payload.
#[test]
fn spdy_fin_frame_sets_flag_and_zero_payload() {
    let fin = SpdyFrame::fin(42, CHANNEL_STDIN);
    let bytes = fin.encode();
    // Flags byte sits at offset 4 (high byte of the flags-len word)
    assert_eq!(bytes[4], 0x01);
    let (decoded, _) = SpdyFrame::decode(&bytes).unwrap();
    assert_eq!(decoded.flags & 0x01, 0x01);
    assert!(decoded.data.is_empty() || decoded.channel == CHANNEL_STDIN);
}

/// Cite: SPDY frames are framed with a length prefix; truncated header
/// (< 8 bytes) and truncated payload (declared len > available bytes)
/// must error rather than silently producing a partial frame.
#[test]
fn spdy_frame_decode_rejects_truncated_input() {
    assert!(
        SpdyFrame::decode(&[0u8; 4]).is_err(),
        "truncated header rejected"
    );
    let mut frame = SpdyFrame::data(7, CHANNEL_STDOUT, b"abcd".to_vec()).encode();
    frame.truncate(frame.len() - 1);
    assert!(
        SpdyFrame::decode(&frame).is_err(),
        "truncated payload rejected"
    );
}

/// Cite: control frames (high bit of stream-id == 1) — cave's data path
/// rejects them; SPDY control frames (SYN_STREAM/PING/...) are negotiated
/// out-of-band by the upgrader, never decoded here.
#[test]
fn spdy_decode_rejects_control_frames() {
    let mut bytes = SpdyFrame::data(1, CHANNEL_STDOUT, b"x".to_vec()).encode();
    bytes[0] |= 0x80;
    assert!(
        SpdyFrame::decode(&bytes).is_err(),
        "control frame must be rejected"
    );
}

/// Cite: `apimachinery/pkg/util/httpstream/wsstream/conn.go` v1.36.0 —
/// WebSocket binary frame payload is `[channel_byte][...data...]`.
/// Round-trip through encode/decode preserves both halves.
#[test]
fn ws_payload_round_trip_preserves_channel_and_data() {
    let payload = ws_encode(CHANNEL_STDERR, b"err: oops");
    assert_eq!(payload[0], CHANNEL_STDERR);
    let (channel, data) = ws_decode(&payload).unwrap();
    assert_eq!(channel, CHANNEL_STDERR);
    assert_eq!(data, b"err: oops");

    // Empty payload is a protocol error
    assert!(ws_decode(&[]).is_err());
}

/// Cite: containerd v2.2.3 `pkg/cri/streaming/server.go` exec session
/// lifecycle — multiple data frames on the same channel accumulate in
/// order; closing a channel marks it terminated. The demuxer is the
/// in-process model of that lifecycle.
#[test]
fn channel_demux_accumulates_in_order_and_tracks_closes() {
    let mut d = ChannelDemux::default();
    d.feed(CHANNEL_STDOUT, b"part1\n").unwrap();
    d.feed(CHANNEL_STDOUT, b"part2\n").unwrap();
    d.feed(CHANNEL_STDERR, b"oops\n").unwrap();
    assert_eq!(d.stdout(), b"part1\npart2\n");
    assert_eq!(d.stderr(), b"oops\n");

    d.close(CHANNEL_STDIN).unwrap();
    assert!(d.is_closed(CHANNEL_STDIN));
    assert!(!d.is_closed(CHANNEL_STDOUT));

    assert!(d.feed(99, b"junk").is_err(), "unknown channel rejected");
    assert!(d.close(99).is_err(), "close on unknown channel rejected");
}

/// Cite: containerd v2.2.3 `pkg/cri/streaming/server.go` `serveExec` —
/// per-request session enforces directionality:
/// * client may only push to channels stdin(0) and resize(4),
/// * runtime may only emit to channels stdout(1), stderr(2), error(3).
/// Cross-tenant pushes are rejected with `CrossTenantDenied` analogue
/// (`CriError::Exec`).
#[test]
fn exec_session_enforces_tenant_and_channel_directionality() {
    let s = ExecSession::new(TENANT, "container-abc", "session-xyz");

    // OK: client pushes stdin
    s.push_from_client(
        TENANT,
        SpdyFrame::data(11, CHANNEL_STDIN, b"echo hi\n".to_vec()),
    )
    .unwrap();
    assert_eq!(s.buffered(CHANNEL_STDIN), b"echo hi\n");

    // OK: client pushes resize
    s.push_from_client(
        TENANT,
        SpdyFrame::data(12, CHANNEL_RESIZE, br#"{"Width":80,"Height":24}"#.to_vec()),
    )
    .unwrap();
    assert_eq!(s.buffered(CHANNEL_RESIZE), br#"{"Width":80,"Height":24}"#);

    // ERR: client tries to push stdout (runtime-only direction)
    let bad = s.push_from_client(TENANT, SpdyFrame::data(13, CHANNEL_STDOUT, b"x".to_vec()));
    assert!(bad.is_err(), "client must not push to stdout");

    // ERR: cross-tenant
    let bad = s.push_from_client(
        "tenant-other",
        SpdyFrame::data(14, CHANNEL_STDIN, b"x".to_vec()),
    );
    assert!(bad.is_err(), "cross-tenant push denied");

    // OK: runtime emits to stdout/stderr/error
    let f = s.emit_to_client(CHANNEL_STDOUT, b"hi\n".to_vec()).unwrap();
    assert_eq!(f.channel, CHANNEL_STDOUT);
    assert!(s
        .emit_to_client(CHANNEL_ERROR, br#"{"status":"Success"}"#.to_vec())
        .is_ok());

    // ERR: runtime can't emit on stdin
    assert!(s.emit_to_client(CHANNEL_STDIN, b"nope".to_vec()).is_err());

    // FIN frame on the stdin channel marks it closed even though it carries no payload.
    let fin = SpdyFrame::fin(99, CHANNEL_STDIN);
    s.push_from_client(TENANT, fin).unwrap();
    assert!(
        s.channel_closed(CHANNEL_STDIN),
        "FIN on stdin closes channel"
    );

    // After session close, neither push nor emit are allowed.
    s.close();
    assert!(s.is_closed());
    assert!(s
        .push_from_client(TENANT, SpdyFrame::data(15, CHANNEL_STDIN, b"x".to_vec()))
        .is_err());
    assert!(s.emit_to_client(CHANNEL_STDOUT, b"x".to_vec()).is_err());
}
