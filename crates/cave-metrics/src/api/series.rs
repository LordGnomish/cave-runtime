//! Series matching API handler.

use std::sync::Arc;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::promql::parser::parse;
use crate::promql::ast::Expr;
use crate::model::LabelMatcher;
use crate::state::MetricsState;

#[derive(Deserialize)]
pub struct SeriesParams {
    #[serde(rename = "match[]")]
    pub matchers: Vec<String>,
}

pub async fn series(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<SeriesParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut all_matchers: Vec<LabelMatcher> = Vec::new();
    for m in &params.matchers {
        match parse(m) {
            Ok(Expr::VectorSelector { matchers, .. }) => all_matchers.extend(matchers),
            Ok(_) => {}
            Err(e) => return Err((StatusCode::BAD_REQUEST, Json(json!({"status":"error","error":e.to_string()})))),
        }
    }

    let series = state.tsdb.series_for(&all_matchers);
    let result: Vec<Value> = series.into_iter().map(|labels| {
        let metric: serde_json::Map<String, Value> = labels.0.into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        Value::Object(metric)
    }).collect();

    Ok(Json(json!({"status":"success","data":result})))
}
