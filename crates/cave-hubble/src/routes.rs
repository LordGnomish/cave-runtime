//! HTTP routes for cave-hubble.

use crate::filter::FlowQuery;
use crate::models::IngestFlowRequest;
use crate::HubbleState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<HubbleState>) -> Router {
    Router::new()
        .route("/api/v1/hubble/flows", get(list_flows).post(ingest_flow))
        .route("/api/v1/hubble/flows/stats", get(flow_stats))
        .route("/api/v1/hubble/flows/aggregated", get(aggregated_flows))
        .route("/api/v1/hubble/flows/dropped", get(top_dropped))
        .route("/api/v1/hubble/flows/{id}", get(get_flow))
        .route("/api/v1/hubble/dns", get(list_dns))
        .route("/api/v1/hubble/dns/{name}", get(lookup_dns))
        .with_state(state)
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": msg })))
}

async fn list_flows(
    Query(q): Query<FlowQuery>,
    State(state): State<Arc<HubbleState>>,
) -> Json<serde_json::Value> {
    let filter = q.into_filter();
    let flows = state.flows.query(&filter);
    let count = flows.len();
    Json(serde_json::json!({ "flows": flows, "count": count }))
}

async fn ingest_flow(
    State(state): State<Arc<HubbleState>>,
    Json(req): Json<IngestFlowRequest>,
) -> Json<serde_json::Value> {
    if let Some(dns) = &req.dns {
        if let Some(query) = &dns.query {
            let values = dns.response.clone().unwrap_or_default();
            state.dns.observe(query, "A", values, 300, req.source_pod.clone(), Some(req.source_ns.clone()));
        }
    }
    let flow = state.flows.ingest(req);
    state.aggregator.record(&flow);
    Json(serde_json::json!({ "flow": flow }))
}

async fn get_flow(
    Path(id): Path<String>,
    State(state): State<Arc<HubbleState>>,
) -> ApiResult<serde_json::Value> {
    match state.flows.get(&id) {
        Ok(f) => Ok(Json(serde_json::json!({ "flow": f }))),
        Err(e) => Err(not_found(&e.to_string())),
    }
}

async fn flow_stats(State(state): State<Arc<HubbleState>>) -> Json<serde_json::Value> {
    let stats = state.flows.stats();
    Json(serde_json::json!({ "stats": stats }))
}

async fn aggregated_flows(State(state): State<Arc<HubbleState>>) -> Json<serde_json::Value> {
    let agg = state.aggregator.list();
    let count = agg.len();
    Json(serde_json::json!({ "aggregated": agg, "count": count }))
}

async fn top_dropped(State(state): State<Arc<HubbleState>>) -> Json<serde_json::Value> {
    let dropped = state.aggregator.top_dropped(20);
    Json(serde_json::json!({ "top_dropped": dropped }))
}

async fn list_dns(State(state): State<Arc<HubbleState>>) -> Json<serde_json::Value> {
    let records = state.dns.list();
    let count = records.len();
    Json(serde_json::json!({ "dns_records": records, "count": count }))
}

async fn lookup_dns(
    Path(name): Path<String>,
    State(state): State<Arc<HubbleState>>,
) -> Json<serde_json::Value> {
    let records = state.dns.lookup(&name);
    Json(serde_json::json!({ "name": name, "records": records }))
}
