// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus federation endpoint: GET /federate?match[]=...
//! Returns Prometheus text exposition format with matching series.

use crate::model::LabelMatcher;
use crate::state::MetricsState;
use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct FederateParams {
    #[serde(rename = "match[]")]
    pub matchers: Vec<String>,
    pub honor_labels: Option<bool>,
}

pub async fn federate(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<FederateParams>,
) -> Response<String> {
    let honor_labels = params.honor_labels.unwrap_or(false);

    let now_ms = now_ms();
    let lookback_ms = 5 * 60 * 1000; // 5m default

    let mut body = String::new();
    let mut seen_names = std::collections::HashSet::new();

    for matcher_str in &params.matchers {
        let matchers = parse_matchers(matcher_str);
        let result = state.tsdb.select_at(&matchers, now_ms, lookback_ms);

        for (labels, sample) in result {
            let metric_name = labels.metric_name().unwrap_or("").to_string();

            if !seen_names.contains(&metric_name) {
                body.push_str(&format!("# TYPE {} untyped\n", metric_name));
                seen_names.insert(metric_name.clone());
            }

            // Render labels
            let label_str: Vec<String> = labels
                .iter()
                .filter(|(k, _)| *k != "__name__")
                .map(|(k, v)| format!("{}=\"{}\"", k, escape_label_value(v)))
                .collect();

            if label_str.is_empty() {
                body.push_str(&format!(
                    "{} {} {}\n",
                    metric_name,
                    format_value(sample.value),
                    sample.timestamp_ms
                ));
            } else {
                body.push_str(&format!(
                    "{}{{{}}} {} {}\n",
                    metric_name,
                    label_str.join(","),
                    format_value(sample.value),
                    sample.timestamp_ms
                ));
            }
        }
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )
        .body(body)
        .unwrap_or_else(|_| Response::new(String::new()))
}

fn parse_matchers(s: &str) -> Vec<LabelMatcher> {
    use crate::promql::{ast::Expr, parse};
    if let Ok(Expr::VectorSelector(vs)) = parse(s) {
        vs.matchers
    } else {
        vec![LabelMatcher::equal("__name__", s)]
    }
}

fn escape_label_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn format_value(v: f64) -> String {
    if v.is_infinite() {
        if v > 0.0 {
            "+Inf".to_string()
        } else {
            "-Inf".to_string()
        }
    } else if v.is_nan() {
        "NaN".to_string()
    } else {
        v.to_string()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
