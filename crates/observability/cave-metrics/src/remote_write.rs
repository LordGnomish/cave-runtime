// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus remote_write protocol (protobuf + snappy).

#![allow(dead_code)]

use bytes::Bytes;
use prost::Message;
use crate::error::{MetricsError, MetricsResult};
use crate::model::{Labels, Sample, TimeSeries};

// Protobuf types for remote_write
#[derive(Clone, PartialEq, prost::Message)]
pub struct WriteRequest {
    #[prost(message, repeated, tag = "1")]
    pub timeseries: Vec<ProtoTimeSeries>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoTimeSeries {
    #[prost(message, repeated, tag = "1")]
    pub labels: Vec<ProtoLabel>,
    #[prost(message, repeated, tag = "2")]
    pub samples: Vec<ProtoSample>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoLabel {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSample {
    #[prost(double, tag = "1")]
    pub value: f64,
    #[prost(int64, tag = "2")]
    pub timestamp: i64,
}

/// Encode a slice of TimeSeries to snappy-compressed protobuf bytes.
pub fn encode_write_request(timeseries: &[TimeSeries]) -> MetricsResult<Bytes> {
    let req = WriteRequest {
        timeseries: timeseries.iter().map(to_proto_ts).collect(),
    };
    let proto_bytes = req.encode_to_vec();
    let compressed = snap::raw::Encoder::new()
        .compress_vec(&proto_bytes)
        .map_err(|e| MetricsError::Compression(e.to_string()))?;
    Ok(Bytes::from(compressed))
}

/// Decode snappy-compressed protobuf bytes to TimeSeries.
pub fn decode_write_request(body: &[u8]) -> MetricsResult<Vec<TimeSeries>> {
    let decompressed = snap::raw::Decoder::new()
        .decompress_vec(body)
        .map_err(|e| MetricsError::Compression(e.to_string()))?;
    let req = WriteRequest::decode(decompressed.as_slice())
        .map_err(|e| MetricsError::Proto(e.to_string()))?;
    Ok(req.timeseries.iter().map(from_proto_ts).collect())
}

fn to_proto_ts(ts: &TimeSeries) -> ProtoTimeSeries {
    ProtoTimeSeries {
        labels: ts.labels.0.iter().map(|(k, v)| ProtoLabel {
            name: k.clone(),
            value: v.clone(),
        }).collect(),
        samples: ts.samples.iter().map(|s| ProtoSample {
            value: s.value,
            timestamp: s.timestamp,
        }).collect(),
    }
}

fn from_proto_ts(pts: &ProtoTimeSeries) -> TimeSeries {
    let labels = Labels::from_pairs(pts.labels.iter().map(|l| (l.name.clone(), l.value.clone())));
    let samples = pts.samples.iter().map(|s| Sample {
        timestamp: s.timestamp,
        value: s.value,
    }).collect();
    TimeSeries { labels, samples }
}
