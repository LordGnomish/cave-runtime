// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Newline-delimited JSON (NDJSON) event stream encoder + line decoder.
//!
//! Upstream: `pkg/encoder/json_encoder.go`. NDJSON is Tetragon's default
//! export format when the user passes `--export-format json`.

use crate::error::Result;
use crate::events::KernelEvent;

/// Encode a single event into a JSON line (no trailing newline).
pub fn encode_line(ev: &KernelEvent) -> Result<String> {
    Ok(serde_json::to_string(ev)?)
}

/// Encode many events into one NDJSON blob (each event on its own line,
/// newline-terminated). Empty list returns "".
pub fn encode_ndjson(evs: &[KernelEvent]) -> Result<String> {
    let mut out = String::new();
    for ev in evs {
        out.push_str(&encode_line(ev)?);
        out.push('\n');
    }
    Ok(out)
}

/// Decode an NDJSON blob into individual events. Blank lines are
/// ignored. A single malformed line aborts the decode.
pub fn decode_ndjson(blob: &str) -> Result<Vec<KernelEvent>> {
    let mut out = Vec::new();
    for line in blob.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let ev: KernelEvent = serde_json::from_str(line)?;
        out.push(ev);
    }
    Ok(out)
}

/// Streaming NDJSON writer — for use when events are produced over time
/// and the consumer is a file or socket.
pub struct NdjsonWriter<W: std::io::Write> {
    inner: W,
}

impl<W: std::io::Write> NdjsonWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub fn write_event(&mut self, ev: &KernelEvent) -> std::io::Result<()> {
        let line = encode_line(ev).map_err(std::io::Error::other)?;
        self.inner.write_all(line.as_bytes())?;
        self.inner.write_all(b"\n")?;
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
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

    fn ev(id: &str) -> KernelEvent {
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
    fn test_encode_line_no_trailing_newline() {
        let s = encode_line(&ev("a")).unwrap();
        assert!(!s.ends_with('\n'));
        assert!(s.contains("\"process_exec\""));
    }

    #[test]
    fn test_ndjson_roundtrip() {
        let evs = vec![ev("a"), ev("b"), ev("c")];
        let blob = encode_ndjson(&evs).unwrap();
        let back = decode_ndjson(&blob).unwrap();
        assert_eq!(back, evs);
    }

    #[test]
    fn test_empty_ndjson_yields_empty_string() {
        let blob = encode_ndjson(&[]).unwrap();
        assert!(blob.is_empty());
    }

    #[test]
    fn test_blank_lines_ignored_during_decode() {
        let evs = vec![ev("a"), ev("b")];
        let mut blob = encode_ndjson(&evs).unwrap();
        blob.push('\n');
        blob.push('\n');
        let back = decode_ndjson(&blob).unwrap();
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn test_malformed_line_errors() {
        let blob = "{not json}";
        assert!(decode_ndjson(blob).is_err());
    }

    #[test]
    fn test_streaming_writer_appends_newlines() {
        let mut sink: Vec<u8> = Vec::new();
        {
            let mut w = NdjsonWriter::new(&mut sink);
            w.write_event(&ev("a")).unwrap();
            w.write_event(&ev("b")).unwrap();
            w.flush().unwrap();
        }
        let s = String::from_utf8(sink).unwrap();
        assert_eq!(s.lines().count(), 2);
    }

    #[test]
    fn test_writer_into_inner_returns_sink() {
        let buf: Vec<u8> = Vec::new();
        let w = NdjsonWriter::new(buf);
        let inner = w.into_inner();
        assert!(inner.is_empty());
    }
}
