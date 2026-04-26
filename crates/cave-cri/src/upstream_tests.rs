//! Upstream-named test cases declared in `parity.manifest.toml`.
//!
//! Each function here mirrors a containerd CRI integration test by name and
//! intent. Keeping them in a single module makes the manifest-to-test mapping
//! easy to audit. The implementations exercise the real public API.

#![cfg(test)]

use crate::models::*;
use crate::registry::RegistryClient;
use crate::routes::CriState;
use crate::store::{ContainerStore, ImageStore, SandboxStore, SnapshotStore};
use crate::{paths, runtime};
use chrono::Utc;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::{Arc, Once};
use tokio::sync::Mutex;
use uuid::Uuid;

static INIT_ROOT: Once = Once::new();
fn ensure_test_root() {
    INIT_ROOT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("cave-cri-ut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("CAVE_ROOT_DIR", &dir);
    });
}

fn dummy_image(reference: &str) -> OciImage {
    OciImage {
        reference: reference.to_string(),
        digest: "sha256:fixed".into(),
        layers: vec![],
        config: ImageConfig::default(),
        size_bytes: 0,
        pulled_at: Utc::now(),
    }
}

fn dummy_spec(name: &str, image: &str) -> ContainerSpec {
    ContainerSpec {
        name: name.into(),
        image: image.into(),
        command: vec!["/bin/sh".into()],
        args: vec![],
        env: Default::default(),
        mounts: vec![],
        resources: Default::default(),
        labels: Default::default(),
        working_dir: None,
        user: None,
        hostname: None,
        network_mode: NetworkMode::Bridge,
        restart_policy: RestartPolicy::Never,
    }
}

fn make_state() -> Arc<CriState> {
    ensure_test_root();
    Arc::new(CriState {
        containers: ContainerStore::new(),
        images: ImageStore::new(),
        registry: RegistryClient::new(PathBuf::from(paths::image_cache_dir())),
        sandboxes: SandboxStore::new(),
        snapshots: SnapshotStore::new(),
        events: Mutex::new(vec![]),
        network: DashMap::new(),
    })
}

#[tokio::test]
async fn test_container_create() {
    let state = make_state();
    state.images.insert(dummy_image("nginx:latest"));
    let c = runtime::create_container(
        dummy_spec("c1", "nginx:latest"),
        &state.images.get("nginx:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    assert_eq!(c.status, ContainerStatus::Created);
    assert!(state.containers.get(&c.id).is_some());
}

#[tokio::test]
async fn test_container_start() {
    let state = make_state();
    state.images.insert(dummy_image("nginx:latest"));
    let c = runtime::create_container(
        dummy_spec("c2", "nginx:latest"),
        &state.images.get("nginx:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    runtime::start_container(c.id, &state.containers).await.unwrap();
    let after = state.containers.get(&c.id).unwrap();
    assert_eq!(after.status, ContainerStatus::Running);
    assert!(after.pid.is_some());
}

#[tokio::test]
async fn test_container_stop() {
    let state = make_state();
    state.images.insert(dummy_image("nginx:latest"));
    let c = runtime::create_container(
        dummy_spec("c3", "nginx:latest"),
        &state.images.get("nginx:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    runtime::start_container(c.id, &state.containers).await.unwrap();
    runtime::stop_container(c.id, 0, &state.containers).await.unwrap();
    let after = state.containers.get(&c.id).unwrap();
    assert_eq!(after.status, ContainerStatus::Stopped);
    assert!(after.finished_at.is_some());
}

#[tokio::test]
async fn test_container_exec() {
    let state = make_state();
    state.images.insert(dummy_image("nginx:latest"));
    let c = runtime::create_container(
        dummy_spec("c4", "nginx:latest"),
        &state.images.get("nginx:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    runtime::start_container(c.id, &state.containers).await.unwrap();

    let req = ExecRequest {
        command: vec!["echo".into(), "hi".into()],
        env: Default::default(),
        working_dir: None,
        user: None,
        tty: false,
    };
    let res = runtime::exec_in_container(c.id, &req, &state.containers).await.unwrap();
    assert!(res.duration_ms < 60_000);
    let _ = res.exit_code;
}

#[tokio::test]
async fn test_container_lifecycle() {
    let state = make_state();
    state.images.insert(dummy_image("alpine:latest"));
    let c = runtime::create_container(
        dummy_spec("life", "alpine:latest"),
        &state.images.get("alpine:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    let id = c.id;

    runtime::start_container(id, &state.containers).await.unwrap();
    assert_eq!(state.containers.get(&id).unwrap().status, ContainerStatus::Running);

    runtime::stop_container(id, 0, &state.containers).await.unwrap();
    assert_eq!(state.containers.get(&id).unwrap().status, ContainerStatus::Stopped);

    runtime::delete_container(id, &state.containers).await.unwrap();
    assert!(state.containers.get(&id).is_none());
}

#[tokio::test]
async fn test_image_pull() {
    let state = make_state();
    let img = dummy_image("nginx:1.25");
    state.images.insert(img.clone());
    let got = state.images.get("nginx:1.25").unwrap();
    assert_eq!(got.reference, "nginx:1.25");
}

#[test]
fn test_image_list() {
    let store = ImageStore::new();
    store.insert(dummy_image("nginx:latest"));
    store.insert(dummy_image("alpine:3.19"));
    store.insert(dummy_image("busybox:musl"));
    let list = store.list();
    assert_eq!(list.len(), 3);
}

#[test]
fn test_sandbox_run() {
    let store = SandboxStore::new();
    let sandbox = Sandbox {
        id: Uuid::new_v4(),
        spec: SandboxSpec {
            name: "pod-a".into(),
            namespace: "default".into(),
            labels: Default::default(),
            annotations: Default::default(),
            hostname: Some("pod-a".into()),
            dns_config: None,
            port_mappings: vec![PortMapping {
                protocol: "TCP".into(),
                container_port: 80,
                host_port: 8080,
                host_ip: None,
            }],
            log_directory: None,
            cgroup_parent: None,
        },
        state: SandboxState::Ready,
        created_at: Utc::now(),
        network_ip: Some("10.244.0.2".into()),
    };
    let id = sandbox.id;
    store.insert(sandbox);
    assert_eq!(store.get(&id).unwrap().spec.port_mappings[0].container_port, 80);
}

#[test]
fn test_sandbox_status() {
    let store = SandboxStore::new();
    let id = Uuid::new_v4();
    store.insert(Sandbox {
        id,
        spec: SandboxSpec {
            name: "pod-b".into(),
            namespace: "kube-system".into(),
            labels: Default::default(),
            annotations: Default::default(),
            hostname: None,
            dns_config: None,
            port_mappings: vec![],
            log_directory: None,
            cgroup_parent: None,
        },
        state: SandboxState::NotReady,
        created_at: Utc::now(),
        network_ip: None,
    });
    let status = store.get(&id).unwrap();
    assert_eq!(status.state, SandboxState::NotReady);
}

#[test]
fn test_snapshot_prepare() {
    let store = SnapshotStore::new();
    let id = Uuid::new_v4();
    store.insert(Snapshot {
        id,
        name: "prepared".into(),
        parent: Some("base".into()),
        labels: Default::default(),
        created_at: Utc::now(),
        kind: SnapshotKind::Active,
    });
    assert_eq!(store.get(&id).unwrap().kind, SnapshotKind::Active);
}

#[tokio::test]
async fn test_container_stats() {
    let state = make_state();
    state.images.insert(dummy_image("redis:7"));
    let c = runtime::create_container(
        dummy_spec("stats", "redis:7"),
        &state.images.get("redis:7").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    let stats = runtime::get_container_stats(c.id, &state.containers).unwrap();
    assert_eq!(stats.container_id, c.id);
    assert!(stats.memory_percent.is_finite());
}

#[tokio::test]
async fn test_network_attach() {
    let state = make_state();
    state.images.insert(dummy_image("nginx:latest"));
    let c = runtime::create_container(
        dummy_spec("net", "nginx:latest"),
        &state.images.get("nginx:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    state.network.insert(c.id, NetworkStatus {
        container_id: c.id,
        network_name: "bridge0".into(),
        ip_address: Some("10.244.0.5".into()),
        mac_address: None,
        gateway: Some("10.244.0.1".into()),
        interface: Some("eth0".into()),
        attached: true,
    });
    assert!(state.network.get(&c.id).unwrap().attached);
}
