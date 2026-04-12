//! Label names and values API handlers.

use std::sync::Arc;
use axum::{
    extract::{Path, State},
    response::Json,
};
use serde_json::{json, Value};
use crate::state::MetricsState;

pub async fn label_names(State(state): State<Arc<MetricsState>>) -> Json<Value> {
    let names = state.tsdb.label_names();
    Json(json!({"status":"success","data":names}))
}

pub async fn label_values(
    State(state): State<Arc<MetricsState>>,
    Path(name): Path<String>,
) -> Json<Value> {
    let values = state.tsdb.label_values(&name);
    Json(json!({"status":"success","data":values}))
}
