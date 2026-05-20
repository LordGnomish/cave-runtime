// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! /api/v1/series

use crate::model::LabelMatcher;
use crate::state::MetricsState;
use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct SeriesParams {
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub limit: Option<u64>,
}

pub async fn list_series(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<SeriesParams>,
) -> Json<serde_json::Value> {
    let matchers = parse_matchers(params.matchers.as_deref().unwrap_or(&[]));
    let series = state.tsdb.series_for(&matchers);
    let data: Vec<serde_json::Value> = series
        .into_iter()
        .map(|labels| serde_json::json!(labels.0))
        .collect();
    Json(serde_json::json!({ "status": "success", "data": data }))
}

fn parse_matchers(raw: &[String]) -> Vec<LabelMatcher> {
    raw.iter().flat_map(|m| parse_single_matcher(m)).collect()
}

/// Parse a simple `{key="value"}` or `metric_name` matcher expression.
fn parse_single_matcher(s: &str) -> Vec<LabelMatcher> {
    use crate::promql::ast::Expr;
    use crate::promql::parse;

    let expr_str = if s.contains('{') {
        s.to_string()
    } else {
        format!("{}{{__name__=\"{}\"}}", s, s)
    };

    if let Ok(Expr::VectorSelector(vs)) = parse(&expr_str) {
        vs.matchers
    } else if !s.is_empty() {
        vec![LabelMatcher::equal("__name__", s)]
    } else {
        vec![]
    }
}
