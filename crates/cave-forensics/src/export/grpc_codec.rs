// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! gRPC length-prefixed frame codec.
//!
//! Upstream: gRPC HTTP/2 framing — 1-byte compressed flag + 4-byte
//! big-endian length, followed by the payload. We provide a pure
//! encode/decode pair so cave-forensics can feed a real `tonic`
//! transport from any sibling crate without hard-depending on tonic.

use crate::error::{ForensicsError, Result};
use crate::events::KernelEvent;
use bytes::{Buf, BufMut, BytesMut};

/// Encode a kernel event into a single gRPC frame (uncompressed, JSON
/// payload — matches `pkg/exporter/exporter.go` Marshal path).
pub fn encode_event(ev: &KernelEvent) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(ev)?;
    let mut buf = BytesMut::with_capacity(5 + payload.len());
    buf.put_u8(0); // compressed flag — 0 = no compression
    buf.put_u32(payload.len() as u32);
    buf.put_slice(&payload);
    Ok(buf.to_vec())
}

/// Decode one or more frames from a byte slice. Returns each decoded
/// event in order; on partial frame returns `Decode("incomplete...")`.
pub fn decode_events(bytes: &[u8]) -> Result<Vec<KernelEvent>> {
    let mut cursor = std::io::Cursor::new(bytes);
    let mut out = Vec::new();
    while (cursor.position() as usize) < bytes.len() {
        let remaining = bytes.len() - cursor.position() as usize;
        if remaining < 5 {
            return Err(ForensicsError::Decode(format!(
                "incomplete frame header: {remaining} bytes < 5"
            )));
        }
        let compressed = cursor.get_u8();
        let len = cursor.get_u32() as usize;
        let body_start = cursor.position() as usize;
        if bytes.len() < body_start + len {
            return Err(ForensicsError::Decode(format!(
                "incomplete frame body: have {} need {}",
                bytes.len() - body_start,
                len
            )));
        }
        if compressed != 0 {
            return Err(ForensicsError::Decode(
                "compressed gRPC frame not supported".into(),
            ));
        }
        let body = &bytes[body_start..body_start + len];
        let ev: KernelEvent = serde_json::from_slice(body)?;
        out.push(ev);
        cursor.set_position((body_start + len) as u64);
    }
    Ok(out)
}

/// Encode many events in one byte stream (back-to-back frames).
pub fn encode_many(evs: &[KernelEvent]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    for ev in evs {
        buf.extend_from_slice(&encode_event(ev)?);
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::process_exec::ProcessExecEvent;
    use crate::process::{Credentials, Namespaces, Process};
    use chrono::{TimeZone, Utc};

    fn ts() -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    fn exec(id: &str) -> KernelEvent {
        KernelEvent::ProcessExec(ProcessExecEvent {
            process: Process {
                exec_id: id.into(),
                pid: 1,
                pid_in_ns: 1,
                binary: "/bin/sh".into(),
                arguments: String::new(),
                cwd: "/".into(),
                credentials: Credentials::default(),
                namespaces: Namespaces::default(),
                parent_exec_id: None,
                container_id: None,
                pod_name: None,
                pod_namespace: None,
                start_time: ts(),
                end_time: None,
            },
            ancestors: vec![],
            observed_at: ts(),
        })
    }

    #[test]
    fn test_encode_single_frame_layout() {
        let ev = exec("a");
        let bytes = encode_event(&ev).unwrap();
        assert_eq!(bytes[0], 0, "compressed flag = 0");
        let len_be = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
        assert_eq!(len_be as usize, bytes.len() - 5);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let ev = exec("a");
        let bytes = encode_event(&ev).unwrap();
        let back = decode_events(&bytes).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], ev);
    }

    #[test]
    fn test_encode_many_back_to_back_frames() {
        let evs = vec![exec("a"), exec("b"), exec("c")];
        let bytes = encode_many(&evs).unwrap();
        let back = decode_events(&bytes).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back, evs);
    }

    #[test]
    fn test_decode_incomplete_header_errors() {
        let bytes = vec![0, 0, 0, 1];
        let err = decode_events(&bytes).unwrap_err();
        assert!(format!("{err}").contains("frame header"));
    }

    #[test]
    fn test_decode_incomplete_body_errors() {
        let mut bytes = encode_event(&exec("a")).unwrap();
        bytes.truncate(bytes.len() - 1);
        let err = decode_events(&bytes).unwrap_err();
        assert!(format!("{err}").contains("frame body"));
    }

    #[test]
    fn test_decode_rejects_compressed_flag() {
        let mut bytes = encode_event(&exec("a")).unwrap();
        bytes[0] = 1;
        let err = decode_events(&bytes).unwrap_err();
        assert!(format!("{err}").contains("compressed"));
    }
}
