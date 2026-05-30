// SPDX-License-Identifier: AGPL-3.0-only
//
// Parity tests for the WebSocket framing path of the watch endpoint.
//
// Upstream: kubernetes/kubernetes (Apache-2.0)
//   pkg/endpoints/handlers/watch.go  — WatchServer, WebSocket framing
//   (apimachinery uses gorilla/websocket; the wire format is RFC 6455).
//
// kubectl watches over `wss://` when the client requests a WebSocket upgrade.
// The server completes the RFC 6455 handshake (Sec-WebSocket-Accept) and then
// streams each watch event as an unmasked server text frame. These tests pin:
//   * the canonical RFC 6455 §1.3 Sec-WebSocket-Accept vector,
//   * a 101 Switching Protocols handshake response carrying that accept value,
//   * correct payload-length framing across the 7-bit / 16-bit / 64-bit cases,
//   * server frames are never masked (mask bit clear),
//   * a watch event round-trips through a frame unchanged.

use cave_apiserver::routes::{WatchFraming, WatchServer};
use serde_json::json;

#[test]
fn ws_accept_key_matches_rfc6455_vector() {
    // RFC 6455 §1.3: key "dGhlIHNhbXBsZSBub25jZQ==" yields this accept value.
    let accept = WatchServer::ws_accept_key("dGhlIHNhbXBsZSBub25jZQ==");
    assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[test]
fn handshake_response_is_101_switching_protocols() {
    let ws = WatchServer {
        framing: WatchFraming::WebSocket,
        bookmarks: true,
        start_resource_version: 0,
    };
    let resp = ws.ws_handshake_response("dGhlIHNhbXBsZSBub25jZQ==");
    assert!(resp.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
    assert!(resp.contains("Upgrade: websocket\r\n"));
    assert!(resp.contains("Connection: Upgrade\r\n"));
    assert!(resp.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
    assert!(resp.ends_with("\r\n\r\n"));
}

#[test]
fn ws_frame_small_payload_uses_7bit_length_unmasked() {
    let ws = WatchServer::new();
    let frame = ws.ws_frame(b"hello"); // 5 bytes
    assert_eq!(frame[0], 0x81, "FIN + text opcode");
    // server frames are unmasked: high bit of byte1 clear, low 7 bits = length.
    assert_eq!(frame[1], 5);
    assert_eq!(&frame[2..], b"hello");
    assert_eq!(frame.len(), 7);
}

#[test]
fn ws_frame_medium_payload_uses_16bit_length() {
    let ws = WatchServer::new();
    let payload = vec![b'x'; 200]; // 126..=65535 -> 16-bit extended length
    let frame = ws.ws_frame(&payload);
    assert_eq!(frame[0], 0x81);
    assert_eq!(frame[1], 126);
    assert_eq!(u16::from_be_bytes([frame[2], frame[3]]), 200);
    assert_eq!(&frame[4..], &payload[..]);
}

#[test]
fn ws_frame_large_payload_uses_64bit_length() {
    let ws = WatchServer::new();
    let payload = vec![b'y'; 70_000]; // > 65535 -> 64-bit extended length
    let frame = ws.ws_frame(&payload);
    assert_eq!(frame[0], 0x81);
    assert_eq!(frame[1], 127);
    let len = u64::from_be_bytes([
        frame[2], frame[3], frame[4], frame[5], frame[6], frame[7], frame[8], frame[9],
    ]);
    assert_eq!(len, 70_000);
    assert_eq!(&frame[10..], &payload[..]);
}

#[test]
fn frame_event_ws_round_trips_json() {
    let ws = WatchServer::new();
    let event = json!({"type": "ADDED", "object": {"kind": "Pod", "rv": "42"}});
    let frame = ws.frame_event_ws(&event);

    // decode: header is byte0 + 7-bit length (payload is small).
    assert_eq!(frame[0], 0x81);
    let len = (frame[1] & 0x7f) as usize;
    assert!(len < 126);
    let payload = &frame[2..2 + len];
    let decoded: serde_json::Value = serde_json::from_slice(payload).unwrap();
    assert_eq!(decoded, event);
}

#[test]
fn ws_close_frame_has_close_opcode() {
    let ws = WatchServer::new();
    let frame = ws.ws_close_frame();
    // 0x88 = FIN + close opcode (0x8).
    assert_eq!(frame[0], 0x88);
}
