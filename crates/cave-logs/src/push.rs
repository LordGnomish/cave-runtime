//! Push API — parse JSON and Protobuf+Snappy push requests, ingest into LogStore.

use crate::models::{Labels, LogEntry, PushRequest};
use crate::store::LogStore;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tracing::{debug, warn};

/// Ingest a JSON push request.
pub fn ingest_json(store: &LogStore, req: PushRequest, tenant: Option<String>) {
    for stream in req.streams {
        let labels = Labels::new(stream.stream);
        let entries = stream
            .values
            .iter()
            .filter_map(|v| parse_json_entry(v))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            warn!("stream had no valid entries");
            continue;
        }
        debug!(count = entries.len(), "ingesting JSON stream");
        store.push(labels, entries, tenant.clone());
    }
}

/// Parse a single `[timestamp_ns, line]` or `[timestamp_ns, line, metadata]` value.
fn parse_json_entry(v: &serde_json::Value) -> Option<LogEntry> {
    let arr = v.as_array()?;
    if arr.len() < 2 {
        return None;
    }
    let ts_str = arr[0].as_str()?;
    let line = arr[1].as_str()?.to_string();
    let timestamp = parse_ns_timestamp(ts_str)?;

    let structured_metadata = if arr.len() >= 3 {
        parse_metadata(&arr[2])
    } else {
        HashMap::new()
    };

    Some(LogEntry { timestamp, line, structured_metadata })
}

fn parse_ns_timestamp(s: &str) -> Option<DateTime<Utc>> {
    let ns: i64 = s.parse().ok()?;
    let secs = ns / 1_000_000_000;
    let nanos = (ns % 1_000_000_000).unsigned_abs() as u32;
    DateTime::from_timestamp(secs, nanos)
}

fn parse_metadata(v: &serde_json::Value) -> HashMap<String, String> {
    match v {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), val)
            })
            .collect(),
        serde_json::Value::String(s) => {
            // Could be a JSON-encoded string
            serde_json::from_str(s).unwrap_or_default()
        }
        _ => HashMap::new(),
    }
}

/// Ingest a Protobuf+Snappy push request (raw bytes).
pub fn ingest_proto(store: &LogStore, body: bytes::Bytes, tenant: Option<String>) -> Result<(), String> {
    use prost::Message;

    // Decompress Snappy (raw block format)
    let mut decoder = snap::raw::Decoder::new();
    let decompressed = decoder
        .decompress_vec(&body)
        .map_err(|e| format!("snappy decompress: {e}"))?;

    // Decode protobuf
    let req = crate::models::proto::PushRequest::decode(decompressed.as_slice())
        .map_err(|e| format!("protobuf decode: {e}"))?;

    for stream in req.streams {
        let labels = parse_proto_labels(&stream.labels)?;
        let entries = stream
            .entries
            .into_iter()
            .filter_map(|e| {
                let ts = e.timestamp.as_ref()?;
                let timestamp = DateTime::from_timestamp(ts.seconds, ts.nanos as u32)?;
                let structured_metadata = e
                    .structured_metadata
                    .into_iter()
                    .map(|lp| (lp.name, lp.value))
                    .collect();
                Some(LogEntry { timestamp, line: e.line, structured_metadata })
            })
            .collect::<Vec<_>>();

        store.push(labels, entries, tenant.clone());
    }
    Ok(())
}

/// Parse Loki label selector string `{app="foo", env="prod"}` into a Labels map.
pub fn parse_proto_labels(s: &str) -> Result<Labels, String> {
    use crate::logql::lexer::{Lexer, Token};
    let tokens = Lexer::new(s).tokenize().map_err(|e| format!("label parse: {e}"))?;
    let mut pos = 0;
    let peek = |pos: usize| tokens.get(pos).unwrap_or(&Token::Eof);

    if peek(pos) != &Token::LBrace {
        return Err(format!("expected '{{' in label string: {s}"));
    }
    pos += 1;

    let mut map = HashMap::new();
    loop {
        match peek(pos) {
            Token::RBrace | Token::Eof => break,
            Token::Ident(name) => {
                let name = name.clone();
                pos += 1;
                // expect =
                pos += 1; // skip op (only = supported in proto labels)
                if let Token::Str(val) = peek(pos).clone() {
                    map.insert(name, val);
                    pos += 1;
                }
                if peek(pos) == &Token::Comma {
                    pos += 1;
                }
            }
            _ => break,
        }
    }
    Ok(Labels::new(map))
}
