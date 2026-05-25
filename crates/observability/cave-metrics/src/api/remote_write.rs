// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Remote write API handler.

use std::sync::Arc;
use axum::{
    extract::State,
    http::StatusCode,
    body::Bytes,
    response::Json,
};
use serde_json::{json, Value};
use crate::remote_write::decode_write_request;
use crate::state::MetricsState;

pub async fn remote_write(
    State(state): State<Arc<MetricsState>>,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let timeseries = decode_write_request(&body).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(json!({"status":"error","error":e.to_string()})))
    })?;

    for ts in timeseries {
        for sample in &ts.samples {
            if let Err(e) = state.tsdb.append(ts.labels.clone(), sample.timestamp, sample.value) {
                tracing::warn!("Failed to append remote write sample: {}", e);
            }
        }
    }

    Ok(Json(json!({"status":"success"})))
}
