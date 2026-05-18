// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus remote_write protocol: protobuf + Snappy compression.
//! Compatible with Prometheus remote_write 1.0 and 2.0.

use prost::Message;
use snap::raw::{Decoder, Encoder};
use crate::error::{MetricsError, Result};
use crate::model::{Labels, Sample, TimeSeries};
use super::IngestedBatch;

// ─── Protobuf types (inline, no separate .proto file needed) ────────────────

/// Prometheus remote_write WriteRequest
#[derive(Clone, PartialEq, Message)]
pub struct WriteRequest {
    #[prost(message, repeated, tag = "1")]
    pub timeseries: Vec<ProtoTimeSeries>,
    #[prost(message, repeated, tag = "3")]
    pub metadata: Vec<MetricMetadata>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ProtoTimeSeries {
    #[prost(message, repeated, tag = "1")]
    pub labels: Vec<ProtoLabel>,
    #[prost(message, repeated, tag = "2")]
    pub samples: Vec<ProtoSample>,
    #[prost(message, repeated, tag = "3")]
    pub exemplars: Vec<Exemplar>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ProtoLabel {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ProtoSample {
    #[prost(double, tag = "1")]
    pub value: f64,
    #[prost(int64, tag = "2")]
    pub timestamp: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Exemplar {
    #[prost(message, repeated, tag = "1")]
    pub labels: Vec<ProtoLabel>,
    #[prost(double, tag = "2")]
    pub value: f64,
    #[prost(int64, tag = "3")]
    pub timestamp: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct MetricMetadata {
    #[prost(enumeration = "MetricType", tag = "1")]
    pub r#type: i32,
    #[prost(string, tag = "2")]
    pub metric_family_name: String,
    #[prost(string, tag = "3")]
    pub help: String,
    #[prost(string, tag = "4")]
    pub unit: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum MetricType {
    Unknown = 0,
    Counter = 1,
    Gauge = 2,
    Histogram = 3,
    GaugeHistogram = 4,
    Summary = 5,
    Info = 6,
    StateSet = 7,
}

/// ReadRequest for remote_read.
#[derive(Clone, PartialEq, Message)]
pub struct ReadRequest {
    #[prost(message, repeated, tag = "1")]
    pub queries: Vec<Query>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Query {
    #[prost(int64, tag = "1")]
    pub start_timestamp_ms: i64,
    #[prost(int64, tag = "2")]
    pub end_timestamp_ms: i64,
    #[prost(message, repeated, tag = "3")]
    pub matchers: Vec<LabelMatcher>,
}

#[derive(Clone, PartialEq, Message)]
pub struct LabelMatcher {
    #[prost(enumeration = "MatchType", tag = "1")]
    pub r#type: i32,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum MatchType {
    Eq  = 0,
    Neq = 1,
    Re  = 2,
    Nre = 3,
}

#[derive(Clone, PartialEq, Message)]
pub struct ReadResponse {
    #[prost(message, repeated, tag = "1")]
    pub results: Vec<QueryResult>,
}

#[derive(Clone, PartialEq, Message)]
pub struct QueryResult {
    #[prost(message, repeated, tag = "1")]
    pub timeseries: Vec<ProtoTimeSeries>,
}

// ─── Encode / decode ─────────────────────────────────────────────────────────

/// Encode a WriteRequest as snappy-compressed protobuf bytes.
pub fn encode_write_request(req: &WriteRequest) -> Result<Vec<u8>> {
    let proto_bytes = req.encode_to_vec();
    Encoder::new().compress_vec(&proto_bytes)
        .map_err(|e| MetricsError::Ingestion(format!("snappy encode: {}", e)))
}

/// Decode a snappy-compressed protobuf WriteRequest.
pub fn decode_write_request(body: &[u8]) -> Result<WriteRequest> {
    let raw = Decoder::new().decompress_vec(body)
        .map_err(|e| MetricsError::Ingestion(format!("snappy decode: {}", e)))?;
    WriteRequest::decode(raw.as_slice())
        .map_err(|e| MetricsError::Ingestion(format!("protobuf decode: {}", e)))
}

/// Convert a WriteRequest into our internal TimeSeries representation.
pub fn write_request_to_batch(req: WriteRequest) -> IngestedBatch {
    req.timeseries.into_iter().map(|pts| {
        let labels = Labels::from_pairs(pts.labels.into_iter().map(|l| (l.name, l.value)));
        let samples = pts.samples.into_iter().map(|s| Sample::new(s.timestamp, s.value)).collect();
        TimeSeries { labels, samples }
    }).collect()
}

/// Convert internal TimeSeries into a WriteRequest.
pub fn batch_to_write_request(batch: IngestedBatch) -> WriteRequest {
    WriteRequest {
        timeseries: batch.into_iter().map(|ts| {
            ProtoTimeSeries {
                labels: ts.labels.iter().map(|(k, v)| ProtoLabel { name: k.to_string(), value: v.to_string() }).collect(),
                samples: ts.samples.into_iter().map(|s| ProtoSample { value: s.value, timestamp: s.timestamp_ms }).collect(),
                exemplars: vec![],
            }
        }).collect(),
        metadata: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_write_roundtrip() {
        let req = WriteRequest {
            timeseries: vec![ProtoTimeSeries {
                labels: vec![
                    ProtoLabel { name: "__name__".into(), value: "cpu_usage".into() },
                    ProtoLabel { name: "job".into(), value: "test".into() },
                ],
                samples: vec![ProtoSample { value: 0.75, timestamp: 1_700_000_000_000 }],
                exemplars: vec![],
            }],
            metadata: vec![],
        };

        let encoded = encode_write_request(&req).unwrap();
        let decoded = decode_write_request(&encoded).unwrap();
        assert_eq!(decoded.timeseries.len(), 1);
        assert_eq!(decoded.timeseries[0].samples[0].value, 0.75);
    }
}
