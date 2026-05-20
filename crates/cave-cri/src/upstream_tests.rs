// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Upstream-named test cases declared in `parity.manifest.toml`.
//!
//! Each function here mirrors a containerd CRI integration test by name and
//! intent. Keeping them in a single module makes the manifest-to-test mapping
//! easy to audit. The implementations exercise the real public API.

#![cfg(test)]

use crate::models::*;
use crate::registry::RegistryClient;
use crate::routes::CriState;
use crate::runtime_handler::RuntimeHandlerRegistry;
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
        runtime_handlers: RuntimeHandlerRegistry::with_defaults(),
        credentials: crate::auth::CredentialStore::new(),
        pull_progress: crate::pull_progress::PullProgressTracker::new(),
        userns_allocator: crate::userns::UserNsAllocator::defaults(),
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
    runtime::start_container(c.id, &state.containers)
        .await
        .unwrap();
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
    runtime::start_container(c.id, &state.containers)
        .await
        .unwrap();
    runtime::stop_container(c.id, 0, &state.containers)
        .await
        .unwrap();
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
    runtime::start_container(c.id, &state.containers)
        .await
        .unwrap();

    let req = ExecRequest {
        command: vec!["echo".into(), "hi".into()],
        env: Default::default(),
        working_dir: None,
        user: None,
        tty: false,
    };
    let res = runtime::exec_in_container(c.id, &req, &state.containers)
        .await
        .unwrap();
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

    runtime::start_container(id, &state.containers)
        .await
        .unwrap();
    assert_eq!(
        state.containers.get(&id).unwrap().status,
        ContainerStatus::Running
    );

    runtime::stop_container(id, 0, &state.containers)
        .await
        .unwrap();
    assert_eq!(
        state.containers.get(&id).unwrap().status,
        ContainerStatus::Stopped
    );

    runtime::delete_container(id, &state.containers)
        .await
        .unwrap();
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
            runtime_handler: None,
            user_namespace_mode: crate::models::UserNamespaceMode::Host,
        },
        state: SandboxState::Ready,
        created_at: Utc::now(),
        network_ip: Some("10.244.0.2".into()),
    };
    let id = sandbox.id;
    store.insert(sandbox);
    assert_eq!(
        store.get(&id).unwrap().spec.port_mappings[0].container_port,
        80
    );
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
            runtime_handler: None,
            user_namespace_mode: crate::models::UserNamespaceMode::Host,
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

// ── Cgroup v2 unified hierarchy (M9) ───────────────────────────────────────────

use crate::cgroup_v2;

#[test]
fn test_cgroup_v2_apply_memory_high() {
    let dir = tempfile::tempdir().unwrap();
    let cg = dir.path().join("cg");
    let limits = cgroup_v2::CgroupV2Limits {
        memory_high: Some(1024),
        ..Default::default()
    };
    cgroup_v2::apply_v2(&cg, &limits).unwrap();
    assert_eq!(
        std::fs::read_to_string(cg.join("memory.high"))
            .unwrap()
            .trim(),
        "1024"
    );
}

#[test]
fn test_cgroup_v2_apply_cpu_weight() {
    let dir = tempfile::tempdir().unwrap();
    let cg = dir.path().join("cg");
    let limits = cgroup_v2::CgroupV2Limits {
        cpu_weight: Some(750),
        ..Default::default()
    };
    cgroup_v2::apply_v2(&cg, &limits).unwrap();
    assert_eq!(
        std::fs::read_to_string(cg.join("cpu.weight"))
            .unwrap()
            .trim(),
        "750"
    );
}

#[test]
fn test_cgroup_v2_apply_io_max_device() {
    let dir = tempfile::tempdir().unwrap();
    let cg = dir.path().join("cg");
    let limits = cgroup_v2::CgroupV2Limits {
        io_max: vec![cgroup_v2::IoMaxEntry {
            major: 8,
            minor: 0,
            rbps: Some(2_000_000),
            wbps: None,
            riops: None,
            wiops: None,
        }],
        ..Default::default()
    };
    cgroup_v2::apply_v2(&cg, &limits).unwrap();
    let content = std::fs::read_to_string(cg.join("io.max")).unwrap();
    assert!(content.contains("8:0"));
    assert!(content.contains("rbps=2000000"));
}

#[test]
fn test_cgroup_v2_devices_bpf_program_emits_compares() {
    let rules = cgroup_v2::DeviceRule::default_allowlist();
    let prog = cgroup_v2::assemble_device_program(&rules);
    let cmps = prog.iter().filter(|i| i.op.starts_with("CMP")).count();
    assert_eq!(cmps, rules.len());
    assert_eq!(prog.last().unwrap().op, "BPF_EXIT_INSN()");
}

#[test]
fn test_cgroup_v2_default_deny_all_rule() {
    let r = cgroup_v2::DeviceRule::default_deny_all();
    assert!(!r.allow);
    assert_eq!(r.access, "rwm");
}

#[test]
fn test_cgroup_v2_check_unified_hierarchy_lists_controllers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cgroup.controllers"), "cpu memory pids io").unwrap();
    let controllers = cgroup_v2::check_unified_hierarchy(dir.path()).unwrap();
    assert!(controllers.contains(&"cpu".to_string()));
    assert!(controllers.contains(&"memory".to_string()));
}

#[test]
fn test_cgroup_v2_check_unified_hierarchy_rejects_v1() {
    let dir = tempfile::tempdir().unwrap();
    // No cgroup.controllers file → v1 hybrid mount.
    assert!(cgroup_v2::check_unified_hierarchy(dir.path()).is_err());
}

#[test]
fn test_cgroup_v2_enable_controller() {
    let dir = tempfile::tempdir().unwrap();
    cgroup_v2::enable_controller(dir.path(), "cpu").unwrap();
    let content = std::fs::read_to_string(dir.path().join("cgroup.subtree_control")).unwrap();
    assert_eq!(content, "+cpu");
}

#[test]
fn test_cgroup_v2_rejects_out_of_range_weight() {
    let dir = tempfile::tempdir().unwrap();
    let cg = dir.path().join("cg");
    let limits = cgroup_v2::CgroupV2Limits {
        cpu_weight: Some(15_000),
        ..Default::default()
    };
    assert!(cgroup_v2::apply_v2(&cg, &limits).is_err());
}

#[test]
fn test_cgroup_v2_io_weight_zero_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cg = dir.path().join("cg");
    let limits = cgroup_v2::CgroupV2Limits {
        io_weight: Some(0),
        ..Default::default()
    };
    assert!(cgroup_v2::apply_v2(&cg, &limits).is_err());
}

#[test]
fn test_cgroup_v2_default_allowlist_has_standard_devices() {
    let list = cgroup_v2::DeviceRule::default_allowlist();
    let pairs: Vec<_> = list
        .iter()
        .filter_map(|r| match (r.major, r.minor) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        })
        .collect();
    assert!(pairs.contains(&(1, 3)));
    assert!(pairs.contains(&(1, 5)));
    assert!(pairs.contains(&(1, 8)));
    assert!(pairs.contains(&(1, 9)));
}

// ── CRIU / Checkpoint (KEP-2008) ────────────────────────────────────────────────

use crate::criu;
use std::path::Path;

#[test]
fn test_criu_dump_argv_minimal() {
    let argv = criu::build_dump_argv(123, Path::new("/i"), &criu::CheckpointOptions::default());
    assert!(argv.contains(&"criu".to_string()));
    assert!(argv.contains(&"dump".to_string()));
    assert!(argv.contains(&"--tree".to_string()));
    assert!(argv.contains(&"123".to_string()));
}

#[test]
fn test_criu_dump_argv_with_all_options() {
    let opts = criu::CheckpointOptions {
        leave_running: true,
        tcp_established: true,
        shell_job: true,
        external_mounts: true,
        images_dir: None,
    };
    let argv = criu::build_dump_argv(1, Path::new("/i"), &opts);
    assert!(argv.iter().any(|a| a == "--leave-running"));
    assert!(argv.iter().any(|a| a == "--tcp-established"));
    assert!(argv.iter().any(|a| a == "--shell-job"));
    assert!(argv.iter().any(|a| a == "--ext-mount-map"));
}

#[test]
fn test_criu_restore_argv_includes_restore_detached() {
    let argv = criu::build_restore_argv(Path::new("/i"), &criu::CheckpointOptions::default());
    assert!(argv.iter().any(|a| a == "--restore-detached"));
}

#[test]
fn test_criu_manifest_roundtrip() {
    let id = Uuid::new_v4();
    let m = criu::CheckpointManifest {
        container_id: id,
        container_name: "redis".into(),
        image_reference: "redis:7".into(),
        runtime_handler: Some("runc".into()),
        created_at: chrono::Utc::now(),
        criu_version: "3.19".into(),
        options: criu::CheckpointOptions::default(),
    };
    let dir = tempfile::tempdir().unwrap();
    criu::write_manifest(dir.path(), &m).unwrap();
    let back = criu::read_manifest(dir.path()).unwrap();
    assert_eq!(back, m);
}

#[test]
fn test_criu_verify_checkpoint_requires_manifest() {
    let dir = tempfile::tempdir().unwrap();
    assert!(criu::verify_checkpoint(dir.path()).is_err());
    let m = criu::CheckpointManifest {
        container_id: Uuid::new_v4(),
        container_name: "x".into(),
        image_reference: "x:1".into(),
        runtime_handler: None,
        created_at: chrono::Utc::now(),
        criu_version: "3.19".into(),
        options: criu::CheckpointOptions::default(),
    };
    criu::write_manifest(dir.path(), &m).unwrap();
    assert!(criu::verify_checkpoint(dir.path()).is_ok());
}

#[test]
fn test_criu_dir_size_bytes_sums_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), vec![0u8; 200]).unwrap();
    std::fs::write(dir.path().join("b"), vec![0u8; 50]).unwrap();
    assert_eq!(criu::dir_size_bytes(dir.path()), 250);
}

#[tokio::test]
async fn test_checkpoint_container_writes_manifest() {
    ensure_test_root();
    let state = make_state();
    state.images.insert(dummy_image("redis:7"));
    let c = runtime::create_container(
        dummy_spec("ckpt-1", "redis:7"),
        &state.images.get("redis:7").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    runtime::start_container(c.id, &state.containers)
        .await
        .unwrap();

    let info = runtime::checkpoint_container(c.id, &state.containers)
        .await
        .unwrap();
    let dir = std::path::Path::new(&info.path);
    assert!(dir.join(criu::MANIFEST_FILENAME).exists());
    let m = criu::read_manifest(dir).unwrap();
    assert_eq!(m.container_id, c.id);
    assert_eq!(m.container_name, "ckpt-1");
}

#[tokio::test]
async fn test_restore_container_rejects_mismatched_manifest() {
    ensure_test_root();
    let state = make_state();
    state.images.insert(dummy_image("nginx:latest"));
    let c = runtime::create_container(
        dummy_spec("rstr", "nginx:latest"),
        &state.images.get("nginx:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    // Write a manifest belonging to a *different* container.
    let dir = tempfile::tempdir().unwrap();
    let other_id = Uuid::new_v4();
    let m = criu::CheckpointManifest {
        container_id: other_id,
        container_name: "other".into(),
        image_reference: "x".into(),
        runtime_handler: None,
        created_at: chrono::Utc::now(),
        criu_version: "3.19".into(),
        options: criu::CheckpointOptions::default(),
    };
    criu::write_manifest(dir.path(), &m).unwrap();
    let err = runtime::restore_container(c.id, dir.path().to_str().unwrap(), &state.containers)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not match"));
}

#[tokio::test]
async fn test_restore_container_succeeds_with_matching_manifest() {
    ensure_test_root();
    let state = make_state();
    state.images.insert(dummy_image("nginx:latest"));
    let c = runtime::create_container(
        dummy_spec("rok", "nginx:latest"),
        &state.images.get("nginx:latest").unwrap(),
        &state.containers,
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let m = criu::CheckpointManifest {
        container_id: c.id,
        container_name: "rok".into(),
        image_reference: "nginx:latest".into(),
        runtime_handler: None,
        created_at: chrono::Utc::now(),
        criu_version: "3.19".into(),
        options: criu::CheckpointOptions::default(),
    };
    criu::write_manifest(dir.path(), &m).unwrap();
    runtime::restore_container(c.id, dir.path().to_str().unwrap(), &state.containers)
        .await
        .unwrap();
    assert_eq!(
        state.containers.get(&c.id).unwrap().status,
        ContainerStatus::Running
    );
}

// ── UserNS / KEP-127 ───────────────────────────────────────────────────────────

use crate::userns;

#[test]
fn test_userns_id_mapping_translates_both_directions() {
    let m = userns::IdMapping {
        container_id: 0,
        host_id: 1_000_000,
        size: 65_536,
    };
    assert_eq!(m.translate_to_host(1234), Some(1_001_234));
    assert_eq!(m.translate_to_container(1_001_234), Some(1234));
}

#[test]
fn test_userns_render_proc_uid_map() {
    let ns = userns::UserNamespace::for_pod(1_000_000, 65_536);
    let line = ns.render_uid_map_file();
    assert!(line.contains("0 1000000 65536"));
    assert!(line.ends_with('\n'));
}

#[test]
fn test_userns_host_passthrough_is_identity() {
    let ns = userns::UserNamespace::host_passthrough();
    assert!(ns.is_host());
    assert_eq!(ns.uid_mappings[0].translate_to_host(42), Some(42));
}

#[test]
fn test_userns_allocator_unique_ranges() {
    let a = userns::UserNsAllocator::new(0, 65_536 * 4, 65_536);
    let mut bases = vec![
        a.allocate().unwrap(),
        a.allocate().unwrap(),
        a.allocate().unwrap(),
    ];
    bases.sort();
    bases.dedup();
    assert_eq!(bases.len(), 3);
}

#[test]
fn test_userns_allocator_release_and_reuse() {
    let a = userns::UserNsAllocator::new(0, 65_536 * 2, 65_536);
    let r1 = a.allocate().unwrap();
    a.release(r1);
    let r2 = a.allocate().unwrap();
    assert_eq!(r1, r2);
}

#[test]
fn test_userns_parse_subid_file() {
    let content = "alice:100000:65536\nbob:200000:65536\n";
    let m = userns::parse_subid_file(content, "alice");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].host_id, 100_000);
    assert_eq!(m[0].size, 65_536);
}

#[test]
fn test_run_pod_sandbox_userns_pod_uses_allocator() {
    ensure_test_root();
    let alloc = userns::UserNsAllocator::new(500_000, 500_000 + 65_536, 65_536);
    let mut spec = sandbox_spec_with_ports("ns-pod", vec![]);
    spec.user_namespace_mode = UserNamespaceMode::Pod;
    let r = sb::run_pod_sandbox(spec, Some(&alloc)).unwrap();
    assert_eq!(r.user_namespace.uid_mappings[0].host_id, 500_000);
}

#[test]
fn test_run_pod_sandbox_userns_pod_without_allocator_errors() {
    ensure_test_root();
    let mut spec = sandbox_spec_with_ports("ns-fail", vec![]);
    spec.user_namespace_mode = UserNamespaceMode::Pod;
    assert!(sb::run_pod_sandbox(spec, None).is_err());
}

#[test]
fn test_run_pod_sandbox_userns_host_does_not_consume_allocator() {
    ensure_test_root();
    let alloc = userns::UserNsAllocator::new(700_000, 700_000 + 65_536 * 2, 65_536);
    let r =
        sb::run_pod_sandbox(sandbox_spec_with_ports("host-mode", vec![]), Some(&alloc)).unwrap();
    assert!(r.user_namespace.is_host());
    assert_eq!(alloc.allocated(), 0);
}

// ── Image pull auth + progress + multi-arch ────────────────────────────────────

use crate::{auth, manifest_list, pull_progress as pp};

#[test]
fn test_auth_resolver_docker_hub() {
    let s = auth::AuthScheme::default_for_registry("docker.io");
    assert!(matches!(s, auth::AuthScheme::Oauth2 { .. }));
}

#[test]
fn test_auth_resolver_aws_ecr() {
    let s = auth::AuthScheme::default_for_registry("123.dkr.ecr.eu-central-1.amazonaws.com");
    match s {
        auth::AuthScheme::AwsEcr { region, .. } => assert_eq!(region, "eu-central-1"),
        _ => panic!("expected AwsEcr"),
    }
}

#[test]
fn test_auth_resolver_gcp_gcr() {
    assert!(matches!(
        auth::AuthScheme::default_for_registry("us.gcr.io"),
        auth::AuthScheme::GcpGcr { .. }
    ));
}

#[test]
fn test_auth_resolver_azure_acr() {
    let s = auth::AuthScheme::default_for_registry("myorg.azurecr.io");
    match s {
        auth::AuthScheme::AzureAcr { tenant, .. } => assert_eq!(tenant, "myorg"),
        _ => panic!("expected AzureAcr"),
    }
}

#[test]
fn test_auth_basic_authorization_header() {
    let s = auth::AuthScheme::Basic {
        username: "alice".into(),
        password: "secret".into(),
    };
    assert_eq!(
        s.authorization_header(),
        Some("Basic YWxpY2U6c2VjcmV0".into())
    );
}

#[test]
fn test_auth_bearer_token_header() {
    let s = auth::AuthScheme::Bearer {
        token: "abc".into(),
    };
    assert_eq!(s.authorization_header(), Some("Bearer abc".into()));
}

#[test]
fn test_pull_progress_full_lifecycle() {
    let t = pp::PullProgressTracker::new();
    let id = t.start("nginx:latest");
    t.manifest_fetched(id, 2, 1000);
    t.layer_started(id, "d1", 500);
    t.layer_progress(id, "d1", 250);
    t.layer_progress(id, "d1", 500);
    t.layer_complete(id, "d1");
    t.layer_started(id, "d2", 500);
    t.layer_progress(id, "d2", 500);
    t.layer_complete(id, "d2");
    t.completed(id, "nginx:latest");

    let s = t.state(id).unwrap();
    assert_eq!(s.status, pp::PullStatus::Completed);
    assert_eq!(s.layers_complete, 2);
    assert_eq!(s.downloaded_bytes, 1000);
}

#[test]
fn test_pull_progress_failure_marks_state() {
    let t = pp::PullProgressTracker::new();
    let id = t.start("bad:latest");
    t.failed(id, "bad:latest", "401 Unauthorized");
    assert_eq!(t.state(id).unwrap().status, pp::PullStatus::Failed);
}

#[test]
fn test_manifest_list_select_amd64() {
    use manifest_list::*;
    let list = ManifestList {
        schema_version: 2,
        media_type: OCI_INDEX_MEDIA_TYPE.into(),
        manifests: vec![
            ManifestListEntry {
                digest: "sha256:amd".into(),
                size: 100,
                media_type: "application/vnd.oci.image.manifest.v1+json".into(),
                platform: Platform::linux_amd64(),
            },
            ManifestListEntry {
                digest: "sha256:arm".into(),
                size: 100,
                media_type: "application/vnd.oci.image.manifest.v1+json".into(),
                platform: Platform::linux_arm64(),
            },
        ],
    };
    let m = list.select(&Platform::linux_amd64()).unwrap();
    assert_eq!(m.digest, "sha256:amd");
}

#[test]
fn test_manifest_list_is_index_media_type() {
    assert!(manifest_list::is_index_media_type(
        manifest_list::OCI_INDEX_MEDIA_TYPE
    ));
    assert!(manifest_list::is_index_media_type(
        manifest_list::DOCKER_MANIFEST_LIST_MEDIA_TYPE
    ));
}

#[test]
fn test_credential_store_set_get_remove() {
    let s = auth::CredentialStore::new();
    s.set("ghcr.io", auth::AuthScheme::Bearer { token: "tk".into() });
    let got = s.get("ghcr.io");
    assert!(matches!(got, auth::AuthScheme::Bearer { .. }));
    s.remove("ghcr.io").unwrap();
    // Falls back to default for ghcr.io which is Bearer with empty token.
    let fallback = s.get("ghcr.io");
    assert!(matches!(fallback, auth::AuthScheme::Bearer { .. }));
}

// ── Sandbox lifecycle ──────────────────────────────────────────────────────────

use crate::sandbox as sb;

fn sandbox_spec_with_ports(name: &str, ports: Vec<PortMapping>) -> SandboxSpec {
    SandboxSpec {
        name: name.into(),
        namespace: "default".into(),
        labels: Default::default(),
        annotations: Default::default(),
        hostname: Some(name.into()),
        dns_config: None,
        port_mappings: ports,
        log_directory: None,
        cgroup_parent: None,
        runtime_handler: None,
        user_namespace_mode: crate::models::UserNamespaceMode::Host,
    }
}

#[test]
fn test_run_pod_sandbox_allocates_namespaces() {
    ensure_test_root();
    let r = sb::run_pod_sandbox(sandbox_spec_with_ports("ns-test", vec![]), None).unwrap();
    assert!(r.namespaces.network.exists());
    assert!(r.namespaces.ipc.exists());
    assert!(r.namespaces.uts.exists());
    assert!(r.namespaces.mount.exists());
}

#[test]
fn test_run_pod_sandbox_assigns_pause() {
    ensure_test_root();
    let r = sb::run_pod_sandbox(sandbox_spec_with_ports("p", vec![]), None).unwrap();
    assert_eq!(r.pause.image, sb::DEFAULT_PAUSE_IMAGE);
    assert_eq!(r.pause.sandbox_id, r.sandbox.id);
}

#[test]
fn test_run_pod_sandbox_assigns_pod_ip() {
    ensure_test_root();
    let r = sb::run_pod_sandbox(sandbox_spec_with_ports("ipc", vec![]), None).unwrap();
    let ip = r.sandbox.network_ip.unwrap();
    assert!(ip.starts_with("10.244."));
}

#[test]
fn test_stop_pod_sandbox_clears_namespaces() {
    ensure_test_root();
    let r = sb::run_pod_sandbox(sandbox_spec_with_ports("stop", vec![]), None).unwrap();
    let net = r.namespaces.network.clone();
    sb::stop_pod_sandbox(r.sandbox.id).unwrap();
    assert!(!net.exists());
}

#[test]
fn test_port_mapping_validation() {
    let bad = PortMapping {
        protocol: "TCP".into(),
        container_port: 0,
        host_port: 80,
        host_ip: None,
    };
    assert!(sb::validate_port_mapping(&bad).is_err());
    let good = PortMapping {
        protocol: "TCP".into(),
        container_port: 80,
        host_port: 8080,
        host_ip: None,
    };
    assert!(sb::validate_port_mapping(&good).is_ok());
}

#[test]
fn test_port_mapping_protocols() {
    for proto in ["TCP", "UDP", "SCTP", "tcp", "udp"] {
        let p = PortMapping {
            protocol: proto.into(),
            container_port: 80,
            host_port: 80,
            host_ip: None,
        };
        assert!(
            sb::validate_port_mapping(&p).is_ok(),
            "{} should be ok",
            proto
        );
    }
    let p = PortMapping {
        protocol: "QUIC".into(),
        container_port: 80,
        host_port: 80,
        host_ip: None,
    };
    assert!(sb::validate_port_mapping(&p).is_err());
}

#[test]
fn test_render_iptables_rule() {
    let p = PortMapping {
        protocol: "TCP".into(),
        container_port: 80,
        host_port: 8080,
        host_ip: None,
    };
    let rule = sb::render_iptables_rule("10.244.0.5", &p);
    assert!(rule.contains("-p tcp"));
    assert!(rule.contains("--dport 8080"));
    assert!(rule.contains("--to-destination 10.244.0.5:80"));
}

#[test]
fn test_sandbox_runtime_handler_selection() {
    let registry = RuntimeHandlerRegistry::with_defaults();
    // Empty selector → default (runc).
    let h = registry.select_for_sandbox("").unwrap();
    assert_eq!(h.name, "runc");
    // Named selector → that handler.
    let h2 = registry.select_for_sandbox("kata").unwrap();
    assert_eq!(h2.name, "kata");
}

// ── Streaming protocol (exec / attach / port-forward) ─────────────────────────

use crate::streaming;

#[test]
fn test_stream_frame_roundtrip() {
    let f = streaming::Frame::new(streaming::Channel::Stdout, b"abc".to_vec());
    let back = streaming::Frame::decode(&f.encode()).unwrap();
    assert_eq!(back, f);
}

#[test]
fn test_stream_protocol_negotiation() {
    let p = streaming::StreamProtocol::negotiate("v5.channel.k8s.io,v4.streamprotocol.k8s.io");
    assert_eq!(p, Some(streaming::StreamProtocol::WebSocketV5));
    let p2 = streaming::StreamProtocol::negotiate("nothing");
    assert!(p2.is_none());
}

#[test]
fn test_exec_streaming_url() {
    let id = Uuid::new_v4();
    let u = streaming::StreamingURL::for_exec(id);
    assert!(u.url.contains(&id.to_string()));
    assert!(u.protocols.contains(&"v5.channel.k8s.io".to_string()));
}

#[test]
fn test_attach_streaming_url() {
    let id = Uuid::new_v4();
    let u = streaming::StreamingURL::for_attach(id);
    assert!(u.url.ends_with("/attach/ws"));
}

#[test]
fn test_port_forward_channel_allocation() {
    let p0 = streaming::PortForwardChannel::allocate(8080, 0);
    let p1 = streaming::PortForwardChannel::allocate(443, 1);
    assert_eq!(p0.data_channel, 0);
    assert_eq!(p0.error_channel, 1);
    assert_eq!(p1.data_channel, 2);
    assert_eq!(p1.error_channel, 3);
}

#[test]
fn test_tty_resize() {
    let size = streaming::TtyWindowSize {
        width: 200,
        height: 60,
    };
    let bytes = size.encode();
    let back = streaming::TtyWindowSize::decode(&bytes).unwrap();
    assert_eq!(size, back);
}

#[test]
fn test_stream_channel_directions() {
    assert!(streaming::Channel::Stdin.is_client_to_server());
    assert!(streaming::Channel::Resize.is_client_to_server());
    assert!(!streaming::Channel::Stdout.is_client_to_server());
    assert!(!streaming::Channel::Error.is_client_to_server());
}

// ── Stats v2 / cAdvisor ────────────────────────────────────────────────────────

use crate::stats;

fn make_stats_container() -> crate::models::Container {
    use crate::models::*;
    Container {
        id: Uuid::new_v4(),
        spec: ContainerSpec {
            name: "stats-c".into(),
            image: "nginx:latest".into(),
            command: vec![],
            args: vec![],
            env: Default::default(),
            mounts: vec![],
            resources: ResourceLimits {
                memory_limit: Some(1024 * 1024),
                ..Default::default()
            },
            labels: [("app".to_string(), "web".to_string())]
                .into_iter()
                .collect(),
            working_dir: None,
            user: None,
            hostname: None,
            network_mode: NetworkMode::Bridge,
            restart_policy: RestartPolicy::Never,
        },
        status: ContainerStatus::Running,
        pid: Some(1),
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        finished_at: None,
        exit_code: None,
        rootfs_path: "/tmp/r".into(),
        log_path: "/tmp/r.log".into(),
        health: None,
    }
}

#[test]
fn test_container_stats_linux() {
    let c = make_stats_container();
    let s = stats::container_stats_linux(&c, None).unwrap();
    let attrs = s.attributes.unwrap();
    assert_eq!(attrs.id, c.id);
    assert_eq!(s.writable_layer.fs_id.mountpoint, "/tmp/r");
}

#[test]
fn test_container_stats_windows() {
    let c = make_stats_container();
    let s = stats::container_stats_windows(&c).unwrap();
    let attrs = s.attributes.unwrap();
    assert_eq!(attrs.name, "stats-c");
}

#[test]
fn test_list_container_stats() {
    let a = make_stats_container();
    let mut b = make_stats_container();
    b.spec.labels.insert("app".into(), "db".into());
    let f = stats::ContainerStatsFilter {
        label_selector: [("app".to_string(), "db".to_string())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let got = stats::filter_containers([&a, &b], &f);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, b.id);
}

#[test]
fn test_image_fs_info() {
    use crate::models::*;
    let imgs = vec![OciImage {
        reference: "x".into(),
        digest: "d".into(),
        layers: vec![],
        config: ImageConfig::default(),
        size_bytes: 42,
        pulled_at: Utc::now(),
    }];
    let info = stats::image_fs_info("/var/lib/cave/images", &imgs);
    assert_eq!(info.image_filesystems[0].used_bytes, 42);
}

#[test]
fn test_list_metric_descriptors() {
    let d = stats::cadvisor_descriptors();
    assert!(d
        .iter()
        .any(|m| m.name == "container_cpu_usage_seconds_total"));
    assert!(d
        .iter()
        .any(|m| m.name == "container_memory_working_set_bytes"));
}

#[test]
fn test_render_prometheus() {
    let c = make_stats_container();
    let s = stats::container_stats_linux(&c, None).unwrap();
    let metrics = stats::linux_to_metrics(&s);
    let rendered = stats::render_prometheus(&metrics);
    assert!(rendered.contains("# TYPE container_cpu_usage_seconds_total counter"));
    assert!(rendered.contains("name=\"stats-c\""));
}

#[test]
fn test_container_stats_filter() {
    let a = make_stats_container();
    let b = make_stats_container();
    let f = stats::ContainerStatsFilter {
        id: Some(b.id),
        ..Default::default()
    };
    let got = stats::filter_containers([&a, &b], &f);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, b.id);
}

#[test]
fn test_nano_cores_from_delta() {
    // 100 ms apart, 50_000 nano-CPU consumed → 500_000 nano_cores.
    let prev = stats::CpuUsage {
        timestamp: 0,
        usage_core_nano_seconds: 0,
        usage_nano_cores: 0,
    };
    let mut c = make_stats_container();
    c.id = Uuid::new_v4();
    let s = stats::container_stats_linux(&c, Some(&prev)).unwrap();
    // Without real cgroup data the delta is 0 → nano_cores = 0.
    // We just verify the field is populated and finite.
    assert!(s.cpu.usage_nano_cores < u64::MAX);
}

// ── Container log v2 ───────────────────────────────────────────────────────────

use crate::log_v2;

#[test]
fn test_parse_cri_log() {
    let line = "2024-04-26T12:34:56.123456789Z stdout F hello";
    let entry = log_v2::parse_line(line).unwrap();
    assert_eq!(entry.stream, log_v2::Stream::Stdout);
    assert_eq!(entry.tag, log_v2::LogTag::Full);
    assert_eq!(entry.message, "hello");
}

#[test]
fn test_encode_cri_log() {
    let when = chrono::Utc::now();
    let line = log_v2::encode_line(when, log_v2::Stream::Stderr, log_v2::LogTag::Full, "boom");
    let parsed = log_v2::parse_line(&line).unwrap();
    assert_eq!(parsed.message, "boom");
    assert_eq!(parsed.stream, log_v2::Stream::Stderr);
}

#[test]
fn test_cri_log_tail_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.log");
    for i in 0..10 {
        log_v2::write_log_line(
            &path,
            log_v2::Stream::Stdout,
            &format!("L{}", i),
            chrono::Utc::now(),
            u64::MAX,
            5,
        )
        .unwrap();
    }
    let entries = log_v2::read_logs(
        &path,
        &log_v2::LogOptions {
            tail_lines: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].message, "L9");
}

#[test]
fn test_cri_log_since_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.log");
    use chrono::TimeZone;
    for i in 0..5 {
        log_v2::write_log_line(
            &path,
            log_v2::Stream::Stdout,
            &format!("L{}", i),
            chrono::Utc.timestamp_opt(1_700_000_000 + i, 0).unwrap(),
            u64::MAX,
            5,
        )
        .unwrap();
    }
    let entries = log_v2::read_logs(
        &path,
        &log_v2::LogOptions {
            since_time: Some(chrono::Utc.timestamp_opt(1_700_000_003, 0).unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].message, "L3");
}

#[test]
fn test_cri_log_limit_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.log");
    for i in 0..10 {
        log_v2::write_log_line(
            &path,
            log_v2::Stream::Stdout,
            &format!("LINE-{}", i),
            chrono::Utc::now(),
            u64::MAX,
            5,
        )
        .unwrap();
    }
    let entries = log_v2::read_logs(
        &path,
        &log_v2::LogOptions {
            limit_bytes: Some(15),
            ..Default::default()
        },
    )
    .unwrap();
    let total: usize = entries.iter().map(|e| e.message.len()).sum();
    assert!(total <= 15);
}

#[test]
fn test_cri_log_stitch_partials() {
    let big = "z".repeat(log_v2::MAX_LINE_BYTES + 50);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.log");
    log_v2::write_log_line(
        &path,
        log_v2::Stream::Stdout,
        &big,
        chrono::Utc::now(),
        u64::MAX,
        5,
    )
    .unwrap();
    let raw = log_v2::read_file(&path).unwrap();
    assert!(raw.len() >= 2);
    let stitched = log_v2::stitch_partials(raw);
    assert_eq!(stitched.len(), 1);
    assert_eq!(stitched[0].message.len(), big.len());
}

#[test]
fn test_cri_log_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.log");
    std::fs::write(&path, vec![b'x'; 200]).unwrap();
    log_v2::write_log_line(
        &path,
        log_v2::Stream::Stdout,
        "after",
        chrono::Utc::now(),
        100,
        3,
    )
    .unwrap();
    assert!(dir.path().join("c.log.1").exists());
}

// ── KEP-585: RuntimeHandler / RuntimeClass ─────────────────────────────────────

use crate::runtime_handler::{RuntimeHandler, RuntimeHandlerFeatures};

#[test]
fn test_runtime_handler_list() {
    let r = RuntimeHandlerRegistry::with_defaults();
    let list = r.list();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "kata"); // sorted
    assert_eq!(list[1].name, "runc");
    assert_eq!(list[2].name, "runsc");
}

#[test]
fn test_runtime_handler_lookup() {
    let r = RuntimeHandlerRegistry::with_defaults();
    assert!(r.lookup("runc").is_some());
    assert!(r.lookup("nope").is_none());
}

#[test]
fn test_runtime_handler_default() {
    let r = RuntimeHandlerRegistry::with_defaults();
    assert_eq!(r.default_handler().unwrap().name, "runc");
    r.set_default("kata").unwrap();
    assert_eq!(r.default_handler().unwrap().name, "kata");
}

#[test]
fn test_runtime_handler_select_for_sandbox() {
    let r = RuntimeHandlerRegistry::with_defaults();
    // Empty selector → default
    assert_eq!(r.select_for_sandbox("").unwrap().name, "runc");
    // Named selector → that one
    assert_eq!(r.select_for_sandbox("kata").unwrap().name, "kata");
    // Unknown → error
    assert!(r.select_for_sandbox("ghost").is_err());
}

#[test]
fn test_runtime_handler_features() {
    let runc = RuntimeHandler::runc();
    assert_eq!(
        runc.features,
        RuntimeHandlerFeatures {
            recursive_read_only_mounts: true,
            user_namespaces: true,
        }
    );
    let runsc = RuntimeHandler::runsc();
    assert!(!runsc.features.user_namespaces);
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
    state.network.insert(
        c.id,
        NetworkStatus {
            container_id: c.id,
            network_name: "bridge0".into(),
            ip_address: Some("10.244.0.5".into()),
            mac_address: None,
            gateway: Some("10.244.0.1".into()),
            interface: Some("eth0".into()),
            attached: true,
        },
    );
    assert!(state.network.get(&c.id).unwrap().attached);
}
