// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! /api/v1/metadata

use crate::state::MetricsState;
use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct MetadataParams {
    pub metric: Option<String>,
    pub limit: Option<u64>,
}

pub async fn metric_metadata(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<MetadataParams>,
) -> Json<serde_json::Value> {
    // Return metadata for all known metric names.
    let names = if let Some(m) = params.metric {
        vec![m]
    } else {
        state.tsdb.label_values("__name__", &[])
    };

    let limit = params.limit.unwrap_or(u64::MAX) as usize;
    let data: serde_json::Map<String, serde_json::Value> = names
        .into_iter()
        .take(limit)
        .map(|name| {
            let meta = serde_json::json!([{
                "type": "gauge",
                "help": "",
                "unit": "",
            }]);
            (name, meta)
        })
        .collect();

    Json(serde_json::json!({ "status": "success", "data": data }))
}
