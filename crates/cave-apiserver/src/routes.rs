// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! K8s-compatible API routes — full core surface.

use crate::resources::*;
use crate::store::ResourceStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

type S = State<Arc<ResourceStore>>;
type NamePath = Path<String>;
type NsNamePath = Path<(String, String)>;
type NsPath = Path<String>;

pub fn create_router(state: Arc<ResourceStore>) -> Router {
    Router::new()
        // ── Health / discovery ────────────────────────────────────────────
        .route("/api/apiserver/health", get(health))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/version", get(version))
        .route("/api", get(api_versions))
        .route("/api/v1", get(api_v1_resources))
        .route("/apis", get(api_groups))
        .route("/apis/apps/v1", get(api_apps_v1_resources))
        // ── Namespaces (core/v1 cluster-scoped) ───────────────────────────
        .route(
            "/api/v1/namespaces",
            get(list_namespaces).post(create_namespace),
        )
        .route(
            "/api/v1/namespaces/{name}",
            get(get_namespace).delete(delete_namespace),
        )
        // ── Nodes (core/v1 cluster-scoped) ────────────────────────────────
        .route("/api/v1/nodes", get(list_nodes).post(create_node))
        .route("/api/v1/nodes/{name}", get(get_node).delete(delete_node))
        // ── PersistentVolumes (core/v1 cluster-scoped) ────────────────────
        .route("/api/v1/persistentvolumes", get(list_pvs).post(create_pv))
        .route(
            "/api/v1/persistentvolumes/{name}",
            get(get_pv).delete(delete_pv),
        )
        // ── Pods ──────────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/pods",
            get(list_pods).post(create_pod),
        )
        .route(
            "/api/v1/namespaces/{ns}/pods/{name}",
            get(get_pod).delete(delete_pod),
        )
        .route(
            "/api/v1/namespaces/{ns}/pods/{name}/status",
            get(get_pod_status).put(put_pod_status),
        )
        // ── Services ──────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/services",
            get(list_services).post(create_service),
        )
        .route(
            "/api/v1/namespaces/{ns}/services/{name}",
            get(get_service).delete(delete_service),
        )
        // ── ConfigMaps ────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/configmaps",
            get(list_configmaps).post(create_configmap),
        )
        .route(
            "/api/v1/namespaces/{ns}/configmaps/{name}",
            get(get_configmap).delete(delete_configmap),
        )
        // ── Secrets ───────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/secrets",
            get(list_secrets).post(create_secret),
        )
        .route(
            "/api/v1/namespaces/{ns}/secrets/{name}",
            get(get_secret).delete(delete_secret),
        )
        // ── ServiceAccounts ───────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/serviceaccounts",
            get(list_serviceaccounts).post(create_serviceaccount),
        )
        .route(
            "/api/v1/namespaces/{ns}/serviceaccounts/{name}",
            get(get_serviceaccount).delete(delete_serviceaccount),
        )
        // ── Events ────────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/events",
            get(list_events).post(create_event),
        )
        .route(
            "/api/v1/namespaces/{ns}/events/{name}",
            get(get_event).delete(delete_event),
        )
        // ── Endpoints ─────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/endpoints",
            get(list_endpoints).post(create_endpoints),
        )
        .route(
            "/api/v1/namespaces/{ns}/endpoints/{name}",
            get(get_endpoints).delete(delete_endpoints),
        )
        // ── PersistentVolumeClaims ────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/persistentvolumeclaims",
            get(list_pvcs).post(create_pvc),
        )
        .route(
            "/api/v1/namespaces/{ns}/persistentvolumeclaims/{name}",
            get(get_pvc).delete(delete_pvc),
        )
        // ── ResourceQuotas ────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/resourcequotas",
            get(list_resourcequotas).post(create_resourcequota),
        )
        .route(
            "/api/v1/namespaces/{ns}/resourcequotas/{name}",
            get(get_resourcequota).delete(delete_resourcequota),
        )
        // ── LimitRanges ───────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/{ns}/limitranges",
            get(list_limitranges).post(create_limitrange),
        )
        .route(
            "/api/v1/namespaces/{ns}/limitranges/{name}",
            get(get_limitrange).delete(delete_limitrange),
        )
        // ── Deployments (apps/v1) ─────────────────────────────────────────
        .route(
            "/apis/apps/v1/namespaces/{ns}/deployments",
            get(list_deployments).post(create_deployment),
        )
        .route(
            "/apis/apps/v1/namespaces/{ns}/deployments/{name}",
            get(get_deployment).delete(delete_deployment),
        )
        .route(
            "/apis/apps/v1/namespaces/{ns}/deployments/{name}/status",
            get(get_deployment_status).put(put_deployment_status),
        )
        .route(
            "/apis/apps/v1/namespaces/{ns}/deployments/{name}/scale",
            get(get_deployment_scale).put(put_deployment_scale),
        )
        // ── StatefulSets (apps/v1) ────────────────────────────────────────
        .route(
            "/apis/apps/v1/namespaces/{ns}/statefulsets",
            get(list_statefulsets).post(create_statefulset),
        )
        .route(
            "/apis/apps/v1/namespaces/{ns}/statefulsets/{name}",
            get(get_statefulset).delete(delete_statefulset),
        )
        // ── DaemonSets (apps/v1) ──────────────────────────────────────────
        .route(
            "/apis/apps/v1/namespaces/{ns}/daemonsets",
            get(list_daemonsets).post(create_daemonset),
        )
        .route(
            "/apis/apps/v1/namespaces/{ns}/daemonsets/{name}",
            get(get_daemonset).delete(delete_daemonset),
        )
        // ── ReplicaSets (apps/v1) ─────────────────────────────────────────
        .route(
            "/apis/apps/v1/namespaces/{ns}/replicasets",
            get(list_replicasets).post(create_replicaset),
        )
        .route(
            "/apis/apps/v1/namespaces/{ns}/replicasets/{name}",
            get(get_replicaset).delete(delete_replicaset),
        )
        // ── Jobs (batch/v1) ───────────────────────────────────────────────
        .route(
            "/apis/batch/v1/namespaces/{ns}/jobs",
            get(list_jobs).post(create_job),
        )
        .route(
            "/apis/batch/v1/namespaces/{ns}/jobs/{name}",
            get(get_job).delete(delete_job),
        )
        // ── CronJobs (batch/v1) ───────────────────────────────────────────
        .route(
            "/apis/batch/v1/namespaces/{ns}/cronjobs",
            get(list_cronjobs).post(create_cronjob),
        )
        .route(
            "/apis/batch/v1/namespaces/{ns}/cronjobs/{name}",
            get(get_cronjob).delete(delete_cronjob),
        )
        // ── Ingresses (networking.k8s.io/v1) ─────────────────────────────
        .route(
            "/apis/networking.k8s.io/v1/namespaces/{ns}/ingresses",
            get(list_ingresses).post(create_ingress),
        )
        .route(
            "/apis/networking.k8s.io/v1/namespaces/{ns}/ingresses/{name}",
            get(get_ingress).delete(delete_ingress),
        )
        // ── NetworkPolicies (networking.k8s.io/v1) ────────────────────────
        .route(
            "/apis/networking.k8s.io/v1/namespaces/{ns}/networkpolicies",
            get(list_networkpolicies).post(create_networkpolicy),
        )
        .route(
            "/apis/networking.k8s.io/v1/namespaces/{ns}/networkpolicies/{name}",
            get(get_networkpolicy).delete(delete_networkpolicy),
        )
        // ── StorageClasses (storage.k8s.io/v1) ───────────────────────────
        .route(
            "/apis/storage.k8s.io/v1/storageclasses",
            get(list_storageclasses).post(create_storageclass),
        )
        .route(
            "/apis/storage.k8s.io/v1/storageclasses/{name}",
            get(get_storageclass).delete(delete_storageclass),
        )
        // ── Roles (rbac.authorization.k8s.io/v1) ─────────────────────────
        .route(
            "/apis/rbac.authorization.k8s.io/v1/namespaces/{ns}/roles",
            get(list_roles).post(create_role),
        )
        .route(
            "/apis/rbac.authorization.k8s.io/v1/namespaces/{ns}/roles/{name}",
            get(get_role).delete(delete_role),
        )
        // ── ClusterRoles (rbac.authorization.k8s.io/v1) ───────────────────
        .route(
            "/apis/rbac.authorization.k8s.io/v1/clusterroles",
            get(list_clusterroles).post(create_clusterrole),
        )
        .route(
            "/apis/rbac.authorization.k8s.io/v1/clusterroles/{name}",
            get(get_clusterrole).delete(delete_clusterrole),
        )
        // ── RoleBindings (rbac.authorization.k8s.io/v1) ───────────────────
        .route(
            "/apis/rbac.authorization.k8s.io/v1/namespaces/{ns}/rolebindings",
            get(list_rolebindings).post(create_rolebinding),
        )
        .route(
            "/apis/rbac.authorization.k8s.io/v1/namespaces/{ns}/rolebindings/{name}",
            get(get_rolebinding).delete(delete_rolebinding),
        )
        // ── ClusterRoleBindings (rbac.authorization.k8s.io/v1) ────────────
        .route(
            "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings",
            get(list_clusterrolebindings).post(create_clusterrolebinding),
        )
        .route(
            "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings/{name}",
            get(get_clusterrolebinding).delete(delete_clusterrolebinding),
        )
        // Parity
        .route("/api/apiserver/parity", get(parity))
        .with_state(state)
}

// ── Parity ────────────────────────────────────────────────────────────────────

async fn parity() -> Json<serde_json::Value> {
    match crate::calculate_parity() {
        Ok(report) => Json(serde_json::to_value(&report).unwrap_or_default()),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// ── Discovery ─────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"module":"cave-apiserver","status":"ok","upstream":"kube-apiserver"}))
}

async fn healthz() -> &'static str {
    "ok"
}
async fn readyz() -> &'static str {
    "ok"
}

async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "major": "1", "minor": "29",
        "gitVersion": "v1.29.0",
        "platform": "linux/amd64",
        "goVersion": "go1.21.5"
    }))
}

async fn api_versions() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "kind": "APIVersions",
        "apiVersion": "v1",
        "versions": ["v1"],
        "serverAddressByClientCIDRs": [{"clientCIDR":"0.0.0.0/0","serverAddress":""}]
    }))
}

async fn api_v1_resources() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "kind": "APIResourceList",
        "apiVersion": "v1",
        "groupVersion": "v1",
        "resources": [
            {"name":"namespaces","singularName":"namespace","namespaced":false,"kind":"Namespace","verbs":["create","delete","get","list"]},
            {"name":"nodes","singularName":"node","namespaced":false,"kind":"Node","verbs":["create","delete","get","list"]},
            {"name":"persistentvolumes","singularName":"persistentvolume","namespaced":false,"kind":"PersistentVolume","verbs":["create","delete","get","list"]},
            {"name":"pods","singularName":"pod","namespaced":true,"kind":"Pod","verbs":["create","delete","get","list"]},
            {"name":"services","singularName":"service","namespaced":true,"kind":"Service","verbs":["create","delete","get","list"]},
            {"name":"configmaps","singularName":"configmap","namespaced":true,"kind":"ConfigMap","verbs":["create","delete","get","list"]},
            {"name":"secrets","singularName":"secret","namespaced":true,"kind":"Secret","verbs":["create","delete","get","list"]},
            {"name":"serviceaccounts","singularName":"serviceaccount","namespaced":true,"kind":"ServiceAccount","verbs":["create","delete","get","list"]},
            {"name":"events","singularName":"event","namespaced":true,"kind":"Event","verbs":["create","delete","get","list"]},
            {"name":"endpoints","singularName":"endpoints","namespaced":true,"kind":"Endpoints","verbs":["create","delete","get","list"]},
            {"name":"persistentvolumeclaims","singularName":"persistentvolumeclaim","namespaced":true,"kind":"PersistentVolumeClaim","verbs":["create","delete","get","list"]},
            {"name":"resourcequotas","singularName":"resourcequota","namespaced":true,"kind":"ResourceQuota","verbs":["create","delete","get","list"]},
            {"name":"limitranges","singularName":"limitrange","namespaced":true,"kind":"LimitRange","verbs":["create","delete","get","list"]}
        ]
    }))
}

async fn api_groups() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "kind": "APIGroupList",
        "apiVersion": "v1",
        "groups": [
            {"name":"apps","versions":[{"groupVersion":"apps/v1","version":"v1"}],"preferredVersion":{"groupVersion":"apps/v1","version":"v1"}},
            {"name":"batch","versions":[{"groupVersion":"batch/v1","version":"v1"}],"preferredVersion":{"groupVersion":"batch/v1","version":"v1"}},
            {"name":"networking.k8s.io","versions":[{"groupVersion":"networking.k8s.io/v1","version":"v1"}],"preferredVersion":{"groupVersion":"networking.k8s.io/v1","version":"v1"}},
            {"name":"storage.k8s.io","versions":[{"groupVersion":"storage.k8s.io/v1","version":"v1"}],"preferredVersion":{"groupVersion":"storage.k8s.io/v1","version":"v1"}},
            {"name":"rbac.authorization.k8s.io","versions":[{"groupVersion":"rbac.authorization.k8s.io/v1","version":"v1"}],"preferredVersion":{"groupVersion":"rbac.authorization.k8s.io/v1","version":"v1"}}
        ]
    }))
}

async fn api_apps_v1_resources() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "kind": "APIResourceList",
        "apiVersion": "v1",
        "groupVersion": "apps/v1",
        "resources": [
            {"name":"deployments","singularName":"deployment","namespaced":true,"kind":"Deployment","verbs":["create","delete","get","list"]},
            {"name":"statefulsets","singularName":"statefulset","namespaced":true,"kind":"StatefulSet","verbs":["create","delete","get","list"]},
            {"name":"daemonsets","singularName":"daemonset","namespaced":true,"kind":"DaemonSet","verbs":["create","delete","get","list"]},
            {"name":"replicasets","singularName":"replicaset","namespaced":true,"kind":"ReplicaSet","verbs":["create","delete","get","list"]}
        ]
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn not_found(e: crate::error::ApiError) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, e.to_string())
}
fn conflict(e: crate::error::ApiError) -> (StatusCode, String) {
    (StatusCode::CONFLICT, e.to_string())
}

// ── Namespace ─────────────────────────────────────────────────────────────────

async fn list_namespaces(State(s): S) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"NamespaceList","items":s.list("Namespace", "")}))
}
async fn create_namespace(
    State(s): S,
    Json(r): Json<Namespace>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    s.create(Resource::Namespace(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_namespace(
    State(s): S,
    Path(name): NamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Namespace", "", &name).map(Json).map_err(not_found)
}
async fn delete_namespace(
    State(s): S,
    Path(name): NamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Namespace", "", &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Node ──────────────────────────────────────────────────────────────────────

async fn list_nodes(State(s): S) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"NodeList","items":s.list("Node", "")}))
}
async fn create_node(
    State(s): S,
    Json(r): Json<Node>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    s.create(Resource::Node(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_node(
    State(s): S,
    Path(name): NamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Node", "", &name).map(Json).map_err(not_found)
}
async fn delete_node(
    State(s): S,
    Path(name): NamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Node", "", &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── PersistentVolume ──────────────────────────────────────────────────────────

async fn list_pvs(State(s): S) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"PersistentVolumeList","items":s.list("PersistentVolume", "")}))
}
async fn create_pv(
    State(s): S,
    Json(r): Json<PersistentVolume>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    s.create(Resource::PersistentVolume(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_pv(State(s): S, Path(name): NamePath) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("PersistentVolume", "", &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_pv(State(s): S, Path(name): NamePath) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("PersistentVolume", "", &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Pod ───────────────────────────────────────────────────────────────────────

async fn list_pods(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"PodList","items":s.list("Pod", &ns)}))
}
async fn create_pod(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Pod>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Pod(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_pod(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Pod", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_pod(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Pod", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}
async fn get_pod_status(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Pod", &ns, &name).map(Json).map_err(not_found)
}
async fn put_pod_status(
    State(s): S,
    Path((ns, name)): NsNamePath,
    Json(mut r): Json<Pod>,
) -> Result<Json<Resource>, (StatusCode, String)> {
    r.metadata.namespace = ns;
    r.metadata.name = name;
    s.update(Resource::Pod(r)).map(Json).map_err(not_found)
}

// ── Service ───────────────────────────────────────────────────────────────────

async fn list_services(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ServiceList","items":s.list("Service", &ns)}))
}
async fn create_service(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Service>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Service(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_service(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Service", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_service(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Service", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── ConfigMap ─────────────────────────────────────────────────────────────────

async fn list_configmaps(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ConfigMapList","items":s.list("ConfigMap", &ns)}))
}
async fn create_configmap(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<ConfigMap>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::ConfigMap(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_configmap(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("ConfigMap", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_configmap(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("ConfigMap", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Secret ────────────────────────────────────────────────────────────────────

async fn list_secrets(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"SecretList","items":s.list("Secret", &ns)}))
}
async fn create_secret(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Secret>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Secret(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_secret(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Secret", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_secret(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Secret", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── ServiceAccount ────────────────────────────────────────────────────────────

async fn list_serviceaccounts(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ServiceAccountList","items":s.list("ServiceAccount", &ns)}))
}
async fn create_serviceaccount(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<ServiceAccount>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::ServiceAccount(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_serviceaccount(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("ServiceAccount", &ns, &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_serviceaccount(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("ServiceAccount", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Event ─────────────────────────────────────────────────────────────────────

async fn list_events(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"EventList","items":s.list("KubeEvent", &ns)}))
}
async fn create_event(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<KubeEvent>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::KubeEvent(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_event(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("KubeEvent", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_event(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("KubeEvent", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Endpoints ─────────────────────────────────────────────────────────────────

async fn list_endpoints(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"EndpointsList","items":s.list("Endpoints", &ns)}))
}
async fn create_endpoints(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Endpoints>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Endpoints(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_endpoints(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Endpoints", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_endpoints(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Endpoints", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── PersistentVolumeClaim ─────────────────────────────────────────────────────

async fn list_pvcs(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"kind":"PersistentVolumeClaimList","items":s.list("PersistentVolumeClaim", &ns)}),
    )
}
async fn create_pvc(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<PersistentVolumeClaim>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::PersistentVolumeClaim(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_pvc(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("PersistentVolumeClaim", &ns, &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_pvc(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("PersistentVolumeClaim", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── ResourceQuota ─────────────────────────────────────────────────────────────

async fn list_resourcequotas(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ResourceQuotaList","items":s.list("ResourceQuota", &ns)}))
}
async fn create_resourcequota(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<ResourceQuota>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::ResourceQuota(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_resourcequota(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("ResourceQuota", &ns, &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_resourcequota(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("ResourceQuota", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── LimitRange ────────────────────────────────────────────────────────────────

async fn list_limitranges(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"LimitRangeList","items":s.list("LimitRange", &ns)}))
}
async fn create_limitrange(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<LimitRange>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::LimitRange(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_limitrange(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("LimitRange", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_limitrange(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("LimitRange", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Deployment ────────────────────────────────────────────────────────────────

async fn list_deployments(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"DeploymentList","items":s.list("Deployment", &ns)}))
}
async fn create_deployment(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Deployment>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Deployment(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_deployment(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Deployment", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_deployment(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Deployment", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}
async fn get_deployment_status(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Deployment", &ns, &name).map(Json).map_err(not_found)
}
async fn put_deployment_status(
    State(s): S,
    Path((ns, name)): NsNamePath,
    Json(mut r): Json<Deployment>,
) -> Result<Json<Resource>, (StatusCode, String)> {
    r.metadata.namespace = ns;
    r.metadata.name = name;
    s.update(Resource::Deployment(r))
        .map(Json)
        .map_err(not_found)
}
async fn get_deployment_scale(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Scale>, (StatusCode, String)> {
    let resource = s.get("Deployment", &ns, &name).map_err(not_found)?;
    let replicas = if let Resource::Deployment(ref d) = resource {
        d.spec.replicas
    } else {
        0
    };
    Ok(Json(Scale {
        api_version: "autoscaling/v1".into(),
        kind: "Scale".into(),
        metadata: resource.metadata().clone(),
        spec: ScaleSpec { replicas },
        status: ScaleStatus {
            replicas,
            selector: None,
        },
    }))
}
async fn put_deployment_scale(
    State(s): S,
    Path((ns, name)): NsNamePath,
    Json(scale): Json<Scale>,
) -> Result<Json<Scale>, (StatusCode, String)> {
    let existing = s.get("Deployment", &ns, &name).map_err(not_found)?;
    if let Resource::Deployment(mut d) = existing {
        d.spec.replicas = scale.spec.replicas;
        s.update(Resource::Deployment(d)).map_err(not_found)?;
    }
    Ok(Json(scale))
}

// ── StatefulSet ───────────────────────────────────────────────────────────────

async fn list_statefulsets(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"StatefulSetList","items":s.list("StatefulSet", &ns)}))
}
async fn create_statefulset(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<StatefulSet>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::StatefulSet(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_statefulset(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("StatefulSet", &ns, &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_statefulset(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("StatefulSet", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── DaemonSet ─────────────────────────────────────────────────────────────────

async fn list_daemonsets(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"DaemonSetList","items":s.list("DaemonSet", &ns)}))
}
async fn create_daemonset(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<DaemonSet>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::DaemonSet(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_daemonset(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("DaemonSet", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_daemonset(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("DaemonSet", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── ReplicaSet ────────────────────────────────────────────────────────────────

async fn list_replicasets(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ReplicaSetList","items":s.list("ReplicaSet", &ns)}))
}
async fn create_replicaset(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<ReplicaSet>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::ReplicaSet(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_replicaset(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("ReplicaSet", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_replicaset(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("ReplicaSet", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Job ───────────────────────────────────────────────────────────────────────

async fn list_jobs(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"JobList","items":s.list("Job", &ns)}))
}
async fn create_job(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Job>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Job(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_job(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Job", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_job(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Job", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── CronJob ───────────────────────────────────────────────────────────────────

async fn list_cronjobs(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"CronJobList","items":s.list("CronJob", &ns)}))
}
async fn create_cronjob(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<CronJob>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::CronJob(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_cronjob(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("CronJob", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_cronjob(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("CronJob", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Ingress ───────────────────────────────────────────────────────────────────

async fn list_ingresses(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"IngressList","items":s.list("Ingress", &ns)}))
}
async fn create_ingress(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Ingress>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Ingress(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_ingress(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Ingress", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_ingress(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Ingress", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── NetworkPolicy ─────────────────────────────────────────────────────────────

async fn list_networkpolicies(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"NetworkPolicyList","items":s.list("NetworkPolicy", &ns)}))
}
async fn create_networkpolicy(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<NetworkPolicy>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::NetworkPolicy(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_networkpolicy(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("NetworkPolicy", &ns, &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_networkpolicy(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("NetworkPolicy", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── StorageClass ──────────────────────────────────────────────────────────────

async fn list_storageclasses(State(s): S) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"StorageClassList","items":s.list("StorageClass", "")}))
}
async fn create_storageclass(
    State(s): S,
    Json(r): Json<StorageClass>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    s.create(Resource::StorageClass(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_storageclass(
    State(s): S,
    Path(name): NamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("StorageClass", "", &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_storageclass(
    State(s): S,
    Path(name): NamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("StorageClass", "", &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── Role ──────────────────────────────────────────────────────────────────────

async fn list_roles(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"RoleList","items":s.list("Role", &ns)}))
}
async fn create_role(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<Role>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::Role(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_role(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Role", &ns, &name).map(Json).map_err(not_found)
}
async fn delete_role(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Role", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── ClusterRole ───────────────────────────────────────────────────────────────

async fn list_clusterroles(State(s): S) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ClusterRoleList","items":s.list("ClusterRole", "")}))
}
async fn create_clusterrole(
    State(s): S,
    Json(r): Json<ClusterRole>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    s.create(Resource::ClusterRole(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_clusterrole(
    State(s): S,
    Path(name): NamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("ClusterRole", "", &name).map(Json).map_err(not_found)
}
async fn delete_clusterrole(
    State(s): S,
    Path(name): NamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("ClusterRole", "", &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── RoleBinding ───────────────────────────────────────────────────────────────

async fn list_rolebindings(State(s): S, Path(ns): NsPath) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"RoleBindingList","items":s.list("RoleBinding", &ns)}))
}
async fn create_rolebinding(
    State(s): S,
    Path(ns): NsPath,
    Json(mut r): Json<RoleBinding>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    r.metadata.namespace = ns;
    s.create(Resource::RoleBinding(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_rolebinding(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("RoleBinding", &ns, &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_rolebinding(
    State(s): S,
    Path((ns, name)): NsNamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("RoleBinding", &ns, &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}

// ── ClusterRoleBinding ────────────────────────────────────────────────────────

async fn list_clusterrolebindings(State(s): S) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"kind":"ClusterRoleBindingList","items":s.list("ClusterRoleBinding", "")}),
    )
}
async fn create_clusterrolebinding(
    State(s): S,
    Json(r): Json<ClusterRoleBinding>,
) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    s.create(Resource::ClusterRoleBinding(r))
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(conflict)
}
async fn get_clusterrolebinding(
    State(s): S,
    Path(name): NamePath,
) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("ClusterRoleBinding", "", &name)
        .map(Json)
        .map_err(not_found)
}
async fn delete_clusterrolebinding(
    State(s): S,
    Path(name): NamePath,
) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("ClusterRoleBinding", "", &name)
        .map(|_| StatusCode::OK)
        .map_err(not_found)
}
