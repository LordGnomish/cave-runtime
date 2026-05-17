// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus remote_read protocol handler.

use crate::error::Result;
use crate::model::LabelMatcher;
use crate::tsdb::Tsdb;
use std::sync::Arc;

use super::remote_write::{
    ReadRequest, ReadResponse, QueryResult as ProtoQueryResult,
    ProtoTimeSeries, ProtoLabel, ProtoSample,
    MatchType, encode_write_request, decode_write_request,
};
use prost::Message;
use snap::raw::{Decoder, Encoder};
use crate::error::MetricsError;

/// Decode a snappy-compressed ReadRequest.
pub fn decode_read_request(body: &[u8]) -> Result<ReadRequest> {
    let raw = Decoder::new().decompress_vec(body)
        .map_err(|e| MetricsError::Ingestion(format!("snappy decode: {}", e)))?;
    ReadRequest::decode(raw.as_slice())
        .map_err(|e| MetricsError::Ingestion(format!("protobuf decode: {}", e)))
}

/// Encode a ReadResponse as snappy-compressed protobuf.
pub fn encode_read_response(resp: &ReadResponse) -> Result<Vec<u8>> {
    let proto_bytes = resp.encode_to_vec();
    Encoder::new().compress_vec(&proto_bytes)
        .map_err(|e| MetricsError::Ingestion(format!("snappy encode: {}", e)))
}

/// Execute a ReadRequest against the TSDB and produce a ReadResponse.
pub fn execute_read(req: ReadRequest, tsdb: &Tsdb) -> Result<ReadResponse> {
    let mut results = Vec::new();

    for query in req.queries {
        // Convert proto matchers to our model
        let matchers: Vec<LabelMatcher> = query.matchers.into_iter()
            .filter_map(|m| {
                match m.r#type {
                    t if t == MatchType::Eq  as i32 => Some(LabelMatcher::equal(&m.name, &m.value)),
                    t if t == MatchType::Neq as i32 => Some(LabelMatcher::not_equal(&m.name, &m.value)),
                    t if t == MatchType::Re  as i32 => LabelMatcher::regex(&m.name, &m.value).ok(),
                    t if t == MatchType::Nre as i32 => LabelMatcher::not_regex(&m.name, &m.value).ok(),
                    _ => None,
                }
            })
            .collect();

        let series = tsdb.select(&matchers, query.start_timestamp_ms, query.end_timestamp_ms);

        let timeseries: Vec<ProtoTimeSeries> = series.into_iter().map(|(labels, samps)| {
            ProtoTimeSeries {
                labels: labels.iter().map(|(k, v)| ProtoLabel { name: k.to_string(), value: v.to_string() }).collect(),
                samples: samps.into_iter().map(|s| ProtoSample { value: s.value, timestamp: s.timestamp_ms }).collect(),
                exemplars: vec![],
            }
        }).collect();

        results.push(ProtoQueryResult { timeseries });
    }

    Ok(ReadResponse { results })
}
