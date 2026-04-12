//! HTTP routes for the cave-cluster module.

use crate::models::*;
use crate::ClusterState;
use crate::{health as health_mod, provisioner, tenant as tenant_mod};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<ClusterState>) -> Router {
    Router::new()
        // Templates must be registered before /:id to avoid shadowing.
        .route("/api/v1/clusters/templates", get(list_templates))
        // Cluster CRUD
        .route(
            "/api/v1/clusters",
            get(list_clusters).post(create_cluster),
        )
        .route(
            "/api/v1/clusters/:id",
            get(get_cluster).put(update_cluster).delete(delete_cluster),
        )
        // Cluster actions
        .route("/api/v1/clusters/:id/upgrade", post(trigger_upgrade))
        .route("/api/v1/clusters/:id/health", get(get_cluster_health))
        .route("/api/v1/clusters/:id/kubeconfig", get(get_kubeconfig))
        // Node pools
        .route(
            "/api/v1/clusters/:id/nodepools",
            get(list_node_pools).post(create_node_pool),
        )
        .route(
            "/api/v1/clusters/:id/nodepools/:pool_id",
            get(get_node_pool)
                .put(scale_node_pool)
                .delete(delete_node_pool),
        )
        // Add-ons
        .route(
            "/api/v1/clusters/:id/addons",
            get(list_cluster_addons).post(install_addon),
        )
        // Tenant CRUD
        .route(
            "/api/v1/tenants",
            get(list_tenants).post(create_tenant),
        )
        .route(
            "/api/v1/tenants/:id",
            get(get_tenant).delete(delete_tenant),
        )
        // Tenant views
        .route("/api/v1/tenants/:id/clusters", get(list_tenant_clusters))
        .route("/api/v1/tenants/:id/quota", get(get_tenant_quota))
        // Module health
        .route("/api/cluster/health", get(module_health))
        .with_state(state)
}

// ── Cluster ───────────────────────────────────────────────────────────────────

/// GET /api/v1/clusters — list all clusters (TODO: filter by tenant via query param)
async fn list_clusters(State(state): State<Arc<ClusterState>>) -> Json<Vec<Cluster>> {
    let store = state.store.lock().unwrap();
    Json(store.clusters.values().cloned().collect())
}

/// POST /api/v1/clusters — provision a new cluster
async fn create_cluster(
    State(state): State<Arc<ClusterState>>,
    Json(req): Json<CreateClusterRequest>,
) -> (StatusCode, Json<Cluster>) {
    let cluster = provisioner::provision_cluster(&req);
    let mut store = state.store.lock().unwrap();
    store.clusters.insert(cluster.id, cluster.clone());
    store.events.push(crate::models::ClusterEvent {
        id: Uuid::new_v4(),
        cluster_id: cluster.id,
        event_type: ClusterEventType::Created,
        message: format!("Cluster '{}' provisioning started", cluster.name),
        occurred_at: chrono::Utc::now(),
    });
    (StatusCode::CREATED, Json(cluster))
}

/// GET /api/v1/clusters/:id
async fn get_cluster(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Cluster>, StatusCode> {
    let store = state.store.lock().unwrap();
    store
        .clusters
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// PUT /api/v1/clusters/:id — update cluster metadata
async fn update_cluster(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateClusterRequest>,
) -> Result<Json<Cluster>, StatusCode> {
    let mut store = state.store.lock().unwrap();
    let cluster = store.clusters.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    if let Some(name) = req.name {
        cluster.name = name;
    }
    if let Some(policy) = req.upgrade_policy {
        cluster.upgrade_policy = policy;
    }
    cluster.updated_at = chrono::Utc::now();
    Ok(Json(cluster.clone()))
}

/// DELETE /api/v1/clusters/:id — begin graceful teardown
async fn delete_cluster(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    if store.clusters.remove(&id).is_some() {
        let event = provisioner::delete_cluster(id);
        store.events.push(event);
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── Cluster Actions ───────────────────────────────────────────────────────────

/// POST /api/v1/clusters/:id/upgrade — trigger a rolling upgrade
async fn trigger_upgrade(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpgradeClusterRequest>,
) -> Result<Json<Cluster>, StatusCode> {
    let mut store = state.store.lock().unwrap();
    let cluster = store.clusters.get(&id).cloned().ok_or(StatusCode::NOT_FOUND)?;
    let upgraded = provisioner::upgrade_cluster(&cluster, &req);
    store.clusters.insert(id, upgraded.clone());
    store.events.push(crate::models::ClusterEvent {
        id: Uuid::new_v4(),
        cluster_id: id,
        event_type: ClusterEventType::Upgraded,
        message: format!("Upgrade to {} initiated", req.target_version),
        occurred_at: chrono::Utc::now(),
    });
    Ok(Json(upgraded))
}

/// GET /api/v1/clusters/:id/health — live health check
async fn get_cluster_health(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ClusterHealth>, StatusCode> {
    let store = state.store.lock().unwrap();
    let cluster = store.clusters.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    let h = health_mod::check_cluster_health(cluster);
    let _ = health_mod::detect_unhealthy_nodes(&h);
    Ok(Json(h))
}

/// GET /api/v1/clusters/:id/kubeconfig — return kubeconfig reference (fetch from cave-vault)
async fn get_kubeconfig(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<KubeconfigRef>, StatusCode> {
    let store = state.store.lock().unwrap();
    let cluster = store.clusters.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    cluster
        .kubeconfig_ref
        .clone()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ── Node Pools ────────────────────────────────────────────────────────────────

/// GET /api/v1/clusters/:id/nodepools
async fn list_node_pools(
    State(state): State<Arc<ClusterState>>,
    Path(cluster_id): Path<Uuid>,
) -> Json<Vec<NodePool>> {
    let store = state.store.lock().unwrap();
    let pools: Vec<NodePool> = store
        .node_pools
        .values()
        .filter(|p| p.cluster_id == cluster_id)
        .cloned()
        .collect();
    Json(pools)
}

/// POST /api/v1/clusters/:id/nodepools
async fn create_node_pool(
    State(state): State<Arc<ClusterState>>,
    Path(cluster_id): Path<Uuid>,
    Json(req): Json<CreateNodePoolRequest>,
) -> (StatusCode, Json<NodePool>) {
    let pool = NodePool {
        id: Uuid::new_v4(),
        cluster_id,
        name: req.name,
        instance_type: req.instance_type,
        min_nodes: req.min_nodes,
        max_nodes: req.max_nodes,
        current_nodes: req.min_nodes,
        labels: req.labels,
        taints: req.taints,
        autoscaling_enabled: req.autoscaling_enabled,
    };
    let mut store = state.store.lock().unwrap();
    store.node_pools.insert(pool.id, pool.clone());
    (StatusCode::CREATED, Json(pool))
}

/// GET /api/v1/clusters/:id/nodepools/:pool_id
async fn get_node_pool(
    State(state): State<Arc<ClusterState>>,
    Path((_cluster_id, pool_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<NodePool>, StatusCode> {
    let store = state.store.lock().unwrap();
    store
        .node_pools
        .get(&pool_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// PUT /api/v1/clusters/:id/nodepools/:pool_id — scale a node pool
async fn scale_node_pool(
    State(state): State<Arc<ClusterState>>,
    Path((_cluster_id, pool_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ScaleNodePoolRequest>,
) -> Result<Json<NodePool>, StatusCode> {
    let mut store = state.store.lock().unwrap();
    let pool = store
        .node_pools
        .get(&pool_id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    let scaled = provisioner::scale_node_pool(&pool, &req);
    store.node_pools.insert(pool_id, scaled.clone());
    Ok(Json(scaled))
}

/// DELETE /api/v1/clusters/:id/nodepools/:pool_id
async fn delete_node_pool(
    State(state): State<Arc<ClusterState>>,
    Path((_cluster_id, pool_id)): Path<(Uuid, Uuid)>,
) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    if store.node_pools.remove(&pool_id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── Add-ons ───────────────────────────────────────────────────────────────────

/// GET /api/v1/clusters/:id/addons
async fn list_cluster_addons(
    State(state): State<Arc<ClusterState>>,
    Path(cluster_id): Path<Uuid>,
) -> Json<Vec<ClusterAddon>> {
    let store = state.store.lock().unwrap();
    let addons = store
        .addons
        .get(&cluster_id)
        .cloned()
        .unwrap_or_default();
    Json(addons)
}

/// POST /api/v1/clusters/:id/addons — install an add-on
async fn install_addon(
    State(state): State<Arc<ClusterState>>,
    Path(cluster_id): Path<Uuid>,
    Json(req): Json<InstallAddonRequest>,
) -> Result<(StatusCode, Json<ClusterAddon>), StatusCode> {
    // Verify cluster exists
    {
        let store = state.store.lock().unwrap();
        if !store.clusters.contains_key(&cluster_id) {
            return Err(StatusCode::NOT_FOUND);
        }
    }

    let addon = provisioner::install_addons(cluster_id, &req);
    let mut store = state.store.lock().unwrap();
    store
        .addons
        .entry(cluster_id)
        .or_default()
        .push(addon.clone());
    store.events.push(crate::models::ClusterEvent {
        id: Uuid::new_v4(),
        cluster_id,
        event_type: ClusterEventType::AddonInstalled,
        message: format!("Add-on {:?} v{} installation started", addon.addon_type, addon.version),
        occurred_at: chrono::Utc::now(),
    });
    Ok((StatusCode::CREATED, Json(addon)))
}

// ── Tenants ───────────────────────────────────────────────────────────────────

/// GET /api/v1/tenants
async fn list_tenants(State(state): State<Arc<ClusterState>>) -> Json<Vec<Tenant>> {
    let store = state.store.lock().unwrap();
    Json(store.tenants.values().cloned().collect())
}

/// POST /api/v1/tenants
async fn create_tenant(
    State(state): State<Arc<ClusterState>>,
    Json(req): Json<CreateTenantRequest>,
) -> (StatusCode, Json<Tenant>) {
    let t = tenant_mod::create_tenant(&req);
    let mut store = state.store.lock().unwrap();
    store.tenants.insert(t.id, t.clone());
    (StatusCode::CREATED, Json(t))
}

/// GET /api/v1/tenants/:id
async fn get_tenant(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Tenant>, StatusCode> {
    let store = state.store.lock().unwrap();
    store
        .tenants
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// DELETE /api/v1/tenants/:id
async fn delete_tenant(
    State(state): State<Arc<ClusterState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    // TODO: reject if tenant still has active clusters
    if store.tenants.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// GET /api/v1/tenants/:id/clusters — all clusters owned by a tenant
async fn list_tenant_clusters(
    State(state): State<Arc<ClusterState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<Vec<Cluster>>, StatusCode> {
    let store = state.store.lock().unwrap();
    if !store.tenants.contains_key(&tenant_id) {
        return Err(StatusCode::NOT_FOUND);
    }
    let clusters: Vec<Cluster> = store
        .clusters
        .values()
        .filter(|c| c.tenant_id == tenant_id)
        .cloned()
        .collect();
    Ok(Json(clusters))
}

/// GET /api/v1/tenants/:id/quota — quota usage snapshot
async fn get_tenant_quota(
    State(state): State<Arc<ClusterState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<QuotaUsage>, StatusCode> {
    let store = state.store.lock().unwrap();
    let tenant = store.tenants.get(&tenant_id).ok_or(StatusCode::NOT_FOUND)?;
    let clusters: Vec<Cluster> = store
        .clusters
        .values()
        .filter(|c| c.tenant_id == tenant_id)
        .cloned()
        .collect();
    let pools: Vec<NodePool> = store
        .node_pools
        .values()
        .filter(|p| clusters.iter().any(|c| c.id == p.cluster_id))
        .cloned()
        .collect();
    let usage = tenant_mod::calculate_quota_usage(tenant, &clusters, &pools);
    Ok(Json(usage))
}

// ── Templates ─────────────────────────────────────────────────────────────────

/// GET /api/v1/clusters/templates — predefined cluster shapes
async fn list_templates() -> Json<Vec<ClusterTemplate>> {
    Json(builtin_templates())
}

fn builtin_templates() -> Vec<ClusterTemplate> {
    vec![
        ClusterTemplate {
            id: Uuid::new_v4(),
            name: "small-dev".into(),
            tier: TemplateTier::Dev,
            description: "Single-node control plane, 1–3 workers. Ideal for dev/test.".into(),
            default_version: "1.31.0".into(),
            node_pools: vec![NodePoolTemplate {
                name: "workers".into(),
                instance_type: "cx21".into(),
                min_nodes: 1,
                max_nodes: 3,
                autoscaling_enabled: true,
            }],
            default_addons: vec![ClusterAddonType::CertManager, ClusterAddonType::IngressNginx],
        },
        ClusterTemplate {
            id: Uuid::new_v4(),
            name: "medium-staging".into(),
            tier: TemplateTier::Staging,
            description: "HA control plane (3 nodes), 3–10 workers. Mirrors production.".into(),
            default_version: "1.31.0".into(),
            node_pools: vec![NodePoolTemplate {
                name: "workers".into(),
                instance_type: "cx31".into(),
                min_nodes: 3,
                max_nodes: 10,
                autoscaling_enabled: true,
            }],
            default_addons: vec![
                ClusterAddonType::CertManager,
                ClusterAddonType::IngressNginx,
                ClusterAddonType::MonitoringStack,
                ClusterAddonType::CaveEbpfAgent,
            ],
        },
        ClusterTemplate {
            id: Uuid::new_v4(),
            name: "large-production".into(),
            tier: TemplateTier::Production,
            description: "HA control plane (3 nodes), 5–50 workers, full observability.".into(),
            default_version: "1.31.0".into(),
            node_pools: vec![
                NodePoolTemplate {
                    name: "system".into(),
                    instance_type: "cx41".into(),
                    min_nodes: 3,
                    max_nodes: 5,
                    autoscaling_enabled: false,
                },
                NodePoolTemplate {
                    name: "workload".into(),
                    instance_type: "cx51".into(),
                    min_nodes: 2,
                    max_nodes: 45,
                    autoscaling_enabled: true,
                },
            ],
            default_addons: vec![
                ClusterAddonType::CertManager,
                ClusterAddonType::IngressNginx,
                ClusterAddonType::MonitoringStack,
                ClusterAddonType::CaveEbpfAgent,
            ],
        },
    ]
}

// ── Module Health ─────────────────────────────────────────────────────────────

/// GET /api/cluster/health
async fn module_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-cluster",
        "status": "ok",
        "replaces": ["rancher", "gardener", "cluster-api"],
        "upstream_tracked_versions": {
            "rancher": "2.x",
            "gardener": "1.x",
            "cluster-api": "1.x"
        }
    }))
}
