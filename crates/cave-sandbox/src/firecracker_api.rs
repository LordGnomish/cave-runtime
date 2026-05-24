// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: REST surface adapted from firecracker-microvm/firecracker
// src/api_server/* (Apache-2.0).
//! Firecracker REST API — axum router mirroring the upstream surface.
//!
//! Boots in-process: the routes update the in-memory `VmResources`. Real
//! VMM is OUT OF SCOPE.

use crate::firecracker_vmm::{
    Balloon, BootSource, Drive, Entropy, Logger, MachineConfig, Metrics, NetworkInterface, VmResources, Vsock,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared in-process VMM state.
#[derive(Default, Clone)]
pub struct VmmState {
    pub inner: Arc<RwLock<VmResources>>,
    pub instance_info: Arc<RwLock<InstanceInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstanceInfo {
    pub id: String,
    pub state: String,
    pub vmm_version: String,
    pub app_name: String,
}

impl Default for InstanceInfo {
    fn default() -> Self {
        InstanceInfo {
            id: "anonymous-instance".into(),
            state: "Not started".into(),
            vmm_version: "1.15.1".into(),
            app_name: "Firecracker".into(),
        }
    }
}

/// `POST /actions` payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionBody {
    pub action_type: ActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ActionType {
    InstanceStart,
    SendCtrlAltDel,
    FlushMetrics,
}

impl VmmState {
    pub fn new() -> Self { VmmState::default() }
}

pub fn router(state: VmmState) -> Router {
    Router::new()
        .route("/", get(get_instance))
        .route("/machine-config", put(put_machine_config).patch(patch_machine_config))
        .route("/boot-source", put(put_boot_source))
        .route("/drives/{id}", put(put_drive))
        .route("/network-interfaces/{id}", put(put_netif))
        .route("/vsock", put(put_vsock))
        .route("/balloon", put(put_balloon).patch(patch_balloon))
        .route("/logger", put(put_logger))
        .route("/metrics", put(put_metrics))
        .route("/entropy", put(put_entropy))
        .route("/actions", post(post_action))
        .with_state(state)
}

async fn get_instance(State(s): State<VmmState>) -> impl IntoResponse {
    let info = s.instance_info.read().clone();
    (StatusCode::OK, Json(info))
}

async fn put_machine_config(State(s): State<VmmState>, Json(body): Json<MachineConfig>) -> StatusCode {
    s.inner.write().machine_config = body;
    StatusCode::NO_CONTENT
}

async fn patch_machine_config(State(s): State<VmmState>, Json(body): Json<MachineConfig>) -> StatusCode {
    s.inner.write().machine_config = body;
    StatusCode::NO_CONTENT
}

async fn put_boot_source(State(s): State<VmmState>, Json(body): Json<BootSource>) -> StatusCode {
    s.inner.write().boot_source = body;
    StatusCode::NO_CONTENT
}

async fn put_drive(State(s): State<VmmState>, Path(id): Path<String>, Json(mut body): Json<Drive>) -> StatusCode {
    body.drive_id = id;
    let mut g = s.inner.write();
    g.drives.retain(|d| d.drive_id != body.drive_id);
    g.drives.push(body);
    StatusCode::NO_CONTENT
}

async fn put_netif(State(s): State<VmmState>, Path(id): Path<String>, Json(mut body): Json<NetworkInterface>) -> StatusCode {
    body.iface_id = id;
    let mut g = s.inner.write();
    g.network_interfaces.retain(|n| n.iface_id != body.iface_id);
    g.network_interfaces.push(body);
    StatusCode::NO_CONTENT
}

async fn put_vsock(State(s): State<VmmState>, Json(body): Json<Vsock>) -> StatusCode {
    s.inner.write().vsock = Some(body);
    StatusCode::NO_CONTENT
}

async fn put_balloon(State(s): State<VmmState>, Json(body): Json<Balloon>) -> StatusCode {
    s.inner.write().balloon = Some(body);
    StatusCode::NO_CONTENT
}

async fn patch_balloon(State(s): State<VmmState>, Json(body): Json<Balloon>) -> StatusCode {
    s.inner.write().balloon = Some(body);
    StatusCode::NO_CONTENT
}

async fn put_logger(State(s): State<VmmState>, Json(body): Json<Logger>) -> StatusCode {
    s.inner.write().logger = Some(body);
    StatusCode::NO_CONTENT
}

async fn put_metrics(State(s): State<VmmState>, Json(body): Json<Metrics>) -> StatusCode {
    s.inner.write().metrics = Some(body);
    StatusCode::NO_CONTENT
}

async fn put_entropy(State(s): State<VmmState>, Json(body): Json<Entropy>) -> StatusCode {
    s.inner.write().entropy = Some(body);
    StatusCode::NO_CONTENT
}

async fn post_action(State(s): State<VmmState>, Json(body): Json<ActionBody>) -> impl IntoResponse {
    let mut info = s.instance_info.write();
    match body.action_type {
        ActionType::InstanceStart => {
            // Validate first; bail if invalid.
            let resources = s.inner.read();
            if let Err(e) = resources.validate() {
                return (StatusCode::BAD_REQUEST, e).into_response();
            }
            info.state = "Running".into();
            StatusCode::NO_CONTENT.into_response()
        }
        ActionType::SendCtrlAltDel => {
            info.state = "Not started".into();
            StatusCode::NO_CONTENT.into_response()
        }
        ActionType::FlushMetrics => StatusCode::NO_CONTENT.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn body(s: &impl Serialize) -> Body { Body::from(serde_json::to_vec(s).unwrap()) }

    #[tokio::test]
    async fn get_root_returns_instance_info() {
        let app = router(VmmState::new());
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn put_boot_source_then_start() {
        let state = VmmState::new();
        let app = router(state.clone());
        let bs = BootSource { kernel_image_path: "/k".into(), boot_args: None, initrd_path: None };
        let req = Request::builder()
            .method("PUT").uri("/boot-source")
            .header("content-type", "application/json")
            .body(body(&bs)).unwrap();
        assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::NO_CONTENT);

        let req = Request::builder()
            .method("POST").uri("/actions")
            .header("content-type", "application/json")
            .body(body(&ActionBody { action_type: ActionType::InstanceStart })).unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(state.instance_info.read().state, "Running");
    }

    #[tokio::test]
    async fn start_without_kernel_400s() {
        let state = VmmState::new();
        let app = router(state.clone());
        let req = Request::builder()
            .method("POST").uri("/actions")
            .header("content-type", "application/json")
            .body(body(&ActionBody { action_type: ActionType::InstanceStart })).unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_drive_persists() {
        let state = VmmState::new();
        let app = router(state.clone());
        let d = Drive {
            drive_id: "rootfs".into(), path_on_host: "/r".into(),
            is_root_device: true, is_read_only: true,
            partuuid: None, io_engine: None, cache_type: None, rate_limiter: None,
        };
        let req = Request::builder()
            .method("PUT").uri("/drives/rootfs")
            .header("content-type", "application/json")
            .body(body(&d)).unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(state.inner.read().drives.len(), 1);
    }

    #[tokio::test]
    async fn put_netif_persists() {
        let state = VmmState::new();
        let app = router(state.clone());
        let n = NetworkInterface { iface_id: "eth0".into(), host_dev_name: "tap0".into(), ..NetworkInterface::default() };
        let req = Request::builder()
            .method("PUT").uri("/network-interfaces/eth0")
            .header("content-type", "application/json")
            .body(body(&n)).unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(state.inner.read().network_interfaces.len(), 1);
    }

    #[tokio::test]
    async fn put_vsock_persists() {
        let state = VmmState::new();
        let app = router(state.clone());
        let v = Vsock { vsock_id: "v0".into(), guest_cid: 3, uds_path: "/run/v.sock".into() };
        let req = Request::builder()
            .method("PUT").uri("/vsock")
            .header("content-type", "application/json")
            .body(body(&v)).unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert!(state.inner.read().vsock.is_some());
    }

    #[tokio::test]
    async fn put_machine_config_persists() {
        let state = VmmState::new();
        let app = router(state.clone());
        let m = MachineConfig { vcpu_count: 2, mem_size_mib: 512, ..MachineConfig::default() };
        let req = Request::builder()
            .method("PUT").uri("/machine-config")
            .header("content-type", "application/json")
            .body(body(&m)).unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(state.inner.read().machine_config.vcpu_count, 2);
    }

    #[tokio::test]
    async fn flush_metrics_succeeds_always() {
        let state = VmmState::new();
        let app = router(state);
        let req = Request::builder()
            .method("POST").uri("/actions")
            .header("content-type", "application/json")
            .body(body(&ActionBody { action_type: ActionType::FlushMetrics })).unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::NO_CONTENT);
    }
}
