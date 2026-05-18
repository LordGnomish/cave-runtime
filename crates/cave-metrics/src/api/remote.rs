// SPDX-License-Identifier: AGPL-3.0-or-later
//! Remote write/read, OTLP, StatsD, Graphite, InfluxDB ingestion handlers.

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;
use crate::state::MetricsState;
use crate::ingestion::{graphite, influx, otlp, statsd};
use crate::ingestion::remote_write::{decode_write_request, write_request_to_batch};
use crate::ingestion::remote_read::{decode_read_request, encode_read_response, execute_read};

// ─── Prometheus remote_write ─────────────────────────────────────────────────

pub async fn remote_write(
    State(state): State<Arc<MetricsState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    match decode_write_request(&body) {
        Ok(req) => {
            let batch = write_request_to_batch(req);
            for ts in batch {
                state.tsdb.append_many(&ts);
            }
            StatusCode::NO_CONTENT
        }
        Err(e) => {
            tracing::warn!("remote_write decode error: {}", e);
            StatusCode::BAD_REQUEST
        }
    }
}

// ─── Prometheus remote_read ──────────────────────────────────────────────────

pub async fn remote_read(
    State(state): State<Arc<MetricsState>>,
    body: Bytes,
) -> (StatusCode, Bytes) {
    match decode_read_request(&body) {
        Ok(req) => {
            match execute_read(req, &state.tsdb) {
                Ok(resp) => {
                    match encode_read_response(&resp) {
                        Ok(encoded) => (StatusCode::OK, Bytes::from(encoded)),
                        Err(e) => {
                            tracing::warn!("remote_read encode error: {}", e);
                            (StatusCode::INTERNAL_SERVER_ERROR, Bytes::new())
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("remote_read execute error: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, Bytes::new())
                }
            }
        }
        Err(e) => {
            tracing::warn!("remote_read decode error: {}", e);
            (StatusCode::BAD_REQUEST, Bytes::new())
        }
    }
}

// ─── OTLP ────────────────────────────────────────────────────────────────────

pub async fn otlp_metrics(
    State(state): State<Arc<MetricsState>>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    let content_type = headers.get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid UTF-8" }))),
    };

    match otlp::parse_json(body_str) {
        Ok(batch) => {
            for ts in batch {
                state.tsdb.append_many(&ts);
            }
            (StatusCode::OK, Json(serde_json::json!({})))
        }
        Err(e) => {
            tracing::warn!("OTLP parse error: {}", e);
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
        }
    }
}

// ─── StatsD ──────────────────────────────────────────────────────────────────

pub async fn statsd_ingest(
    State(state): State<Arc<MetricsState>>,
    body: Bytes,
) -> StatusCode {
    let input = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    for ts in statsd::parse_batch(input) {
        state.tsdb.append_many(&ts);
    }
    StatusCode::NO_CONTENT
}

// ─── Graphite ────────────────────────────────────────────────────────────────

pub async fn graphite_ingest(
    State(state): State<Arc<MetricsState>>,
    body: Bytes,
) -> StatusCode {
    let input = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    for ts in graphite::parse_batch(input) {
        state.tsdb.append_many(&ts);
    }
    StatusCode::NO_CONTENT
}

// ─── InfluxDB line protocol ──────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct InfluxParams {
    pub db: Option<String>,
    pub org: Option<String>,
    pub bucket: Option<String>,
    pub precision: Option<String>,
}

pub async fn influx_write(
    State(state): State<Arc<MetricsState>>,
    Query(_params): Query<InfluxParams>,
    body: Bytes,
) -> StatusCode {
    let input = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    for ts in influx::parse(input) {
        state.tsdb.append_many(&ts);
    }
    StatusCode::NO_CONTENT
}
