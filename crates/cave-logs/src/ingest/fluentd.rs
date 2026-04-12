//! Fluentd forward protocol receiver.
//!
//! Implements the [Fluentd forward protocol v1]
//! (https://github.com/fluent/fluentd/wiki/Forward-Protocol-Specification-v1).
//!
//! The forward protocol uses MessagePack over TCP. We implement a pure-Rust
//! MessagePack decoder to avoid adding a proc-macro dependency.
//!
//! Message modes:
//!   Message mode: [tag, time, record, option?]
//!   Forward mode: [tag, entries, option?]    where entries = [[time, record], ...]
//!   PackedForward: [tag, entries_msgpack_bytes, option?]
//!   CompressedPackedForward: gzip/deflate compressed PackedForward

use std::collections::HashMap;
use std::sync::Arc;
use chrono::Utc;

use crate::models::{Labels, LogEntry, TimestampNs};
use crate::store::LogStore;

// ── Minimal MessagePack decoder ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum MsgpackValue {
    Nil,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    Bin(Vec<u8>),
    Array(Vec<MsgpackValue>),
    Map(Vec<(MsgpackValue, MsgpackValue)>),
    Ext(i8, Vec<u8>),
}

impl MsgpackValue {
    pub fn as_str(&self) -> Option<&str> {
        match self { MsgpackValue::Str(s) => Some(s.as_str()), _ => None }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            MsgpackValue::UInt(n) => Some(*n),
            MsgpackValue::Int(n) if *n >= 0 => Some(*n as u64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            MsgpackValue::Int(n) => Some(*n),
            MsgpackValue::UInt(n) => i64::try_from(*n).ok(),
            _ => None,
        }
    }

    pub fn into_map(self) -> Option<Vec<(MsgpackValue, MsgpackValue)>> {
        match self { MsgpackValue::Map(m) => Some(m), _ => None }
    }

    pub fn into_array(self) -> Option<Vec<MsgpackValue>> {
        match self { MsgpackValue::Array(a) => Some(a), _ => None }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self { MsgpackValue::Bin(b) => Some(b), _ => None }
    }
}

struct MsgpackReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> MsgpackReader<'a> {
    fn new(buf: &'a [u8]) -> Self { Self { buf, pos: 0 } }

    fn read_byte(&mut self) -> anyhow::Result<u8> {
        if self.pos >= self.buf.len() { return Err(anyhow::anyhow!("msgpack: EOF")); }
        let b = self.buf[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        if self.pos + n > self.buf.len() { return Err(anyhow::anyhow!("msgpack: not enough bytes")); }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_u8(&mut self) -> anyhow::Result<u8> { self.read_byte() }
    fn read_u16(&mut self) -> anyhow::Result<u16> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn read_u32(&mut self) -> anyhow::Result<u32> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_u64(&mut self) -> anyhow::Result<u64> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_be_bytes(b.try_into().unwrap()))
    }
    fn read_i8(&mut self) -> anyhow::Result<i8> { Ok(self.read_byte()? as i8) }
    fn read_i16(&mut self) -> anyhow::Result<i16> {
        let b = self.read_bytes(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }
    fn read_i32(&mut self) -> anyhow::Result<i32> {
        let b = self.read_bytes(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_i64(&mut self) -> anyhow::Result<i64> {
        let b = self.read_bytes(8)?;
        Ok(i64::from_be_bytes(b.try_into().unwrap()))
    }
    fn read_f32(&mut self) -> anyhow::Result<f32> {
        let b = self.read_bytes(4)?;
        Ok(f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_f64(&mut self) -> anyhow::Result<f64> {
        let b = self.read_bytes(8)?;
        Ok(f64::from_be_bytes(b.try_into().unwrap()))
    }

    pub fn read_value(&mut self) -> anyhow::Result<MsgpackValue> {
        let byte = self.read_byte()?;
        match byte {
            // positive fixint
            0x00..=0x7f => Ok(MsgpackValue::UInt(byte as u64)),
            // fixmap
            0x80..=0x8f => {
                let n = (byte & 0x0f) as usize;
                self.read_map(n)
            }
            // fixarray
            0x90..=0x9f => {
                let n = (byte & 0x0f) as usize;
                self.read_array(n)
            }
            // fixstr
            0xa0..=0xbf => {
                let n = (byte & 0x1f) as usize;
                let s = self.read_bytes(n)?;
                Ok(MsgpackValue::Str(String::from_utf8_lossy(s).into_owned()))
            }
            0xc0 => Ok(MsgpackValue::Nil),
            0xc2 => Ok(MsgpackValue::Bool(false)),
            0xc3 => Ok(MsgpackValue::Bool(true)),
            0xc4 => { let n = self.read_u8()? as usize; Ok(MsgpackValue::Bin(self.read_bytes(n)?.to_vec())) }
            0xc5 => { let n = self.read_u16()? as usize; Ok(MsgpackValue::Bin(self.read_bytes(n)?.to_vec())) }
            0xc6 => { let n = self.read_u32()? as usize; Ok(MsgpackValue::Bin(self.read_bytes(n)?.to_vec())) }
            0xc7 => { let n = self.read_u8()? as usize; let t = self.read_i8()?; let d = self.read_bytes(n)?.to_vec(); Ok(MsgpackValue::Ext(t, d)) }
            0xc8 => { let n = self.read_u16()? as usize; let t = self.read_i8()?; let d = self.read_bytes(n)?.to_vec(); Ok(MsgpackValue::Ext(t, d)) }
            0xc9 => { let n = self.read_u32()? as usize; let t = self.read_i8()?; let d = self.read_bytes(n)?.to_vec(); Ok(MsgpackValue::Ext(t, d)) }
            0xca => Ok(MsgpackValue::Float(self.read_f32()? as f64)),
            0xcb => Ok(MsgpackValue::Float(self.read_f64()?)),
            0xcc => Ok(MsgpackValue::UInt(self.read_u8()? as u64)),
            0xcd => Ok(MsgpackValue::UInt(self.read_u16()? as u64)),
            0xce => Ok(MsgpackValue::UInt(self.read_u32()? as u64)),
            0xcf => Ok(MsgpackValue::UInt(self.read_u64()?)),
            0xd0 => Ok(MsgpackValue::Int(self.read_i8()? as i64)),
            0xd1 => Ok(MsgpackValue::Int(self.read_i16()? as i64)),
            0xd2 => Ok(MsgpackValue::Int(self.read_i32()? as i64)),
            0xd3 => Ok(MsgpackValue::Int(self.read_i64()?)),
            0xd4..=0xd8 => {
                let len = 1usize << (byte - 0xd4);
                let t = self.read_i8()?;
                let d = self.read_bytes(len)?.to_vec();
                Ok(MsgpackValue::Ext(t, d))
            }
            0xd9 => { let n = self.read_u8()? as usize; let s = self.read_bytes(n)?; Ok(MsgpackValue::Str(String::from_utf8_lossy(s).into_owned())) }
            0xda => { let n = self.read_u16()? as usize; let s = self.read_bytes(n)?; Ok(MsgpackValue::Str(String::from_utf8_lossy(s).into_owned())) }
            0xdb => { let n = self.read_u32()? as usize; let s = self.read_bytes(n)?; Ok(MsgpackValue::Str(String::from_utf8_lossy(s).into_owned())) }
            0xdc => { let n = self.read_u16()? as usize; self.read_array(n) }
            0xdd => { let n = self.read_u32()? as usize; self.read_array(n) }
            0xde => { let n = self.read_u16()? as usize; self.read_map(n) }
            0xdf => { let n = self.read_u32()? as usize; self.read_map(n) }
            // negative fixint
            0xe0..=0xff => Ok(MsgpackValue::Int(byte as i8 as i64)),
            _ => Err(anyhow::anyhow!("msgpack: unrecognised byte 0x{:02x}", byte)),
        }
    }

    fn read_array(&mut self, n: usize) -> anyhow::Result<MsgpackValue> {
        let mut arr = Vec::with_capacity(n);
        for _ in 0..n { arr.push(self.read_value()?); }
        Ok(MsgpackValue::Array(arr))
    }

    fn read_map(&mut self, n: usize) -> anyhow::Result<MsgpackValue> {
        let mut map = Vec::with_capacity(n);
        for _ in 0..n {
            let k = self.read_value()?;
            let v = self.read_value()?;
            map.push((k, v));
        }
        Ok(MsgpackValue::Map(map))
    }
}

// ── Fluentd event time (ext type -1 / 0x0d) ──────────────────────────────────

fn decode_event_time(v: &MsgpackValue) -> Option<TimestampNs> {
    match v {
        MsgpackValue::Ext(-1, data) if data.len() == 8 => {
            // EventTime: 4 bytes seconds + 4 bytes nanoseconds
            let secs = u32::from_be_bytes(data[..4].try_into().ok()?) as i64;
            let nsec = u32::from_be_bytes(data[4..].try_into().ok()?) as i64;
            Some(secs * 1_000_000_000 + nsec)
        }
        MsgpackValue::UInt(n) => Some(*n as i64 * 1_000_000_000),
        MsgpackValue::Int(n) => Some(*n * 1_000_000_000),
        _ => None,
    }
}

// ── Record conversion ─────────────────────────────────────────────────────────

fn map_to_strings(pairs: Vec<(MsgpackValue, MsgpackValue)>) -> HashMap<String, String> {
    pairs.into_iter()
        .map(|(k, v)| {
            let key = k.as_str().unwrap_or("").to_owned();
            let value = match &v {
                MsgpackValue::Str(s) => s.clone(),
                MsgpackValue::UInt(n) => n.to_string(),
                MsgpackValue::Int(n) => n.to_string(),
                MsgpackValue::Float(f) => f.to_string(),
                MsgpackValue::Bool(b) => b.to_string(),
                _ => String::new(),
            };
            (key, value)
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Parse and ingest a Fluentd forward protocol message (MessagePack encoded).
pub fn ingest_forward(
    data: &[u8],
    tenant: &str,
    store: &Arc<LogStore>,
) -> anyhow::Result<usize> {
    let mut reader = MsgpackReader::new(data);
    let msg = reader.read_value()?;

    let arr = match msg.into_array() {
        Some(a) if a.len() >= 2 => a,
        _ => return Err(anyhow::anyhow!("fluentd: expected array message")),
    };

    let tag = arr[0].as_str().unwrap_or("").to_owned();
    let mut label_map: HashMap<String, String> = HashMap::new();
    label_map.insert("fluentd_tag".into(), tag);

    // Determine mode by inspecting element[1]
    match &arr[1] {
        // Forward mode: entries is an array of [time, record]
        MsgpackValue::Array(_) => {
            let entries_val = arr[1].clone();
            let entries_arr = entries_val.into_array().unwrap_or_default();
            let mut log_entries = Vec::new();
            for pair in entries_arr {
                let pair_arr = match pair.into_array() {
                    Some(a) if a.len() >= 2 => a,
                    _ => continue,
                };
                let ts = decode_event_time(&pair_arr[0])
                    .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0));
                let record = match pair_arr[1].clone().into_map() {
                    Some(m) => map_to_strings(m),
                    None => continue,
                };
                let line = record.get("message").or(record.get("log")).cloned().unwrap_or_default();
                let mut meta = record;
                meta.remove("message");
                meta.remove("log");
                log_entries.push(LogEntry { ts, line, metadata: meta });
            }
            let n = log_entries.len();
            if !log_entries.is_empty() {
                store.push(tenant, Labels::new(label_map), log_entries)?;
            }
            Ok(n)
        }
        // Packed / compressed: entries is a binary blob
        MsgpackValue::Bin(raw) => {
            let unpacked = raw.clone();
            ingest_packed_forward(&unpacked, tenant, &label_map, store)
        }
        // Message mode: element[1] is the timestamp, [2] is the record
        time_val if arr.len() >= 3 => {
            let ts = decode_event_time(time_val)
                .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0));
            let record = match arr[2].clone().into_map() {
                Some(m) => map_to_strings(m),
                None => return Ok(0),
            };
            let line = record.get("message").or(record.get("log")).cloned().unwrap_or_default();
            let mut meta = record;
            meta.remove("message");
            meta.remove("log");
            let entry = LogEntry { ts, line, metadata: meta };
            store.push(tenant, Labels::new(label_map), vec![entry])?;
            Ok(1)
        }
        _ => Ok(0),
    }
}

fn ingest_packed_forward(
    data: &[u8],
    tenant: &str,
    base_labels: &HashMap<String, String>,
    store: &Arc<LogStore>,
) -> anyhow::Result<usize> {
    let mut reader = MsgpackReader::new(data);
    let mut entries = Vec::new();

    while reader.pos < reader.buf.len() {
        let arr = match reader.read_value()?.into_array() {
            Some(a) if a.len() >= 2 => a,
            _ => continue,
        };
        let ts = decode_event_time(&arr[0])
            .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let record = match arr[1].clone().into_map() {
            Some(m) => map_to_strings(m),
            None => continue,
        };
        let line = record.get("message").or(record.get("log")).cloned().unwrap_or_default();
        let mut meta = record;
        meta.remove("message");
        meta.remove("log");
        entries.push(LogEntry { ts, line, metadata: meta });
    }

    let n = entries.len();
    if !entries.is_empty() {
        store.push(tenant, Labels::new(base_labels.clone()), entries)?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LogStore;
    use crate::models::Direction;

    /// Build a minimal MessagePack forward-mode message:
    /// [tag, [[time, {record}]]]
    fn build_forward_msg(tag: &str, ts_secs: u64, record: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        // fixarray of 2
        buf.push(0x92);
        // tag (fixstr)
        let tag_bytes = tag.as_bytes();
        buf.push(0xa0 | tag_bytes.len() as u8);
        buf.extend_from_slice(tag_bytes);
        // entries: fixarray of 1
        buf.push(0x91);
        // entry: fixarray of 2
        buf.push(0x92);
        // time: uint32
        buf.push(0xce);
        buf.extend_from_slice(&(ts_secs as u32).to_be_bytes());
        // record: fixmap
        buf.push(0x80 | record.len() as u8);
        for (k, v) in record {
            buf.push(0xa0 | k.len() as u8);
            buf.extend_from_slice(k.as_bytes());
            buf.push(0xa0 | v.len() as u8);
            buf.extend_from_slice(v.as_bytes());
        }
        buf
    }

    #[test]
    fn forward_mode_basic() {
        let store = LogStore::new();
        let msg = build_forward_msg("app.logs", 1_000_000, &[("message", "hello"), ("level", "info")]);
        let n = ingest_forward(&msg, "t", &store).unwrap();
        assert_eq!(n, 1);

        let fps = store.matching_fps("t", |_| true);
        assert!(!fps.is_empty());
        let results = store.query_entries("t", &fps, 0, i64::MAX, 10, Direction::Forward);
        assert_eq!(results[0].2[0].line, "hello");
    }

    #[test]
    fn msgpack_reader_basic_types() {
        let mut buf = Vec::new();
        buf.push(0x92); // fixarray 2
        buf.push(0x01); // positive fixint 1
        buf.push(0xa3); // fixstr len 3
        buf.extend_from_slice(b"foo");

        let mut r = MsgpackReader::new(&buf);
        let v = r.read_value().unwrap();
        if let MsgpackValue::Array(arr) = v {
            assert_eq!(arr.len(), 2);
            assert!(matches!(arr[0], MsgpackValue::UInt(1)));
            assert_eq!(arr[1].as_str(), Some("foo"));
        } else { panic!("expected array"); }
    }
}
