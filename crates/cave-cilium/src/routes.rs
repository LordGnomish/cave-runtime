// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST API for the cilium control plane (mounted at `/api/cilium`).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::ipam::ClusterCidr;
use crate::policy::Labels;
use crate::CiliumState;

pub fn create_router(state: Arc<CiliumState>) -> Router {
    Router::new()
        .route("/api/cilium/health", get(health))
        .route("/api/cilium/status", get(status))
        .route("/api/cilium/ipam/configure", post(configure_ipam))
        .route("/api/cilium/ipam/nodes", get(list_nodes))
        .route("/api/cilium/ipam/nodes/{node}", post(ensure_node))
        .route(
            "/api/cilium/ipam/nodes/{node}/allocate",
            post(allocate_ip),
        )
        .route("/api/cilium/identities/allocate", post(allocate_identity))
        .route("/api/cilium/hubble/flows", get(list_flows))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-cilium",
        "status": "ok",
        "upstream": "cilium",
        "version": "v1.19.4",
        "features": [
            "ebpf-loader", "network-policy", "cluster-pool-ipam",
            "hubble", "l7-mesh", "wireguard-ipsec", "pqc-hybrid"
        ]
    }))
}

async fn status(State(s): State<Arc<CiliumState>>) -> Json<serde_json::Value> {
    let nodes = s.ipam.lock().unwrap().nodes().len();
    let rules = s.policy.lock().unwrap().rule_count();
    let flows = s.hubble.lock().unwrap().len();
    Json(serde_json::json!({
        "ipam_nodes": nodes,
        "policy_rules": rules,
        "hubble_flows": flows,
    }))
}

#[derive(Deserialize)]
struct CidrSpec {
    cidr: String,
    node_mask: u8,
}

#[derive(Deserialize)]
struct ConfigureReq {
    cidrs: Vec<CidrSpec>,
}

async fn configure_ipam(
    State(s): State<Arc<CiliumState>>,
    Json(req): Json<ConfigureReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut cidrs = Vec::new();
    for c in &req.cidrs {
        let net = c
            .cidr
            .parse()
            .map_err(|_| (StatusCode::BAD_REQUEST, format!("bad cidr {}", c.cidr)))?;
        cidrs.push(ClusterCidr::new(net, c.node_mask));
    }
    let n = cidrs.len();
    s.ipam.lock().unwrap().configure(cidrs);
    Ok(Json(serde_json::json!({ "configured": n })))
}

async fn list_nodes(State(s): State<Arc<CiliumState>>) -> Json<Vec<serde_json::Value>> {
    let ipam = s.ipam.lock().unwrap();
    Json(
        ipam.nodes()
            .into_iter()
            .map(|n| {
                let cidr = ipam.node_cidr(&n).map(|c| c.to_string());
                serde_json::json!({ "node": n, "pod_cidr": cidr })
            })
            .collect(),
    )
}

async fn ensure_node(
    State(s): State<Arc<CiliumState>>,
    Path(node): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cidr = s
        .ipam
        .lock()
        .unwrap()
        .ensure_node(&node)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(serde_json::json!({ "node": node, "pod_cidr": cidr.to_string() })))
}

#[derive(Deserialize)]
struct AllocReq {
    owner: String,
}

async fn allocate_ip(
    State(s): State<Arc<CiliumState>>,
    Path(node): Path<String>,
    Json(req): Json<AllocReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let ip = s
        .ipam
        .lock()
        .unwrap()
        .allocate_ip(&node, &req.owner)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ip": ip.to_string(), "owner": req.owner })))
}

#[derive(Deserialize)]
struct LabelReq {
    labels: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize)]
struct IdentityResp {
    identity: u32,
}

async fn allocate_identity(
    State(s): State<Arc<CiliumState>>,
    Json(req): Json<LabelReq>,
) -> Json<IdentityResp> {
    let pairs: Vec<(&str, &str)> = req
        .labels
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let id = s.identities.lock().unwrap().allocate(&Labels::new(&pairs));
    Json(IdentityResp { identity: id })
}

async fn list_flows(State(s): State<Arc<CiliumState>>) -> Json<serde_json::Value> {
    let hubble = s.hubble.lock().unwrap();
    Json(serde_json::json!({ "count": hubble.len() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_reports_module() {
        let Json(v) = health().await;
        assert_eq!(v["module"], "cave-cilium");
        assert_eq!(v["version"], "v1.19.4");
    }

    #[tokio::test]
    async fn configure_then_ensure_node_returns_cidr() {
        let st = crate::new_state();
        let Json(c) = configure_ipam(
            State(st.clone()),
            Json(ConfigureReq {
                cidrs: vec![CidrSpec {
                    cidr: "10.50.0.0/16".into(),
                    node_mask: 24,
                }],
            }),
        )
        .await
        .unwrap();
        assert_eq!(c["configured"], 1);

        let Json(n) = ensure_node(State(st.clone()), Path("node-a".into()))
            .await
            .unwrap();
        assert_eq!(n["pod_cidr"], "10.50.0.0/24");

        // Allocate an IP inside that node's CIDR.
        let Json(a) = allocate_ip(
            State(st.clone()),
            Path("node-a".into()),
            Json(AllocReq {
                owner: "pod-1".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(a["ip"], "10.50.0.1");
    }

    #[tokio::test]
    async fn allocate_identity_is_stable() {
        let st = crate::new_state();
        let mut labels = std::collections::BTreeMap::new();
        labels.insert("app".to_string(), "web".to_string());
        let Json(a) = allocate_identity(State(st.clone()), Json(LabelReq { labels: labels.clone() })).await;
        let Json(b) = allocate_identity(State(st.clone()), Json(LabelReq { labels })).await;
        assert_eq!(a.identity, b.identity);
        assert!(a.identity >= 256);
    }

    #[tokio::test]
    async fn router_builds() {
        let _ = create_router(crate::new_state());
    }
}
