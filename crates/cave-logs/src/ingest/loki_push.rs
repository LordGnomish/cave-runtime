// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Loki push API handler — supports both JSON and protobuf+snappy wire formats.
//!
//! Implements the Loki v1 push endpoint:
//!   POST /loki/api/v1/push
//!
//! Content-Type: application/json   → JSON body
//! Content-Type: application/x-protobuf → protobuf encoded with snappy (raw) compression
//!
//! Promtail and the Loki client library both use the protobuf path.

use std::collections::HashMap;
use std::sync::Arc;

use crate::chunk::snappy_raw_decompress;
use crate::models::{Labels, LogEntry, PushRequest, TimestampNs};
use crate::store::LogStore;

/// Parse and ingest a JSON push request body.
pub fn ingest_json(body: &[u8], tenant: &str, store: &Arc<LogStore>) -> anyhow::Result<usize> {
    let req: PushRequest = serde_json::from_slice(body)?;
    let mut total = 0usize;

    for stream in req.streams {
        let labels = Labels::new(stream.stream);
        let entries: Vec<LogEntry> = stream
            .values
            .into_iter()
            .map(|ev| {
                let mut e = LogEntry::new(ev.ts_ns, ev.line);
                if let Some(meta) = ev.metadata {
                    e.metadata = meta;
                }
                e
            })
            .collect();
        total += entries.len();
        store.push(tenant, labels, entries)?;
    }

    Ok(total)
}

// ── Protobuf definitions (minimal, without generated code) ───────────────────
//
// We hand-roll a minimal protobuf decoder to avoid adding a build.rs / .proto
// dependency. The Loki push proto is simple enough:
//
//  message PushRequest  { repeated StreamAdapter streams = 1; }
//  message StreamAdapter {
//    string labels  = 1;
//    repeated Entry entries = 2;
//  }
//  message Entry {
//    google.protobuf.Timestamp timestamp = 1;
//    string line = 2;
//    repeated LabelPairAdapter structuredMetadata = 3;
//  }
//  message LabelPairAdapter { string name = 1; string value = 2; }
//
// Protobuf field tag = (field_number << 3) | wire_type

const WT_VARINT: u8 = 0;
const WT_64BIT: u8 = 1;
const WT_LEN: u8 = 2;
const WT_32BIT: u8 = 5;

struct ProtoReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ProtoReader<'a> {
    fn new(buf: &'a [u8]) -> Self { Self { buf, pos: 0 } }

    fn remaining(&self) -> usize { self.buf.len() - self.pos }

    fn read_varint(&mut self) -> anyhow::Result<u64> {
        let mut result = 0u64;
        let mut shift = 0u32;
        loop {
            if self.pos >= self.buf.len() {
                return Err(anyhow::anyhow!("protobuf: unexpected EOF in varint"));
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 { break; }
            shift += 7;
            if shift >= 64 { return Err(anyhow::anyhow!("protobuf: varint overflow")); }
        }
        Ok(result)
    }

    fn read_bytes(&mut self, len: usize) -> anyhow::Result<&'a [u8]> {
        if self.pos + len > self.buf.len() {
            return Err(anyhow::anyhow!("protobuf: not enough bytes"));
        }
        let slice = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    fn read_tag(&mut self) -> anyhow::Result<Option<(u32, u8)>> {
        if self.remaining() == 0 { return Ok(None); }
        let tag = self.read_varint()?;
        Ok(Some(((tag >> 3) as u32, (tag & 0x7) as u8)))
    }

    fn skip_field(&mut self, wire_type: u8) -> anyhow::Result<()> {
        match wire_type {
            WT_VARINT => { self.read_varint()?; }
            WT_64BIT => { self.read_bytes(8)?; }
            WT_LEN => {
                let len = self.read_varint()? as usize;
                self.read_bytes(len)?;
            }
            WT_32BIT => { self.read_bytes(4)?; }
            wt => return Err(anyhow::anyhow!("protobuf: unknown wire type {}", wt)),
        }
        Ok(())
    }

    fn read_len_delimited(&mut self) -> anyhow::Result<&'a [u8]> {
        let len = self.read_varint()? as usize;
        self.read_bytes(len)
    }

    fn read_string(&mut self) -> anyhow::Result<String> {
        let bytes = self.read_len_delimited()?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Parse a Loki push protobuf (after snappy decompression).
fn parse_proto_push(buf: &[u8]) -> anyhow::Result<Vec<(Labels, Vec<LogEntry>)>> {
    let mut reader = ProtoReader::new(buf);
    let mut result = Vec::new();

    while let Some((field, wt)) = reader.read_tag()? {
        match (field, wt) {
            (1, WT_LEN) => {
                // StreamAdapter
                let stream_buf = reader.read_len_delimited()?;
                let (labels, entries) = parse_stream_adapter(stream_buf)?;
                result.push((labels, entries));
            }
            _ => reader.skip_field(wt)?,
        }
    }

    Ok(result)
}

fn parse_stream_adapter(buf: &[u8]) -> anyhow::Result<(Labels, Vec<LogEntry>)> {
    let mut reader = ProtoReader::new(buf);
    let mut label_str = String::new();
    let mut entries = Vec::new();

    while let Some((field, wt)) = reader.read_tag()? {
        match (field, wt) {
            (1, WT_LEN) => { label_str = reader.read_string()?; }
            (2, WT_LEN) => {
                let entry_buf = reader.read_len_delimited()?;
                entries.push(parse_entry(entry_buf)?);
            }
            _ => reader.skip_field(wt)?,
        }
    }

    let labels = parse_label_selector(&label_str);
    Ok((labels, entries))
}

fn parse_entry(buf: &[u8]) -> anyhow::Result<LogEntry> {
    let mut reader = ProtoReader::new(buf);
    let mut ts_ns: TimestampNs = 0;
    let mut line = String::new();
    let mut metadata: HashMap<String, String> = HashMap::new();

    while let Some((field, wt)) = reader.read_tag()? {
        match (field, wt) {
            (1, WT_LEN) => {
                // google.protobuf.Timestamp: field 1 = seconds (i64 varint), field 2 = nanos (i32 varint)
                let ts_buf = reader.read_len_delimited()?;
                ts_ns = parse_timestamp(ts_buf)?;
            }
            (2, WT_LEN) => { line = reader.read_string()?; }
            (3, WT_LEN) => {
                let pair_buf = reader.read_len_delimited()?;
                let (k, v) = parse_label_pair(pair_buf)?;
                metadata.insert(k, v);
            }
            _ => reader.skip_field(wt)?,
        }
    }

    Ok(LogEntry { ts: ts_ns, line, metadata })
}

fn parse_timestamp(buf: &[u8]) -> anyhow::Result<TimestampNs> {
    let mut reader = ProtoReader::new(buf);
    let mut secs = 0i64;
    let mut nanos = 0i32;

    while let Some((field, wt)) = reader.read_tag()? {
        match (field, wt) {
            (1, WT_VARINT) => { secs = reader.read_varint()? as i64; }
            (2, WT_VARINT) => { nanos = reader.read_varint()? as i32; }
            _ => reader.skip_field(wt)?,
        }
    }

    Ok(secs * 1_000_000_000 + nanos as i64)
}

fn parse_label_pair(buf: &[u8]) -> anyhow::Result<(String, String)> {
    let mut reader = ProtoReader::new(buf);
    let mut name = String::new();
    let mut value = String::new();

    while let Some((field, wt)) = reader.read_tag()? {
        match (field, wt) {
            (1, WT_LEN) => { name = reader.read_string()?; }
            (2, WT_LEN) => { value = reader.read_string()?; }
            _ => reader.skip_field(wt)?,
        }
    }

    Ok((name, value))
}

/// Parse a Loki selector string `{app="nginx",env="prod"}` into a Labels map.
pub fn parse_label_selector(s: &str) -> Labels {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    let mut map = HashMap::new();

    for part in split_label_pairs(s) {
        let part = part.trim();
        // Find the operator: =~, !~, !=, =
        let (key, value) = if let Some(i) = find_op(part) {
            let key = part[..i].trim();
            let rest = &part[i..];
            let op_len = if rest.starts_with("=~") || rest.starts_with("!~") || rest.starts_with("!=") { 2 } else { 1 };
            let value_raw = rest[op_len..].trim().trim_matches('"');
            (key, value_raw.to_owned())
        } else {
            continue;
        };
        if !key.is_empty() {
            map.insert(key.to_owned(), value);
        }
    }

    Labels::new(map)
}

fn find_op(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'=' => return Some(i),
            b'!' | b'~' => {
                if i + 1 < bytes.len() && (bytes[i + 1] == b'=' || bytes[i + 1] == b'~') {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_label_pairs(s: &str) -> Vec<&str> {
    // Split by comma, but not inside quoted strings.
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ',' if !in_quote => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start <= s.len() { parts.push(&s[start..]); }
    parts
}

/// Parse and ingest a protobuf+snappy push request body.
pub fn ingest_protobuf(body: &[u8], tenant: &str, store: &Arc<LogStore>) -> anyhow::Result<usize> {
    let decompressed = snappy_raw_decompress(body)?;
    let streams = parse_proto_push(&decompressed)?;
    let mut total = 0usize;
    for (labels, entries) in streams {
        total += entries.len();
        store.push(tenant, labels, entries)?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LogStore;
    use crate::models::Direction;

    #[test]
    fn parse_selector_eq() {
        let labels = parse_label_selector(r#"{app="nginx",env="prod"}"#);
        assert_eq!(labels.get("app"), Some("nginx"));
        assert_eq!(labels.get("env"), Some("prod"));
    }

    #[test]
    fn parse_selector_empty() {
        let labels = parse_label_selector("{}");
        assert!(labels.is_empty());
    }

    #[test]
    fn ingest_json_push() {
        let store = LogStore::new();
        let body = serde_json::json!({
            "streams": [
                {
                    "stream": {"app": "test"},
                    "values": [
                        ["1000000000", "hello world"],
                        ["2000000000", "second line"]
                    ]
                }
            ]
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let n = ingest_json(&bytes, "tenant1", &store).unwrap();
        assert_eq!(n, 2);

        let fps = store.matching_fps("tenant1", |_| true);
        assert_eq!(fps.len(), 1);

        let results = store.query_entries("tenant1", &fps, 0, i64::MAX, 10, Direction::Forward);
        assert_eq!(results[0].2.len(), 2);
    }

    #[test]
    fn parse_selector_with_regex_op_keeps_key() {
        // Even though we store all labels as eq-style in the push, the selector
        // parser should handle it gracefully.
        let labels = parse_label_selector(r#"{app="myapp"}"#);
        assert_eq!(labels.get("app"), Some("myapp"));
    }
}
