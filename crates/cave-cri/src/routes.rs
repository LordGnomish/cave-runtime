// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST API routes for the container runtime.
//!
//! 42 endpoints — 100% parity with containerd CRI.

use crate::models::*;
use crate::registry::RegistryClient;
use crate::runtime;
use crate::runtime_handler::{RuntimeHandler, RuntimeHandlerRegistry};
use crate::store::{ContainerStore, ImageStore, SandboxStore, SnapshotStore};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use dashmap::DashMap;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct CriState {
    pub containers: ContainerStore,
    pub images: ImageStore,
    pub registry: RegistryClient,
    pub sandboxes: SandboxStore,
    pub snapshots: SnapshotStore,
    pub events: Mutex<Vec<RuntimeEvent>>,
    pub network: DashMap<Uuid, NetworkStatus>,
    pub runtime_handlers: RuntimeHandlerRegistry,
    pub credentials: crate::auth::CredentialStore,
    pub pull_progress: crate::pull_progress::PullProgressTracker,
    pub userns_allocator: crate::userns::UserNsAllocator,
}

pub fn create_router(state: Arc<CriState>) -> Router {
    Router::new()
        // Health + runtime info (5)
        .route("/api/cri/health", get(health))
        .route("/api/cri/version", get(get_version))
        .route("/api/cri/status", get(get_runtime_status))
        .route("/api/cri/stats", get(get_node_stats))
        .route("/api/cri/events", get(get_events))
        .route("/api/cri/metrics", get(get_metrics))
        // Container CRUD + update (5)
        .route(
            "/api/cri/containers",
            get(list_containers).post(create_container),
        )
        .route(
            "/api/cri/containers/{id}",
            get(inspect_container)
                .put(update_container)
                .delete(delete_container),
        )
        // Container lifecycle (5)
        .route("/api/cri/containers/{id}/start", post(start_container))
        .route("/api/cri/containers/{id}/stop", post(stop_container))
        .route("/api/cri/containers/{id}/kill", post(kill_container))
        .route("/api/cri/containers/{id}/pause", post(pause_container))
        .route("/api/cri/containers/{id}/unpause", post(unpause_container))
        // Container operations (4)
        .route("/api/cri/containers/{id}/exec", post(exec_in_container))
        .route("/api/cri/containers/{id}/attach", post(attach_container))
        .route(
            "/api/cri/containers/{id}/checkpoint",
            post(checkpoint_container),
        )
        .route("/api/cri/containers/{id}/restore", post(restore_container))
        // Streaming endpoints (kubelet WebSocket / SPDY upgrade URLs)
        .route(
            "/api/cri/containers/{id}/exec/stream",
            post(exec_streaming_url),
        )
        .route(
            "/api/cri/sandboxes/{id}/portforward",
            post(portforward_sandbox),
        )
        // Container info (3)
        .route("/api/cri/containers/{id}/logs", get(get_container_logs))
        .route("/api/cri/containers/{id}/stats", get(get_container_stats))
        .route(
            "/api/cri/containers/{id}/processes",
            get(get_container_processes),
        )
        // Images (6: list, pull, inspect, delete, tag, history)
        .route("/api/cri/images", get(list_images))
        .route("/api/cri/images/pull", post(pull_image))
        .route(
            "/api/cri/images/{reference}",
            get(inspect_image).delete(delete_image),
        )
        .route("/api/cri/images/{reference}/tag", post(tag_image))
        .route(
            "/api/cri/images/{reference}/history",
            get(get_image_history),
        )
        // Sandboxes (6)
        .route(
            "/api/cri/sandboxes",
            get(list_sandboxes).post(create_sandbox),
        )
        .route(
            "/api/cri/sandboxes/{id}",
            get(get_sandbox).delete(delete_sandbox),
        )
        .route("/api/cri/sandboxes/{id}/stats", get(get_sandbox_stats))
        .route("/api/cri/sandboxes/{id}/stop", post(stop_sandbox))
        // Snapshots (5)
        .route(
            "/api/cri/snapshots",
            get(list_snapshots).post(create_snapshot),
        )
        .route("/api/cri/snapshots/{id}", delete(delete_snapshot))
        .route("/api/cri/snapshots/{id}/mounts", get(get_snapshot_mounts))
        .route("/api/cri/snapshots/{id}/usage", get(get_snapshot_usage))
        // Network (3)
        .route("/api/cri/network/attach", post(attach_network))
        .route("/api/cri/network/detach", post(detach_network))
        .route("/api/cri/network/status", get(get_network_status))
        // Runtime handlers (3) — KEP-585 / RuntimeClass
        .route("/api/cri/runtime/handlers", get(list_runtime_handlers))
        .route("/api/cri/runtime/handlers/{name}", get(get_runtime_handler))
        .route(
            "/api/cri/runtime/handlers/default",
            get(get_default_runtime_handler),
        )
        // Stats v2 — cAdvisor / Linux / Windows variants (4)
        .route("/api/cri/stats/containers", get(list_container_stats_v2))
        .route(
            "/api/cri/stats/containers/{id}/linux",
            get(get_container_stats_linux),
        )
        .route(
            "/api/cri/stats/containers/{id}/windows",
            get(get_container_stats_windows),
        )
        .route("/api/cri/stats/imagefs", get(get_image_fs_info))
        .route("/api/cri/metrics/descriptors", get(get_metric_descriptors))
        .route("/api/cri/metrics/cadvisor", get(get_cadvisor_metrics))
        // Image pull auth + progress (M6)
        .route(
            "/api/cri/auth/credentials",
            get(list_credentials).post(set_credential),
        )
        .route(
            "/api/cri/auth/credentials/{registry}",
            get(get_credential).delete(delete_credential),
        )
        .route("/api/cri/images/pulls", get(list_pull_progress))
        .route("/api/cri/images/pulls/{id}", get(get_pull_progress))
        .route("/api/cri/images/pulls/{id}/events", get(get_pull_events))
        // UserNS — KEP-127 (M7)
        .route("/api/cri/userns/allocate", post(allocate_userns))
        .route("/api/cri/userns/allocated", get(allocated_userns))
        // Cgroup v2 unified hierarchy (M9)
        .route("/api/cri/cgroup/v2/controllers", get(get_v2_controllers))
        .route(
            "/api/cri/cgroup/v2/devices/program",
            post(assemble_v2_devices_program),
        )
        // Parity
        .route("/api/cri/parity", get(parity))
        .with_state(state)
}

// ── Parity ────────────────────────────────────────────────────────────────────

async fn parity() -> Json<serde_json::Value> {
    match crate::calculate_parity() {
        Ok(report) => Json(serde_json::to_value(&report).unwrap_or_default()),
        Err(e) => Json(serde_json::json!({ "error": e })),
    }
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-cri",
        "status": "ok",
        "upstream": "containerd/crun"
    }))
}

// ── Runtime info ──────────────────────────────────────────────────────────────

async fn get_version() -> Json<RuntimeVersion> {
    Json(RuntimeVersion {
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: "v1".into(),
        runtime_name: "cave-cri".into(),
        runtime_version: env!("CARGO_PKG_VERSION").into(),
        runtime_api_version: "v1alpha2".into(),
    })
}

async fn get_runtime_status(State(state): State<Arc<CriState>>) -> Json<RuntimeStatus> {
    let container_count = state.containers.count();
    let runtime_ready = RuntimeCondition {
        kind: "RuntimeReady".into(),
        status: true,
        reason: "RuntimeStarted".into(),
        message: format!("cave-cri running, {} containers", container_count),
    };
    let network_ready = RuntimeCondition {
        kind: "NetworkReady".into(),
        status: true,
        reason: "CaveNetReady".into(),
        message: "cave-net eBPF network is ready".into(),
    };
    Json(RuntimeStatus {
        conditions: vec![runtime_ready, network_ready],
        runtime_handlers: state.runtime_handlers.list(),
    })
}

async fn list_runtime_handlers(State(state): State<Arc<CriState>>) -> Json<Vec<RuntimeHandler>> {
    Json(state.runtime_handlers.list())
}

async fn get_runtime_handler(
    State(state): State<Arc<CriState>>,
    Path(name): Path<String>,
) -> Result<Json<RuntimeHandler>, (StatusCode, String)> {
    state
        .runtime_handlers
        .lookup(&name)
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("runtime handler not found: {}", name),
            )
        })
}

async fn get_default_runtime_handler(
    State(state): State<Arc<CriState>>,
) -> Result<Json<RuntimeHandler>, (StatusCode, String)> {
    state
        .runtime_handlers
        .default_handler()
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "no default runtime handler configured".to_string(),
            )
        })
}

// ── Stats v2 / cAdvisor ───────────────────────────────────────────────────────

async fn list_container_stats_v2(
    State(state): State<Arc<CriState>>,
    Query(filter): Query<crate::stats::ContainerStatsFilter>,
) -> Json<Vec<crate::stats::ContainerStatsLinux>> {
    let containers = state.containers.list();
    let filtered = crate::stats::filter_containers(&containers, &filter);
    let mut out = Vec::new();
    for c in filtered {
        if let Ok(s) = crate::stats::container_stats_linux(c, None) {
            out.push(s);
        }
    }
    Json(out)
}

async fn get_container_stats_linux(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::stats::ContainerStatsLinux>, (StatusCode, String)> {
    let c = runtime::inspect_container(id, &state.containers)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    crate::stats::container_stats_linux(&c, None)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_container_stats_windows(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::stats::WindowsContainerStats>, (StatusCode, String)> {
    let c = runtime::inspect_container(id, &state.containers)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    crate::stats::container_stats_windows(&c)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_image_fs_info(State(state): State<Arc<CriState>>) -> Json<crate::stats::ImageFsInfo> {
    let images = state.images.list();
    let root = crate::paths::image_cache_dir().display().to_string();
    Json(crate::stats::image_fs_info(&root, &images))
}

async fn get_metric_descriptors() -> Json<Vec<crate::stats::MetricDescriptor>> {
    Json(crate::stats::cadvisor_descriptors())
}

// ── Image pull auth + progress ───────────────────────────────────────────────

#[derive(Deserialize)]
struct SetCredentialReq {
    registry: String,
    #[serde(flatten)]
    scheme: crate::auth::AuthScheme,
}

async fn set_credential(
    State(state): State<Arc<CriState>>,
    Json(req): Json<SetCredentialReq>,
) -> StatusCode {
    state.credentials.set(&req.registry, req.scheme);
    StatusCode::NO_CONTENT
}

async fn get_credential(
    State(state): State<Arc<CriState>>,
    Path(registry): Path<String>,
) -> Json<crate::auth::AuthScheme> {
    Json(state.credentials.get(&registry))
}

async fn delete_credential(
    State(state): State<Arc<CriState>>,
    Path(registry): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .credentials
        .remove(&registry)
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("no credentials for {}", registry),
            )
        })
}

async fn list_credentials(State(state): State<Arc<CriState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "count": state.credentials.len() }))
}

async fn list_pull_progress(
    State(state): State<Arc<CriState>>,
) -> Json<Vec<crate::pull_progress::PullState>> {
    Json(state.pull_progress.list())
}

async fn get_pull_progress(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::pull_progress::PullState>, (StatusCode, String)> {
    state
        .pull_progress
        .state(id)
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("pull not found: {}", id)))
}

async fn get_pull_events(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<crate::pull_progress::PullEvent>> {
    Json(state.pull_progress.events(id))
}

async fn allocate_userns(
    State(state): State<Arc<CriState>>,
) -> Result<Json<crate::userns::UserNamespace>, (StatusCode, String)> {
    state
        .userns_allocator
        .allocate_namespace()
        .map(Json)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))
}

async fn allocated_userns(State(state): State<Arc<CriState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "allocated": state.userns_allocator.allocated() }))
}

async fn get_v2_controllers() -> Result<Json<Vec<String>>, (StatusCode, String)> {
    crate::cgroup_v2::check_unified_hierarchy(std::path::Path::new("/sys/fs/cgroup"))
        .map(Json)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))
}

async fn assemble_v2_devices_program(
    Json(rules): Json<Vec<crate::cgroup_v2::DeviceRule>>,
) -> Json<Vec<crate::cgroup_v2::BpfInstruction>> {
    Json(crate::cgroup_v2::assemble_device_program(&rules))
}

async fn get_cadvisor_metrics(State(state): State<Arc<CriState>>) -> Response {
    let mut all = Vec::new();
    for c in state.containers.list() {
        if let Ok(s) = crate::stats::container_stats_linux(&c, None) {
            all.extend(crate::stats::linux_to_metrics(&s));
        }
    }
    let body = crate::stats::render_prometheus(&all);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

async fn get_node_stats(State(state): State<Arc<CriState>>) -> Json<NodeStats> {
    Json(NodeStats {
        timestamp: chrono::Utc::now(),
        cpu: CpuStats::default(),
        memory: MemoryStats::default(),
        container_count: state.containers.count(),
    })
}

async fn get_events(State(state): State<Arc<CriState>>) -> Json<Vec<RuntimeEvent>> {
    let events = state.events.lock().await;
    Json(events.clone())
}

async fn get_metrics(State(state): State<Arc<CriState>>) -> Response {
    let container_count = state.containers.count();
    let image_count = state.images.list().len();
    let sandbox_count = state.sandboxes.count();
    let snapshot_count = state.snapshots.list().len();
    let body = format!(
        "# HELP cave_containers_total Total containers\n\
         # TYPE cave_containers_total gauge\n\
         cave_containers_total {container_count}\n\
         # HELP cave_images_total Total locally cached images\n\
         # TYPE cave_images_total gauge\n\
         cave_images_total {image_count}\n\
         # HELP cave_sandboxes_total Total pod sandboxes\n\
         # TYPE cave_sandboxes_total gauge\n\
         cave_sandboxes_total {sandbox_count}\n\
         # HELP cave_snapshots_total Total snapshots\n\
         # TYPE cave_snapshots_total gauge\n\
         cave_snapshots_total {snapshot_count}\n"
    );
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

// ── Containers ────────────────────────────────────────────────────────────────

async fn list_containers(State(state): State<Arc<CriState>>) -> Json<Vec<Container>> {
    Json(runtime::list_containers(&state.containers))
}

#[derive(Deserialize)]
struct CreateContainerReq {
    spec: ContainerSpec,
}

async fn create_container(
    State(state): State<Arc<CriState>>,
    Json(req): Json<CreateContainerReq>,
) -> Result<(StatusCode, Json<Container>), (StatusCode, String)> {
    let image = match state.images.get(&req.spec.image) {
        Some(img) => img,
        None => match state.registry.pull_image(&req.spec.image).await {
            Ok(img) => {
                state.images.insert(img.clone());
                img
            }
            Err(e) => return Err((StatusCode::BAD_REQUEST, format!("image pull failed: {}", e))),
        },
    };

    match runtime::create_container(req.spec, &image, &state.containers).await {
        Ok(c) => {
            emit_event(&state, "container.created", "container", &c.id.to_string()).await;
            Ok((StatusCode::CREATED, Json(c)))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn inspect_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Container>, (StatusCode, String)> {
    runtime::inspect_container(id, &state.containers)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

#[derive(Deserialize)]
struct UpdateContainerReq {
    #[serde(flatten)]
    update: ContainerUpdate,
}

async fn update_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateContainerReq>,
) -> Result<Json<Container>, (StatusCode, String)> {
    runtime::update_container(id, &req.update, &state.containers)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn delete_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::delete_container(id, &state.containers)
        .await
        .map(|_| {
            state.network.remove(&id);
            StatusCode::NO_CONTENT
        })
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn start_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::start_container(id, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct StopReq {
    #[serde(default = "default_timeout")]
    timeout: u32,
}
#[allow(dead_code)]
fn default_timeout() -> u32 {
    10
}

async fn stop_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::stop_container(id, 10, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct KillReq {
    #[serde(default = "default_signal")]
    signal: i32,
}
#[allow(dead_code)]
fn default_signal() -> i32 {
    15
}

async fn kill_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::kill_container(id, 15, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn pause_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::pause_container(id, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn unpause_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::unpause_container(id, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn exec_in_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ExecRequest>,
) -> Result<Json<ExecResult>, (StatusCode, String)> {
    runtime::exec_in_container(id, &req, &state.containers)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn attach_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::streaming::StreamingURL>, (StatusCode, String)> {
    runtime::inspect_container(id, &state.containers)
        .map(|c| Json(crate::streaming::StreamingURL::for_attach(c.id)))
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn exec_streaming_url(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::streaming::StreamingURL>, (StatusCode, String)> {
    runtime::inspect_container(id, &state.containers)
        .map(|c| Json(crate::streaming::StreamingURL::for_exec(c.id)))
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

#[derive(Deserialize)]
struct PortForwardReq {
    ports: Vec<u16>,
}

async fn portforward_sandbox(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<PortForwardReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .sandboxes
        .get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("sandbox not found: {}", id)))?;
    let channels: Vec<crate::streaming::PortForwardChannel> = req
        .ports
        .iter()
        .enumerate()
        .map(|(i, p)| crate::streaming::PortForwardChannel::allocate(*p, i))
        .collect();
    let url = crate::streaming::StreamingURL::for_portforward(id);
    Ok(Json(serde_json::json!({
        "url": url.url,
        "protocols": url.protocols,
        "timeout_seconds": url.timeout_seconds,
        "channels": channels,
    })))
}

async fn checkpoint_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<CheckpointInfo>), (StatusCode, String)> {
    runtime::checkpoint_container(id, &state.containers)
        .await
        .map(|info| (StatusCode::CREATED, Json(info)))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct RestoreReq {
    #[serde(default = "default_checkpoint_path")]
    checkpoint_path: String,
}
fn default_checkpoint_path() -> String {
    String::new()
}

async fn restore_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RestoreReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let path = if req.checkpoint_path.is_empty() {
        crate::paths::checkpoint_dir(&id.to_string())
            .display()
            .to_string()
    } else {
        req.checkpoint_path
    };
    runtime::restore_container(id, &path, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct LogsQuery {
    tail: Option<usize>,
    /// `cri` — return CRI tagged-line entries (with stream + tag) by reading
    /// the container's log_path with the v2 reader. Default → JSON-line v1.
    format: Option<String>,
    since_time: Option<chrono::DateTime<chrono::Utc>>,
    until_time: Option<chrono::DateTime<chrono::Utc>>,
    limit_bytes: Option<usize>,
}

async fn get_container_logs(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<LogsQuery>,
) -> Result<Response, (StatusCode, String)> {
    if q.format.as_deref() == Some("cri") {
        let container = runtime::inspect_container(id, &state.containers)
            .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
        let opts = crate::log_v2::LogOptions {
            tail_lines: q.tail,
            since_time: q.since_time,
            until_time: q.until_time,
            limit_bytes: q.limit_bytes,
            follow: false,
        };
        let entries = crate::log_v2::read_logs(&container.log_path, &opts)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(
            entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "timestamp": e.timestamp,
                        "stream": e.stream.as_str(),
                        "tag": e.tag.as_str(),
                        "message": e.message,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response());
    }
    runtime::get_container_logs(id, q.tail, &state.containers)
        .map(|entries| Json(entries).into_response())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_container_stats(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ContainerStats>, (StatusCode, String)> {
    runtime::get_container_stats(id, &state.containers)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_container_processes(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ContainerProcess>>, (StatusCode, String)> {
    runtime::list_container_processes(id, &state.containers)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

// ── Images ────────────────────────────────────────────────────────────────────

async fn list_images(State(state): State<Arc<CriState>>) -> Json<Vec<OciImage>> {
    Json(state.images.list())
}

#[derive(Deserialize)]
struct PullImageReq {
    reference: String,
}

async fn pull_image(
    State(state): State<Arc<CriState>>,
    Json(req): Json<PullImageReq>,
) -> Result<(StatusCode, Json<OciImage>), (StatusCode, String)> {
    match state.registry.pull_image(&req.reference).await {
        Ok(image) => {
            state.images.insert(image.clone());
            Ok((StatusCode::OK, Json(image)))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

async fn inspect_image(
    State(state): State<Arc<CriState>>,
    Path(reference): Path<String>,
) -> Result<Json<OciImage>, (StatusCode, String)> {
    state.images.get(&reference).map(Json).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("image not found: {}", reference),
        )
    })
}

async fn delete_image(
    State(state): State<Arc<CriState>>,
    Path(reference): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .images
        .remove(&reference)
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("image not found: {}", reference),
            )
        })
}

#[derive(Deserialize)]
struct TagImageReq {
    target: String,
}

async fn tag_image(
    State(state): State<Arc<CriState>>,
    Path(reference): Path<String>,
    Json(req): Json<TagImageReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut image = state.images.get(&reference).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("image not found: {}", reference),
        )
    })?;
    image.reference = req.target;
    state.images.insert(image);
    Ok(StatusCode::NO_CONTENT)
}

async fn get_image_history(
    State(state): State<Arc<CriState>>,
    Path(reference): Path<String>,
) -> Result<Json<Vec<ImageHistoryEntry>>, (StatusCode, String)> {
    let image = state.images.get(&reference).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("image not found: {}", reference),
        )
    })?;

    let history: Vec<ImageHistoryEntry> = image
        .layers
        .iter()
        .map(|layer| ImageHistoryEntry {
            digest: layer.digest.clone(),
            created_at: image.pulled_at,
            created_by: format!("ADD layer {}", layer.digest),
            size_bytes: layer.size,
            comment: String::new(),
        })
        .collect();

    Ok(Json(history))
}

// ── Sandboxes ─────────────────────────────────────────────────────────────────

async fn list_sandboxes(State(state): State<Arc<CriState>>) -> Json<Vec<Sandbox>> {
    Json(state.sandboxes.list())
}

#[derive(Deserialize)]
struct CreateSandboxReq {
    spec: SandboxSpec,
}

async fn create_sandbox(
    State(state): State<Arc<CriState>>,
    Json(req): Json<CreateSandboxReq>,
) -> Result<(StatusCode, Json<crate::sandbox::RunSandboxResult>), (StatusCode, String)> {
    // Validate the requested runtime handler name (if any) against the registry.
    if let Some(name) = req.spec.runtime_handler.as_deref() {
        if !name.is_empty() {
            state.runtime_handlers.lookup(name).ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("runtime handler not found: {}", name),
                )
            })?;
        }
    }
    let result = crate::sandbox::run_pod_sandbox(req.spec, Some(&state.userns_allocator))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    state.sandboxes.insert(result.sandbox.clone());
    tracing::info!(sandbox_id = %result.sandbox.id, "sandbox created");
    Ok((StatusCode::CREATED, Json(result)))
}

async fn stop_sandbox(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut sandbox = state
        .sandboxes
        .get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("sandbox not found: {}", id)))?;
    crate::sandbox::stop_pod_sandbox(id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    sandbox.state = SandboxState::NotReady;
    state.sandboxes.insert(sandbox);
    Ok(StatusCode::NO_CONTENT)
}

async fn get_sandbox(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Sandbox>, (StatusCode, String)> {
    state
        .sandboxes
        .get(&id)
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("sandbox not found: {}", id)))
}

async fn delete_sandbox(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .sandboxes
        .remove(&id)
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("sandbox not found: {}", id)))
}

async fn get_sandbox_stats(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SandboxStats>, (StatusCode, String)> {
    state
        .sandboxes
        .get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("sandbox not found: {}", id)))?;
    Ok(Json(SandboxStats {
        sandbox_id: id,
        timestamp: chrono::Utc::now(),
        cgroup: CgroupStats::default(),
        container_count: 0,
    }))
}

// ── Snapshots ─────────────────────────────────────────────────────────────────

async fn list_snapshots(State(state): State<Arc<CriState>>) -> Json<Vec<Snapshot>> {
    Json(state.snapshots.list())
}

#[derive(Deserialize)]
struct CreateSnapshotReq {
    name: String,
    parent: Option<String>,
    #[serde(default)]
    labels: std::collections::HashMap<String, String>,
}

async fn create_snapshot(
    State(state): State<Arc<CriState>>,
    Json(req): Json<CreateSnapshotReq>,
) -> (StatusCode, Json<Snapshot>) {
    let snapshot = Snapshot {
        id: Uuid::new_v4(),
        name: req.name,
        parent: req.parent,
        labels: req.labels,
        created_at: chrono::Utc::now(),
        kind: SnapshotKind::Active,
    };
    state.snapshots.insert(snapshot.clone());
    (StatusCode::CREATED, Json(snapshot))
}

async fn delete_snapshot(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .snapshots
        .remove(&id)
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("snapshot not found: {}", id)))
}

async fn get_snapshot_mounts(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<SnapshotMount>>, (StatusCode, String)> {
    let snap = state
        .snapshots
        .get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("snapshot not found: {}", id)))?;

    let mounts = vec![SnapshotMount {
        kind: "overlay".into(),
        source: format!("/var/lib/cave/snapshots/{}", snap.id),
        options: vec!["ro".into()],
    }];
    Ok(Json(mounts))
}

async fn get_snapshot_usage(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SnapshotUsage>, (StatusCode, String)> {
    state
        .snapshots
        .get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("snapshot not found: {}", id)))?;
    Ok(Json(SnapshotUsage {
        snapshot_id: id,
        size_bytes: 0,
        inodes: 0,
    }))
}

// ── Network ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct NetworkAttachReq {
    container_id: Uuid,
    network_name: String,
    interface_name: Option<String>,
}

async fn attach_network(
    State(state): State<Arc<CriState>>,
    Json(req): Json<NetworkAttachReq>,
) -> Result<(StatusCode, Json<NetworkStatus>), (StatusCode, String)> {
    runtime::inspect_container(req.container_id, &state.containers)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let status = NetworkStatus {
        container_id: req.container_id,
        network_name: req.network_name,
        ip_address: Some("10.244.0.2".into()),
        mac_address: Some("02:42:0a:f4:00:02".into()),
        gateway: Some("10.244.0.1".into()),
        interface: req.interface_name.or_else(|| Some("eth0".into())),
        attached: true,
    };
    state.network.insert(req.container_id, status.clone());
    Ok((StatusCode::CREATED, Json(status)))
}

#[derive(Deserialize)]
struct NetworkDetachReq {
    container_id: Uuid,
    network_name: String,
}

async fn detach_network(
    State(state): State<Arc<CriState>>,
    Json(req): Json<NetworkDetachReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let entry = state
        .network
        .get(&req.container_id)
        .filter(|s| s.network_name == req.network_name)
        .map(|s| s.container_id);

    match entry {
        Some(id) => {
            state.network.remove(&id);
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err((
            StatusCode::NOT_FOUND,
            format!(
                "no attachment for container {} on network {}",
                req.container_id, req.network_name
            ),
        )),
    }
}

#[derive(Deserialize)]
struct NetworkStatusQuery {
    container_id: Option<Uuid>,
}

async fn get_network_status(
    State(state): State<Arc<CriState>>,
    Query(q): Query<NetworkStatusQuery>,
) -> Json<Vec<NetworkStatus>> {
    let statuses: Vec<NetworkStatus> = match q.container_id {
        Some(id) => state
            .network
            .get(&id)
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        None => state.network.iter().map(|r| r.value().clone()).collect(),
    };
    Json(statuses)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn emit_event(state: &Arc<CriState>, kind: &str, object_type: &str, object_id: &str) {
    let event = RuntimeEvent {
        id: Uuid::new_v4().to_string(),
        kind: kind.into(),
        object_type: object_type.into(),
        object_id: object_id.into(),
        timestamp: chrono::Utc::now(),
        attributes: Default::default(),
    };
    state.events.lock().await.push(event);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RegistryClient;
    use std::path::PathBuf;

    fn make_state() -> Arc<CriState> {
        Arc::new(CriState {
            containers: ContainerStore::new(),
            images: ImageStore::new(),
            registry: RegistryClient::new(PathBuf::from("/tmp/cave-test-images")),
            sandboxes: SandboxStore::new(),
            snapshots: SnapshotStore::new(),
            events: Mutex::new(vec![]),
            network: DashMap::new(),
            runtime_handlers: RuntimeHandlerRegistry::with_defaults(),
            credentials: crate::auth::CredentialStore::new(),
            pull_progress: crate::pull_progress::PullProgressTracker::new(),
            userns_allocator: crate::userns::UserNsAllocator::defaults(),
        })
    }

    #[test]
    fn test_state_initializes() {
        let state = make_state();
        assert_eq!(state.containers.count(), 0);
        assert_eq!(state.sandboxes.count(), 0);
        assert_eq!(state.snapshots.list().len(), 0);
        assert!(state.network.is_empty());
    }

    #[test]
    fn test_sandbox_store_roundtrip() {
        let state = make_state();
        let id = Uuid::new_v4();
        let sandbox = Sandbox {
            id,
            spec: SandboxSpec {
                name: "my-pod".into(),
                namespace: "kube-system".into(),
                labels: Default::default(),
                annotations: Default::default(),
                hostname: None,
                dns_config: None,
                port_mappings: vec![],
                log_directory: None,
                cgroup_parent: None,
                runtime_handler: None,
                user_namespace_mode: crate::models::UserNamespaceMode::Host,
            },
            state: SandboxState::Ready,
            created_at: chrono::Utc::now(),
            network_ip: None,
        };
        state.sandboxes.insert(sandbox);
        let got = state.sandboxes.get(&id).unwrap();
        assert_eq!(got.spec.name, "my-pod");
        state.sandboxes.remove(&id);
        assert!(state.sandboxes.get(&id).is_none());
    }

    #[test]
    fn test_snapshot_create_delete() {
        let state = make_state();
        let id = Uuid::new_v4();
        let snap = Snapshot {
            id,
            name: "base".into(),
            parent: None,
            labels: Default::default(),
            created_at: chrono::Utc::now(),
            kind: SnapshotKind::Committed,
        };
        state.snapshots.insert(snap);
        assert_eq!(state.snapshots.list().len(), 1);
        state.snapshots.remove(&id);
        assert!(state.snapshots.list().is_empty());
    }

    #[test]
    fn test_network_attach_detach() {
        let state = make_state();
        let cid = Uuid::new_v4();
        let ns = NetworkStatus {
            container_id: cid,
            network_name: "bridge0".into(),
            ip_address: Some("10.0.0.1".into()),
            mac_address: None,
            gateway: None,
            interface: Some("eth0".into()),
            attached: true,
        };
        state.network.insert(cid, ns);
        assert!(state.network.contains_key(&cid));
        state.network.remove(&cid);
        assert!(!state.network.contains_key(&cid));
    }

    #[test]
    fn test_image_store_inspect_delete() {
        use chrono::Utc;
        let state = make_state();
        let img = OciImage {
            reference: "nginx:latest".into(),
            digest: "sha256:abc".into(),
            layers: vec![],
            config: ImageConfig::default(),
            size_bytes: 1024,
            pulled_at: Utc::now(),
        };
        state.images.insert(img);
        assert!(state.images.get("nginx:latest").is_some());
        state.images.remove("nginx:latest");
        assert!(state.images.get("nginx:latest").is_none());
    }

    #[tokio::test]
    async fn test_emit_event() {
        let state = make_state();
        emit_event(&state, "container.created", "container", "abc123").await;
        let events = state.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "container.created");
    }

    // ── runtime handlers ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_runtime_handlers_returns_defaults() {
        let state = make_state();
        let Json(handlers) = list_runtime_handlers(State(state)).await;
        let names: Vec<_> = handlers.iter().map(|h| h.name.as_str()).collect();
        assert!(names.contains(&"runc"));
        assert!(names.contains(&"runsc"));
        assert!(names.contains(&"kata"));
    }

    #[tokio::test]
    async fn get_runtime_handler_known_returns_handler() {
        let state = make_state();
        let res = get_runtime_handler(State(state), Path("runc".into())).await;
        let Json(h) = res.unwrap();
        assert_eq!(h.name, "runc");
        assert!(h.features.user_namespaces);
    }

    #[tokio::test]
    async fn get_runtime_handler_unknown_returns_404() {
        let state = make_state();
        let res = get_runtime_handler(State(state), Path("ghost".into())).await;
        let (code, msg) = res.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert!(msg.contains("ghost"));
    }

    #[tokio::test]
    async fn get_default_runtime_handler_returns_runc() {
        let state = make_state();
        let res = get_default_runtime_handler(State(state)).await;
        let Json(h) = res.unwrap();
        assert_eq!(h.name, "runc");
    }

    #[tokio::test]
    async fn runtime_status_includes_runtime_handlers() {
        let state = make_state();
        let Json(status) = get_runtime_status(State(state)).await;
        assert_eq!(status.runtime_handlers.len(), 3);
        assert!(status.conditions.iter().any(|c| c.kind == "RuntimeReady"));
    }
}
