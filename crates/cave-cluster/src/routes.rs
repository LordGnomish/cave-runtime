// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-cluster.

use crate::ClusterState;
use crate::addons::AddonManager;
use crate::cluster::{CreateClusterRequest, UpgradeClusterRequest};
use crate::etcd::EtcdBackupStore;
use crate::kubeconfig::{CredentialType, generate, to_yaml};
use crate::nodepool::{CreateNodePoolRequest, NodePoolStore};
use crate::tenant::TenantStore;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

type AppState = Arc<InnerState>;

struct InnerState {
    cluster: Arc<crate::ClusterState>,
    node_pools: Arc<NodePoolStore>,
    addons: Arc<AddonManager>,
    etcd_store: Arc<EtcdBackupStore>,
    tenants: Arc<TenantStore>,
}

pub fn create_router(state: Arc<ClusterState>) -> Router {
    let inner = Arc::new(InnerState {
        cluster: Arc::clone(&state),
        node_pools: Arc::new(NodePoolStore::new()),
        addons: Arc::new(AddonManager::new()),
        etcd_store: Arc::new(EtcdBackupStore::new("s3://cave-backups/etcd".into())),
        tenants: Arc::new(TenantStore::new()),
    });

    Router::new()
        // Health
        .route("/api/cluster/health", get(health))
        // Cluster CRUD
        .route(
            "/api/cluster/clusters",
            get(list_clusters).post(create_cluster),
        )
        .route(
            "/api/cluster/clusters/{name}",
            get(get_cluster).delete(delete_cluster),
        )
        .route(
            "/api/cluster/clusters/{name}/upgrade",
            post(upgrade_cluster),
        )
        .route("/api/cluster/clusters/{name}/health", get(cluster_health))
        .route(
            "/api/cluster/clusters/{name}/kubeconfig",
            get(get_kubeconfig),
        )
        // Node pools
        .route(
            "/api/cluster/clusters/{name}/nodepools",
            get(list_node_pools).post(create_node_pool),
        )
        .route(
            "/api/cluster/clusters/{name}/nodepools/{pool}",
            get(get_node_pool).delete(delete_node_pool),
        )
        .route(
            "/api/cluster/clusters/{name}/nodepools/{pool}/scale",
            post(scale_node_pool),
        )
        // Add-ons
        .route(
            "/api/cluster/clusters/{name}/addons",
            get(list_addons).post(install_addon),
        )
        .route(
            "/api/cluster/clusters/{name}/addons/{addon}",
            get(get_addon).delete(uninstall_addon),
        )
        .route("/api/cluster/addons/available", get(available_addons))
        // etcd backup/restore
        .route(
            "/api/cluster/clusters/{name}/backups",
            get(list_backups).post(create_backup),
        )
        .route(
            "/api/cluster/clusters/{name}/backups/{id}/restore",
            post(restore_backup),
        )
        // Multi-tenancy
        .route(
            "/api/cluster/tenants",
            get(list_tenants).post(create_tenant),
        )
        .route(
            "/api/cluster/tenants/{id}",
            get(get_tenant).delete(delete_tenant),
        )
        .route(
            "/api/cluster/tenants/{id}/clusters/{cluster}",
            post(attach_tenant).delete(detach_tenant),
        )
        // K8s version catalog
        .route("/api/cluster/versions", get(list_versions))
        .with_state(inner)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"module": "cave-cluster", "status": "ok", "upstream": "cluster-api"}))
}

// ── Cluster CRUD ─────────────────────────────────────────────────���────────────

async fn list_clusters(State(s): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(
        s.cluster
            .store
            .list()
            .into_iter()
            .map(|c| {
                json!({
                    "name": c.spec.name, "version": c.spec.kubernetes_version,
                    "status": format!("{:?}", c.status), "region": c.spec.region,
                    "api_endpoint": c.api_endpoint, "created_at": c.created_at,
                })
            })
            .collect(),
    )
}

async fn create_cluster(
    State(s): State<AppState>,
    Json(req): Json<CreateClusterRequest>,
) -> impl IntoResponse {
    match s.cluster.store.create(req, "api") {
        Ok(c) => (
            StatusCode::CREATED,
            Json(json!({
                "name": c.spec.name, "status": format!("{:?}", c.status), "id": c.id
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_cluster(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.cluster.store.get(&name) {
        Ok(c) => Json(json!({
            "name": c.spec.name, "version": c.spec.kubernetes_version,
            "status": format!("{:?}", c.status), "api_endpoint": c.api_endpoint,
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_cluster(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.cluster.store.delete(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn upgrade_cluster(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<UpgradeClusterRequest>,
) -> impl IntoResponse {
    match s.cluster.store.upgrade(&name, &req.kubernetes_version) {
        Ok(c) => {
            Json(json!({"name": c.spec.name, "version": c.spec.kubernetes_version})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn cluster_health(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.cluster.store.get(&name) {
        Ok(cluster) => {
            let np_count = s.node_pools.list(&name).iter().map(|p| p.node_count).sum();
            let health = crate::health::check_cluster_health(&cluster, np_count);
            Json(json!({
                "cluster": name,
                "overall": format!("{:?}", health.overall),
                "components": health.components.iter().map(|c| json!({
                    "name": c.name, "status": format!("{:?}", c.status)
                })).collect::<Vec<_>>(),
                "nodes": health.node_summary,
            }))
            .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_kubeconfig(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.cluster.store.get(&name) {
        Ok(cluster) => {
            let token = crate::kubeconfig::generate_token(&cluster, "cave-admin", "kube-system");
            match generate(&cluster, CredentialType::ServiceAccountToken(token)) {
                Ok(kc) => match to_yaml(&kc) {
                    Ok(yaml) => (StatusCode::OK, [("content-type", "application/yaml")], yaml)
                        .into_response(),
                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
                },
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Node pools ─────────────────────────────���─────────────────────────────���────

async fn list_node_pools(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> Json<Vec<serde_json::Value>> {
    Json(
        s.node_pools
            .list(&name)
            .into_iter()
            .map(|p| {
                json!({
                    "name": p.name, "node_count": p.node_count,
                    "vm_size": p.vm_size, "status": format!("{:?}", p.status),
                })
            })
            .collect(),
    )
}

async fn create_node_pool(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<CreateNodePoolRequest>,
) -> impl IntoResponse {
    match s.node_pools.create(&name, req) {
        Ok(p) => (
            StatusCode::CREATED,
            Json(json!({"name": p.name, "node_count": p.node_count})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_node_pool(
    State(s): State<AppState>,
    Path((cluster, pool)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.node_pools.get(&cluster, &pool) {
        Ok(p) => Json(json!({"name": p.name, "node_count": p.node_count, "vm_size": p.vm_size}))
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_node_pool(
    State(s): State<AppState>,
    Path((cluster, pool)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.node_pools.delete(&cluster, &pool) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ScaleRequest {
    node_count: i32,
}

async fn scale_node_pool(
    State(s): State<AppState>,
    Path((cluster, pool)): Path<(String, String)>,
    Json(req): Json<ScaleRequest>,
) -> impl IntoResponse {
    match s.node_pools.scale(&cluster, &pool, req.node_count) {
        Ok(p) => Json(json!({"name": p.name, "node_count": p.node_count})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Add-ons ───────────────────────────────────────────────────────────────────

async fn list_addons(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> Json<Vec<serde_json::Value>> {
    Json(
        s.addons
            .list(&name)
            .into_iter()
            .map(|a| {
                json!({
                    "name": a.name, "version": a.current_version,
                    "status": format!("{:?}", a.status), "namespace": a.namespace,
                })
            })
            .collect(),
    )
}

#[derive(Deserialize)]
struct InstallAddonRequest {
    name: String,
    version: Option<String>,
    config: Option<HashMap<String, String>>,
}

async fn install_addon(
    State(s): State<AppState>,
    Path(cluster): Path<String>,
    Json(req): Json<InstallAddonRequest>,
) -> impl IntoResponse {
    match s.addons.install(
        &cluster,
        &req.name,
        req.version,
        req.config.unwrap_or_default(),
    ) {
        Ok(a) => (
            StatusCode::CREATED,
            Json(json!({"name": a.name, "version": a.current_version})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_addon(
    State(s): State<AppState>,
    Path((cluster, addon)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.addons.get(&cluster, &addon) {
        Ok(a) => Json(json!({"name": a.name, "version": a.current_version, "status": format!("{:?}", a.status)})).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn uninstall_addon(
    State(s): State<AppState>,
    Path((cluster, addon)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.addons.uninstall(&cluster, &addon) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn available_addons() -> Json<Vec<serde_json::Value>> {
    Json(AddonManager::list_available().into_iter().map(|a| json!({
        "name": a.name, "description": a.description, "latest_version": a.latest_version,
    })).collect())
}

// ── etcd backup/restore ───────────────────────────────────────────────────────

async fn list_backups(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> Json<Vec<serde_json::Value>> {
    Json(
        s.etcd_store
            .list_backups(&name)
            .into_iter()
            .map(|b| {
                json!({
                    "id": b.id, "status": format!("{:?}", b.status),
                    "size_bytes": b.size_bytes, "created_at": b.created_at,
                })
            })
            .collect(),
    )
}

async fn create_backup(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.cluster.store.get(&name) {
        Ok(cluster) => {
            match s
                .etcd_store
                .create_backup(&name, &cluster.spec.kubernetes_version)
            {
                Ok(b) => (
                    StatusCode::CREATED,
                    Json(json!({"id": b.id, "status": format!("{:?}", b.status)})),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response(),
            }
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn restore_backup(
    State(s): State<AppState>,
    Path((name, id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    match s.etcd_store.restore_from_backup(&name, id) {
        Ok(r) => Json(json!({"id": r.id, "status": format!("{:?}", r.status)})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Tenants ───────────────────���─────────────────────────────��─────────────────

async fn list_tenants(State(s): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(
        s.tenants
            .list()
            .into_iter()
            .map(|t| {
                json!({
                    "id": t.id, "name": t.name, "namespace": t.namespace, "clusters": t.clusters,
                })
            })
            .collect(),
    )
}

#[derive(Deserialize)]
struct CreateTenantRequest {
    id: String,
    name: String,
}

async fn create_tenant(
    State(s): State<AppState>,
    Json(req): Json<CreateTenantRequest>,
) -> impl IntoResponse {
    match s.tenants.create(req.id, req.name) {
        Ok(t) => (
            StatusCode::CREATED,
            Json(json!({"id": t.id, "namespace": t.namespace})),
        )
            .into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn get_tenant(State(s): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match s.tenants.get(&id) {
        Ok(t) => {
            Json(json!({"id": t.id, "name": t.name, "namespace": t.namespace})).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_tenant(State(s): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match s.tenants.delete(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn attach_tenant(
    State(s): State<AppState>,
    Path((tenant_id, cluster_name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.tenants.attach_to_cluster(&tenant_id, &cluster_name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn detach_tenant(
    State(s): State<AppState>,
    Path((tenant_id, cluster_name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.tenants.detach_from_cluster(&tenant_id, &cluster_name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn list_versions() -> Json<Vec<serde_json::Value>> {
    Json(
        crate::version::supported_versions()
            .into_iter()
            .map(|v| {
                json!({
                    "version": v.version, "supported": v.is_supported,
                    "latest": v.is_latest, "eol": v.end_of_life,
                })
            })
            .collect(),
    )
}
