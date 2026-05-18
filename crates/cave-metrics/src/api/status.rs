// SPDX-License-Identifier: AGPL-3.0-or-later
//! /api/v1/status/* endpoints

use axum::{extract::State, Json};
use std::sync::Arc;
use crate::state::MetricsState;

pub async fn config(State(_state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "data": { "yaml": "# cave-metrics configuration\n" }
    }))
}

pub async fn flags(State(_state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "storage.tsdb.retention.time": "15d",
            "storage.tsdb.path": "/data/tsdb",
            "web.enable-remote-write-receiver": "true",
            "web.enable-otlp-receiver": "true",
        }
    }))
}

pub async fn runtime_info(State(_state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "startTime": chrono::Utc::now().to_rfc3339(),
            "CWD": "/",
            "reloadConfigSuccess": true,
            "lastConfigTime": chrono::Utc::now().to_rfc3339(),
            "corruptionCount": 0,
            "goroutineCount": 1,
            "GOMAXPROCS": 1,
            "GOMEMLIMIT": 0,
            "GOGC": "",
            "GODEBUG": "",
            "storageRetention": "15d",
        }
    }))
}

pub async fn build_info(State(_state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "version": env!("CARGO_PKG_VERSION"),
            "revision": "unknown",
            "branch": "main",
            "buildUser": "cave-runtime",
            "buildDate": "unknown",
            "goVersion": "N/A (Rust)",
        }
    }))
}

pub async fn tsdb_stats(State(state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    let num_label_names = state.tsdb.label_names(&[]).len();
    let num_series = state.tsdb.series_for(&[]).len();

    Json(serde_json::json!({
        "status": "success",
        "data": {
            "headStats": {
                "numSamples": 0,
                "numSeries": num_series,
                "numLabelPairs": num_label_names,
                "chunkCount": 0,
                "minTime": 0,
                "maxTime": 0,
            },
            "seriesCountByMetricName": [],
            "labelValueCountByLabelName": [],
            "memoryInBytesByLabelName": [],
            "seriesCountByLabelValuePair": [],
        }
    }))
}

pub async fn wal_replay(State(_state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "min": 0,
            "max": 0,
            "current": 0,
            "state": "done",
        }
    }))
}
