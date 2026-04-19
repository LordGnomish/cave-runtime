//! REST API routes for networking.

use crate::dataplane::NetState;
use crate::models::*;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

pub fn create_router(state: Arc<NetState>) -> Router {
    Router::new()
        .route("/api/net/health", get(health))
        .route("/api/net/pods", get(list_pod_ips).post(allocate_pod_ip))
        .route("/api/net/pods/{ns}/{name}", delete(release_pod_ip))
        .route("/api/net/services", get(list_services).post(register_service))
        .route("/api/net/services/{ns}/{name}", delete(remove_service))
        .route("/api/net/policies", get(list_policies).post(apply_policy))
        .route("/api/net/policies/{ns}/{name}", delete(remove_policy))
        .route("/api/net/flows", get(list_flows))
        .route("/api/net/check", post(check_policy))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"module":"cave-net","status":"ok","upstream":"cilium","features":["pod-ip","clusterip","network-policy","flow-records"]}))
}

async fn list_pod_ips(State(s): State<Arc<NetState>>) -> Json<Vec<PodNetwork>> {
    Json(s.pods.iter().map(|r| r.value().clone()).collect())
}

#[derive(Deserialize)]
struct AllocReq { pod_name: String, namespace: String, node_name: String, labels: HashMap<String, String> }

async fn allocate_pod_ip(State(s): State<Arc<NetState>>, Json(req): Json<AllocReq>) -> (StatusCode, Json<PodNetwork>) {
    let pn = s.allocate_pod_ip(&req.pod_name, &req.namespace, &req.node_name, req.labels);
    (StatusCode::CREATED, Json(pn))
}

async fn release_pod_ip(State(s): State<Arc<NetState>>, Path((ns, name)): Path<(String, String)>) -> StatusCode {
    s.release_pod_ip(&name, &ns);
    StatusCode::OK
}

async fn list_services(State(s): State<Arc<NetState>>) -> Json<Vec<ServiceEntry>> {
    Json(s.services.iter().map(|r| r.value().clone()).collect())
}

async fn register_service(State(s): State<Arc<NetState>>, Json(svc): Json<ServiceEntry>) -> (StatusCode, Json<ServiceEntry>) {
    s.register_service(svc.clone());
    (StatusCode::CREATED, Json(svc))
}

async fn remove_service(State(s): State<Arc<NetState>>, Path((ns, name)): Path<(String, String)>) -> StatusCode {
    s.remove_service(&name, &ns);
    StatusCode::OK
}

async fn list_policies(State(s): State<Arc<NetState>>) -> Json<Vec<NetworkPolicy>> {
    Json(s.policies.iter().map(|r| r.value().clone()).collect())
}

async fn apply_policy(State(s): State<Arc<NetState>>, Json(policy): Json<NetworkPolicy>) -> (StatusCode, Json<NetworkPolicy>) {
    s.apply_policy(policy.clone());
    (StatusCode::CREATED, Json(policy))
}

async fn remove_policy(State(s): State<Arc<NetState>>, Path((ns, name)): Path<(String, String)>) -> StatusCode {
    s.remove_policy(&name, &ns);
    StatusCode::OK
}

async fn list_flows(State(s): State<Arc<NetState>>) -> Json<Vec<FlowRecord>> {
    Json(s.flows.iter().map(|r| r.value().clone()).collect())
}

#[derive(Deserialize)]
struct CheckReq { src_pod: String, src_ns: String, dst_pod: String, dst_ns: String, dst_port: u16 }

async fn check_policy(State(s): State<Arc<NetState>>, Json(req): Json<CheckReq>) -> Json<serde_json::Value> {
    let verdict = s.check_policy(&req.src_pod, &req.src_ns, &req.dst_pod, &req.dst_ns, req.dst_port);
    Json(serde_json::json!({"verdict": verdict}))
}
