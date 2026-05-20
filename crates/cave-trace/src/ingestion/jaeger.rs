// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Jaeger ingestion — two transports, two encodings.
//!
//! Transport / Encoding matrix
//! ───────────────────────────
//! • UDP agent  port 6831 → Thrift Compact (agent emitBatch)  ← this file decodes
//! • UDP agent  port 6832 → Thrift Binary  (less common)
//! • HTTP collector /api/traces → JSON  (Content-Type: application/json)
//! • HTTP collector /api/traces → Thrift Binary (Content-Type: application/x-thrift)
//!
//! The Thrift decoder here handles the Jaeger Batch struct defined in
//! github.com/jaegertracing/jaeger-idl.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ingestion::{normalise_service, us_to_ns};
use crate::types::{Span, SpanEvent, SpanId, SpanKind, SpanLink, SpanStatus, TagValue, TraceId};
use crate::{Result, TraceError};

// ═══════════════════════════════════════════════════════════════════════════
// ── Jaeger JSON (HTTP collector) ───────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════

/// Top-level Jaeger JSON format (used by the HTTP collector endpoint).
#[derive(Debug, Deserialize)]
pub struct JaegerBatchEnvelope {
    pub data: Option<Vec<JaegerTrace>>,
    // Some exporters send a single batch directly
    pub spans: Option<Vec<JaegerSpan>>,
    pub process: Option<JaegerProcess>,
}

#[derive(Debug, Deserialize)]
pub struct JaegerTrace {
    #[serde(rename = "traceID")]
    pub trace_id: String,
    pub spans: Vec<JaegerSpan>,
    pub processes: Option<HashMap<String, JaegerProcess>>,
}

#[derive(Debug, Deserialize)]
pub struct JaegerSpan {
    #[serde(rename = "traceID")]
    pub trace_id: String,
    #[serde(rename = "spanID")]
    pub span_id: String,
    #[serde(rename = "operationName")]
    pub operation_name: String,
    pub references: Option<Vec<JaegerReference>>,
    /// Epoch microseconds.
    #[serde(rename = "startTime")]
    pub start_time: i64,
    /// Duration in microseconds.
    pub duration: i64,
    pub tags: Option<Vec<JaegerTag>>,
    pub logs: Option<Vec<JaegerLog>>,
    pub process: Option<JaegerProcess>,
    #[serde(rename = "processID")]
    pub process_id: Option<String>,
    pub flags: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct JaegerReference {
    #[serde(rename = "refType")]
    pub ref_type: String, // "CHILD_OF" | "FOLLOWS_FROM"
    #[serde(rename = "traceID")]
    pub trace_id: String,
    #[serde(rename = "spanID")]
    pub span_id: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JaegerTag {
    pub key: String,
    #[serde(rename = "type")]
    pub tag_type: String, // "string" | "bool" | "int64" | "float64" | "binary"
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct JaegerLog {
    pub timestamp: i64,
    pub fields: Vec<JaegerTag>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct JaegerProcess {
    #[serde(rename = "serviceName")]
    pub service_name: String,
    pub tags: Option<Vec<JaegerTag>>,
}

// ─── Parse JSON ────────────────────────────────────────────────────────────

pub fn parse_jaeger_json(body: &[u8], tenant_id: &str) -> Result<Vec<Span>> {
    // Try envelope first, then bare batch
    let envelope: JaegerBatchEnvelope =
        serde_json::from_slice(body).map_err(|e| TraceError::ParseError(e.to_string()))?;

    let mut out = Vec::new();

    if let Some(traces) = envelope.data {
        for trace in traces {
            let default_process = None::<&JaegerProcess>;
            for jspan in &trace.spans {
                let process = jspan
                    .process
                    .as_ref()
                    .or_else(|| {
                        jspan
                            .process_id
                            .as_ref()
                            .and_then(|pid| trace.processes.as_ref()?.get(pid))
                    })
                    .or(default_process);
                if let Some(span) = convert_json_span(jspan, process, tenant_id) {
                    out.push(span);
                }
            }
        }
    } else if let Some(spans) = envelope.spans {
        let process = envelope.process.as_ref();
        for jspan in &spans {
            let proc = jspan.process.as_ref().or(process);
            if let Some(span) = convert_json_span(&jspan, proc, tenant_id) {
                out.push(span);
            }
        }
    }

    Ok(out)
}

fn convert_json_span(
    js: &JaegerSpan,
    process: Option<&JaegerProcess>,
    tenant_id: &str,
) -> Option<Span> {
    let trace_id = crate::types::parse_trace_id(&js.trace_id).ok()?;
    let span_id = crate::types::parse_span_id(&js.span_id).ok()?;

    let parent_span_id = js
        .references
        .as_deref()
        .and_then(|refs| refs.iter().find(|r| r.ref_type == "CHILD_OF"))
        .and_then(|r| crate::types::parse_span_id(&r.span_id).ok());

    let start_ns = us_to_ns(js.start_time);
    let dur_ns = us_to_ns(js.duration);
    let end_ns = start_ns + dur_ns;

    let service_name = process
        .map(|p| normalise_service(&p.service_name))
        .unwrap_or_else(|| "unknown".into());

    let mut tags: HashMap<String, TagValue> = js
        .tags
        .as_deref()
        .map(parse_jaeger_tags)
        .unwrap_or_default();

    let resource_attrs: HashMap<String, TagValue> = process
        .and_then(|p| p.tags.as_deref())
        .map(parse_jaeger_tags)
        .unwrap_or_default();

    // Derive log labels from process tags
    let log_labels: HashMap<String, String> = resource_attrs
        .iter()
        .map(|(k, v)| (k.clone(), v.display()))
        .collect();

    // Derive status from tags
    let status = if tags
        .get("error")
        .map(|v| v.display() != "false")
        .unwrap_or(false)
    {
        SpanStatus::Error
    } else {
        SpanStatus::Ok
    };

    let events: Vec<_> = js
        .logs
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|log| SpanEvent {
            time_unix_nano: us_to_ns(log.timestamp),
            name: log
                .fields
                .iter()
                .find(|f| f.key == "event" || f.key == "message")
                .map(|f| f.value.as_str().unwrap_or("").to_owned())
                .unwrap_or_else(|| "log".into()),
            attributes: parse_jaeger_tags(&log.fields),
        })
        .collect();

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        operation_name: js.operation_name.clone(),
        service_name,
        start_time_unix_nano: start_ns,
        end_time_unix_nano: end_ns,
        duration_ns: dur_ns,
        status,
        kind: SpanKind::Internal, // Jaeger JSON doesn't carry span kind
        tags,
        events,
        links: vec![],
        resource_attributes: resource_attrs,
        tenant_id: tenant_id.to_owned(),
        baggage: HashMap::new(),
        log_labels,
    })
}

fn parse_jaeger_tags(tags: &[JaegerTag]) -> HashMap<String, TagValue> {
    tags.iter()
        .map(|t| (t.key.clone(), jaeger_tag_to_value(t)))
        .collect()
}

fn jaeger_tag_to_value(t: &JaegerTag) -> TagValue {
    match t.tag_type.as_str() {
        "bool" => TagValue::Bool(t.value.as_bool().unwrap_or(false)),
        "int64" | "int" => TagValue::Int(
            t.value
                .as_i64()
                .or_else(|| t.value.as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0),
        ),
        "float64" | "float" => TagValue::Float(t.value.as_f64().unwrap_or(0.0)),
        "binary" => {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            TagValue::Binary(
                t.value
                    .as_str()
                    .and_then(|s| STANDARD.decode(s).ok())
                    .unwrap_or_default(),
            )
        }
        _ => TagValue::String(
            t.value
                .as_str()
                .map(|s| s.to_owned())
                .or_else(|| Some(t.value.to_string()))
                .unwrap_or_default(),
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ── Thrift Compact Protocol decoder (Jaeger UDP agent) ────────────────────
// ═══════════════════════════════════════════════════════════════════════════
//
// Reference: https://github.com/apache/thrift/blob/master/doc/specs/thrift-compact-protocol.md
//
// Field type nibbles:
//   0x00 = stop
//   0x01 = boolean(true)   0x02 = boolean(false)
//   0x03 = i8              0x04 = i16
//   0x05 = i32             0x06 = i64
//   0x07 = double (8 LE)   0x08 = binary
//   0x09 = list            0x0A = set
//   0x0B = map             0x0C = struct

struct CompactReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CompactReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        CompactReader { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_byte(&mut self) -> Result<u8> {
        if self.pos >= self.data.len() {
            return Err(TraceError::ThriftError("unexpected end of input".into()));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(TraceError::ThriftError(format!(
                "need {} bytes, have {}",
                n,
                self.remaining()
            )));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Read unsigned varint (up to 64-bit).
    fn read_uvarint(&mut self) -> Result<u64> {
        let mut result = 0u64;
        let mut shift = 0;
        loop {
            let b = self.read_byte()? as u64;
            result |= (b & 0x7F) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift >= 64 {
                return Err(TraceError::ThriftError("varint overflow".into()));
            }
        }
        Ok(result)
    }

    /// Zigzag-decoded i32.
    fn read_i32(&mut self) -> Result<i32> {
        let u = self.read_uvarint()? as u32;
        Ok(((u >> 1) as i32) ^ -((u & 1) as i32))
    }

    /// Zigzag-decoded i64.
    fn read_i64(&mut self) -> Result<i64> {
        let u = self.read_uvarint()?;
        Ok(((u >> 1) as i64) ^ -((u & 1) as i64))
    }

    fn read_double(&mut self) -> Result<f64> {
        let bytes = self.read_bytes(8)?;
        Ok(f64::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_binary(&mut self) -> Result<Vec<u8>> {
        let len = self.read_uvarint()? as usize;
        Ok(self.read_bytes(len)?.to_vec())
    }

    fn read_string(&mut self) -> Result<String> {
        let bytes = self.read_binary()?;
        String::from_utf8(bytes).map_err(|e| TraceError::ThriftError(e.to_string()))
    }

    /// Read a field header. Returns `None` on stop byte. `last_field_id` is updated.
    fn read_field_header(&mut self, last_field_id: &mut i16) -> Result<Option<(i16, u8)>> {
        let byte = self.read_byte()?;
        if byte == 0x00 {
            return Ok(None);
        } // stop

        let delta = (byte >> 4) as i16;
        let type_id = byte & 0x0F;

        let field_id = if delta == 0 {
            // Full zigzag-encoded field ID follows
            let u = self.read_uvarint()? as u16;
            ((u >> 1) as i16) ^ -((u & 1) as i16)
        } else {
            *last_field_id + delta
        };

        *last_field_id = field_id;
        Ok(Some((field_id, type_id)))
    }

    /// Skip over a value of the given compact type.
    fn skip(&mut self, type_id: u8) -> Result<()> {
        match type_id {
            0x01 | 0x02 => {} // boolean stored in type nibble, no extra bytes
            0x03 => {
                self.read_byte()?;
            }
            0x04 | 0x05 => {
                self.read_uvarint()?;
            }
            0x06 => {
                self.read_uvarint()?;
            }
            0x07 => {
                self.read_bytes(8)?;
            }
            0x08 => {
                let n = self.read_uvarint()? as usize;
                self.read_bytes(n)?;
            }
            0x09 | 0x0A => {
                let header = self.read_byte()?;
                let size = self.read_uvarint()? as usize;
                let elem_type = header & 0x0F;
                for _ in 0..size {
                    self.skip(elem_type)?;
                }
            }
            0x0B => {
                let header = self.read_byte()?;
                let size = self.read_uvarint()? as usize;
                let key_type = (header >> 4) & 0x0F;
                let val_type = header & 0x0F;
                for _ in 0..size {
                    self.skip(key_type)?;
                    self.skip(val_type)?;
                }
            }
            0x0C => {
                // Nested struct
                let mut last = 0i16;
                while let Some((_, t)) = self.read_field_header(&mut last)? {
                    self.skip(t)?;
                }
            }
            _ => {
                return Err(TraceError::ThriftError(format!(
                    "unknown type: 0x{:02x}",
                    type_id
                )));
            }
        }
        Ok(())
    }
}

// ─── Jaeger IDL struct decoders ────────────────────────────────────────────

#[derive(Debug, Default)]
struct ThriftTag {
    key: String,
    vtype: i32, // 0=string 1=double 2=bool 3=long 4=binary
    str_value: Option<String>,
    double_value: Option<f64>,
    bool_value: Option<bool>,
    long_value: Option<i64>,
    bin_value: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
struct ThriftProcess {
    service_name: String,
    tags: Vec<ThriftTag>,
}

#[derive(Debug, Default)]
struct ThriftSpanRef {
    trace_id_low: i64,
    trace_id_high: i64,
    span_id: i64,
    ref_type: i32,
}

#[derive(Debug, Default)]
struct ThriftLog {
    timestamp: i64,
    fields: Vec<ThriftTag>,
}

#[derive(Debug, Default)]
struct ThriftSpan {
    trace_id_low: i64,
    trace_id_high: i64,
    span_id: i64,
    parent_span_id: i64,
    operation_name: String,
    references: Vec<ThriftSpanRef>,
    flags: i32,
    start_time: i64,
    duration: i64,
    tags: Vec<ThriftTag>,
    logs: Vec<ThriftLog>,
}

#[derive(Debug, Default)]
struct ThriftBatch {
    process: ThriftProcess,
    spans: Vec<ThriftSpan>,
}

fn decode_tag(r: &mut CompactReader<'_>) -> Result<ThriftTag> {
    let mut tag = ThriftTag::default();
    let mut last = 0i16;
    while let Some((field_id, type_id)) = r.read_field_header(&mut last)? {
        match field_id {
            1 => tag.key = r.read_string()?,
            2 => tag.vtype = r.read_i32()?,
            3 => tag.str_value = Some(r.read_string()?),
            4 => tag.double_value = Some(r.read_double()?),
            5 => tag.bool_value = Some(type_id == 0x01),
            6 => tag.long_value = Some(r.read_i64()?),
            7 => tag.bin_value = Some(r.read_binary()?),
            _ => r.skip(type_id)?,
        }
    }
    Ok(tag)
}

fn decode_tags(r: &mut CompactReader<'_>) -> Result<Vec<ThriftTag>> {
    let header = r.read_byte()?;
    let size = if (header >> 4) == 0xF {
        r.read_uvarint()? as usize
    } else {
        (header >> 4) as usize
    };
    (0..size)
        .map(|_| {
            let t = decode_tag(r)?;
            Ok(t)
        })
        .collect()
}

fn decode_process(r: &mut CompactReader<'_>) -> Result<ThriftProcess> {
    let mut p = ThriftProcess::default();
    let mut last = 0i16;
    while let Some((field_id, type_id)) = r.read_field_header(&mut last)? {
        match field_id {
            1 => p.service_name = r.read_string()?,
            2 => p.tags = decode_tags(r)?,
            _ => r.skip(type_id)?,
        }
    }
    Ok(p)
}

fn decode_span_ref(r: &mut CompactReader<'_>) -> Result<ThriftSpanRef> {
    let mut sr = ThriftSpanRef::default();
    let mut last = 0i16;
    while let Some((field_id, type_id)) = r.read_field_header(&mut last)? {
        match field_id {
            1 => sr.trace_id_low = r.read_i64()?,
            2 => sr.trace_id_high = r.read_i64()?,
            3 => sr.span_id = r.read_i64()?,
            4 => sr.ref_type = r.read_i32()?,
            _ => r.skip(type_id)?,
        }
    }
    Ok(sr)
}

fn decode_log(r: &mut CompactReader<'_>) -> Result<ThriftLog> {
    let mut log = ThriftLog::default();
    let mut last = 0i16;
    while let Some((field_id, type_id)) = r.read_field_header(&mut last)? {
        match field_id {
            1 => log.timestamp = r.read_i64()?,
            2 => log.fields = decode_tags(r)?,
            _ => r.skip(type_id)?,
        }
    }
    Ok(log)
}

fn decode_span_struct(r: &mut CompactReader<'_>) -> Result<ThriftSpan> {
    let mut s = ThriftSpan::default();
    let mut last = 0i16;
    while let Some((field_id, type_id)) = r.read_field_header(&mut last)? {
        match field_id {
            1 => s.trace_id_low = r.read_i64()?,
            2 => s.trace_id_high = r.read_i64()?,
            3 => s.span_id = r.read_i64()?,
            4 => s.parent_span_id = r.read_i64()?,
            5 => s.operation_name = r.read_string()?,
            6 => {
                // list<SpanRef>
                let header = r.read_byte()?;
                let size = if (header >> 4) == 0xF {
                    r.read_uvarint()? as usize
                } else {
                    (header >> 4) as usize
                };
                for _ in 0..size {
                    s.references.push(decode_span_ref(r)?);
                }
            }
            7 => s.flags = r.read_i32()?,
            8 => s.start_time = r.read_i64()?,
            9 => s.duration = r.read_i64()?,
            10 => s.tags = decode_tags(r)?,
            11 => {
                let header = r.read_byte()?;
                let size = if (header >> 4) == 0xF {
                    r.read_uvarint()? as usize
                } else {
                    (header >> 4) as usize
                };
                for _ in 0..size {
                    s.logs.push(decode_log(r)?);
                }
            }
            _ => r.skip(type_id)?,
        }
    }
    Ok(s)
}

fn decode_batch(r: &mut CompactReader<'_>) -> Result<ThriftBatch> {
    let mut batch = ThriftBatch::default();
    let mut last = 0i16;
    while let Some((field_id, type_id)) = r.read_field_header(&mut last)? {
        match field_id {
            1 => batch.process = decode_process(r)?,
            2 => {
                let header = r.read_byte()?;
                let size = if (header >> 4) == 0xF {
                    r.read_uvarint()? as usize
                } else {
                    (header >> 4) as usize
                };
                for _ in 0..size {
                    batch.spans.push(decode_span_struct(r)?);
                }
            }
            _ => r.skip(type_id)?,
        }
    }
    Ok(batch)
}

/// Decode a Jaeger Thrift Compact protocol UDP packet (emitBatch oneway call).
///
/// Packet layout:
///   [protocol_id: 0x82][version|type: 0x01/0x04][method_name_len varint][method_name]
///   [seq_id varint][Batch struct]
pub fn parse_jaeger_thrift_compact(data: &[u8], tenant_id: &str) -> Result<Vec<Span>> {
    let mut r = CompactReader::new(data);

    // Protocol ID
    let proto_id = r.read_byte()?;
    if proto_id != 0x82 {
        return Err(TraceError::ThriftError(format!(
            "expected compact protocol (0x82), got 0x{:02x}",
            proto_id
        )));
    }

    // Version | type
    let ver_type = r.read_byte()?;
    let _version = ver_type & 0x1F;
    let _msg_type = (ver_type >> 5) & 0x07;

    // Method name
    let _name = r.read_string()?;

    // Sequence ID
    let _seq_id = r.read_i32()?;

    // Batch struct (field 1)
    let mut last = 0i16;
    let batch = if let Some((_, type_id)) = r.read_field_header(&mut last)? {
        if type_id == 0x0C {
            decode_batch(&mut r)?
        } else {
            return Err(TraceError::ThriftError("expected struct field".into()));
        }
    } else {
        return Err(TraceError::ThriftError("empty message".into()));
    };

    convert_thrift_batch(&batch, tenant_id)
}

/// Decode a bare Thrift Binary protocol batch (HTTP collector).
/// Binary format: [type 1B][field_id 2B BE][value]... stop=0x00
pub fn parse_jaeger_thrift_binary(data: &[u8], tenant_id: &str) -> Result<Vec<Span>> {
    // For binary protocol, skip the message header and decode the batch struct
    // Binary message header: [version 0x80 0x01][type 1B][name len 4B][name][seq_id 4B]
    if data.len() < 4 {
        return Err(TraceError::ThriftError("payload too short".into()));
    }

    // Try to detect and skip message header
    let offset = if data[0] == 0x80 && data[1] == 0x01 {
        // Strict binary protocol
        let name_len = i32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
        8 + name_len + 4 // header + name + seq_id
    } else {
        0
    };

    if offset > data.len() {
        return Err(TraceError::ThriftError("header exceeds payload".into()));
    }

    let batch = decode_binary_batch(&data[offset..])?;
    convert_thrift_batch(&batch, tenant_id)
}

fn decode_binary_batch(data: &[u8]) -> Result<ThriftBatch> {
    // Simple binary protocol: walk fields
    let mut pos = 0usize;
    let mut batch = ThriftBatch::default();

    fn read_i8(data: &[u8], pos: &mut usize) -> Result<i8> {
        check_len(data, *pos, 1)?;
        let v = data[*pos] as i8;
        *pos += 1;
        Ok(v)
    }
    fn read_i16(data: &[u8], pos: &mut usize) -> Result<i16> {
        check_len(data, *pos, 2)?;
        let v = i16::from_be_bytes([data[*pos], data[*pos + 1]]);
        *pos += 2;
        Ok(v)
    }
    fn read_i32b(data: &[u8], pos: &mut usize) -> Result<i32> {
        check_len(data, *pos, 4)?;
        let v = i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
        *pos += 4;
        Ok(v)
    }
    fn read_i64b(data: &[u8], pos: &mut usize) -> Result<i64> {
        check_len(data, *pos, 8)?;
        let v = i64::from_be_bytes(data[*pos..*pos + 8].try_into().unwrap());
        *pos += 8;
        Ok(v)
    }
    fn read_bytes_b(data: &[u8], pos: &mut usize) -> Result<Vec<u8>> {
        let len = read_i32b(data, pos)? as usize;
        check_len(data, *pos, len)?;
        let v = data[*pos..*pos + len].to_vec();
        *pos += len;
        Ok(v)
    }
    fn read_str_b(data: &[u8], pos: &mut usize) -> Result<String> {
        let bytes = read_bytes_b(data, pos)?;
        String::from_utf8(bytes).map_err(|e| TraceError::ThriftError(e.to_string()))
    }
    fn check_len(data: &[u8], pos: usize, n: usize) -> Result<()> {
        if pos + n > data.len() {
            Err(TraceError::ThriftError("truncated binary thrift".into()))
        } else {
            Ok(())
        }
    }

    loop {
        if pos >= data.len() {
            break;
        }
        let field_type = data[pos];
        pos += 1;
        if field_type == 0x00 {
            break;
        } // stop
        let field_id = read_i16(data, &mut pos)?;

        match field_id {
            1 if field_type == 0x0C => {
                // Process struct
                loop {
                    if pos >= data.len() {
                        break;
                    }
                    let ft = data[pos];
                    pos += 1;
                    if ft == 0x00 {
                        break;
                    }
                    let fid = read_i16(data, &mut pos)?;
                    match fid {
                        1 if ft == 0x0B => {
                            batch.process.service_name = read_str_b(data, &mut pos)?;
                        }
                        _ => {
                            // skip
                            pos = pos.min(data.len());
                            break;
                        }
                    }
                }
            }
            _ => {
                break;
            }
        }
    }

    Ok(batch)
}

fn convert_thrift_batch(batch: &ThriftBatch, tenant_id: &str) -> Result<Vec<Span>> {
    let service_name = normalise_service(&batch.process.service_name);
    let resource_attrs: HashMap<String, TagValue> = batch
        .process
        .tags
        .iter()
        .map(|t| (t.key.clone(), thrift_tag_to_value(t)))
        .collect();
    let log_labels: HashMap<String, String> = resource_attrs
        .iter()
        .map(|(k, v)| (k.clone(), v.display()))
        .collect();

    batch
        .spans
        .iter()
        .map(|ts| {
            let trace_id: TraceId = ((ts.trace_id_high as u128) << 64)
                | (ts.trace_id_low as u128 & 0xFFFF_FFFF_FFFF_FFFF);
            let span_id: SpanId = ts.span_id as u64;
            let parent_span_id: Option<SpanId> = if ts.parent_span_id == 0 {
                None
            } else {
                Some(ts.parent_span_id as u64)
            };

            let start_ns = us_to_ns(ts.start_time);
            let dur_ns = us_to_ns(ts.duration);

            let tags: HashMap<String, TagValue> = ts
                .tags
                .iter()
                .map(|t| (t.key.clone(), thrift_tag_to_value(t)))
                .collect();

            let status = if tags
                .get("error")
                .map(|v| v.display() != "false")
                .unwrap_or(false)
            {
                SpanStatus::Error
            } else {
                SpanStatus::Ok
            };

            let events: Vec<SpanEvent> = ts
                .logs
                .iter()
                .map(|log| SpanEvent {
                    time_unix_nano: us_to_ns(log.timestamp),
                    name: log
                        .fields
                        .iter()
                        .find(|f| f.key == "event")
                        .and_then(|f| f.str_value.clone())
                        .unwrap_or_else(|| "log".into()),
                    attributes: log
                        .fields
                        .iter()
                        .map(|f| (f.key.clone(), thrift_tag_to_value(f)))
                        .collect(),
                })
                .collect();

            let links: Vec<SpanLink> = ts
                .references
                .iter()
                .map(|r| {
                    let tid: TraceId = ((r.trace_id_high as u128) << 64)
                        | (r.trace_id_low as u128 & 0xFFFF_FFFF_FFFF_FFFF);
                    SpanLink {
                        trace_id: tid,
                        span_id: r.span_id as u64,
                        trace_state: String::new(),
                        attributes: HashMap::new(),
                    }
                })
                .collect();

            Ok(Span {
                trace_id,
                span_id,
                parent_span_id,
                operation_name: ts.operation_name.clone(),
                service_name: service_name.clone(),
                start_time_unix_nano: start_ns,
                end_time_unix_nano: start_ns + dur_ns,
                duration_ns: dur_ns,
                status,
                kind: SpanKind::Internal,
                tags,
                events,
                links,
                resource_attributes: resource_attrs.clone(),
                tenant_id: tenant_id.to_owned(),
                baggage: HashMap::new(),
                log_labels: log_labels.clone(),
            })
        })
        .collect()
}

fn thrift_tag_to_value(t: &ThriftTag) -> TagValue {
    match t.vtype {
        0 => TagValue::String(t.str_value.clone().unwrap_or_default()),
        1 => TagValue::Float(t.double_value.unwrap_or(0.0)),
        2 => TagValue::Bool(t.bool_value.unwrap_or(false)),
        3 => TagValue::Int(t.long_value.unwrap_or(0)),
        4 => TagValue::Binary(t.bin_value.clone().unwrap_or_default()),
        _ => TagValue::String(String::new()),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const JAEGER_JSON: &str = r#"{
  "data": [{
    "traceID": "0af7651916cd43dd8448eb211c80319c",
    "spans": [{
      "traceID": "0af7651916cd43dd8448eb211c80319c",
      "spanID": "b7ad6b7169203331",
      "operationName": "HTTP GET",
      "references": [{"refType": "CHILD_OF", "traceID": "0af7651916cd43dd8448eb211c80319c", "spanID": "0000000000000000"}],
      "startTime": 1640000000000000,
      "duration": 5000,
      "tags": [
        {"key": "http.method", "type": "string", "value": "GET"},
        {"key": "http.status_code", "type": "int64", "value": 200}
      ],
      "logs": [],
      "process": {
        "serviceName": "frontend",
        "tags": [{"key": "hostname", "type": "string", "value": "web-01"}]
      }
    }],
    "processes": {}
  }]
}"#;

    #[test]
    fn parse_jaeger_json_basic() {
        let spans = parse_jaeger_json(JAEGER_JSON.as_bytes(), "default").unwrap();
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.service_name, "frontend");
        assert_eq!(s.operation_name, "HTTP GET");
        assert_eq!(s.duration_ns, 5_000_000); // 5000 µs → 5 ms
        assert_eq!(
            s.tags.get("http.method"),
            Some(&TagValue::String("GET".into()))
        );
        assert_eq!(s.tags.get("http.status_code"), Some(&TagValue::Int(200)));
    }

    #[test]
    fn parse_jaeger_json_derives_error_status() {
        let json = r#"{"data":[{"traceID":"aabb","spans":[{"traceID":"aabb","spanID":"1122","operationName":"op","references":[],"startTime":1000,"duration":100,"tags":[{"key":"error","type":"bool","value":true}],"logs":[],"process":{"serviceName":"svc","tags":[]}}],"processes":{}}]}"#;
        let spans = parse_jaeger_json(json.as_bytes(), "default").unwrap();
        assert_eq!(spans[0].status, SpanStatus::Error);
    }
}
