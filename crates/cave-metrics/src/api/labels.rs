// SPDX-License-Identifier: AGPL-3.0-or-later
//! /api/v1/labels and /api/v1/label/{name}/values

use axum::{extract::{Path, Query, State}, Json};
use serde::Deserialize;
use std::sync::Arc;
use crate::model::LabelMatcher;
use crate::state::MetricsState;

#[derive(Debug, Deserialize)]
pub struct LabelParams {
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub limit: Option<u64>,
}

pub async fn list_labels(
    State(state): State<Arc<MetricsState>>,
    Query(_params): Query<LabelParams>,
) -> Json<serde_json::Value> {
    let names = state.tsdb.label_names(&[]);
    Json(serde_json::json!({ "status": "success", "data": names }))
}

pub async fn label_values(
    State(state): State<Arc<MetricsState>>,
    Path(name): Path<String>,
    Query(_params): Query<LabelParams>,
) -> Json<serde_json::Value> {
    let values = state.tsdb.label_values(&name, &[]);
    Json(serde_json::json!({ "status": "success", "data": values }))
}
