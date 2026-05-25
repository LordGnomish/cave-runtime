// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge coverage for cave-cri — error, store, ImageReference, models, lease
//! resource kinds.

use cave_cri::error::{CriError, CriResult};
use cave_cri::leases::resource::{Resource, ResourceKind};
use cave_cri::models::{
    Container, ContainerSpec, ContainerStatus, ImageReference, NetworkMode, OciImage, OciLayer,
    ImageConfig, ResourceLimits, RestartPolicy, Sandbox, SandboxSpec, SandboxState,
    UserNamespaceMode,
};
use cave_cri::store::{ContainerStore, ImageStore, SandboxStore};
use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

fn make_container(name: &str) -> Container {
    Container {
        id: Uuid::new_v4(),
        spec: ContainerSpec {
            name: name.into(),
            image: "nginx:latest".into(),
            command: vec![],
            args: vec![],
            env: HashMap::new(),
            mounts: vec![],
            resources: ResourceLimits::default(),
            labels: HashMap::new(),
            working_dir: None,
            user: None,
            hostname: None,
            network_mode: NetworkMode::default(),
            restart_policy: RestartPolicy::default(),
        },
        status: ContainerStatus::Created,
        pid: None,
        created_at: Utc::now(),
        started_at: None,
        finished_at: None,
        exit_code: None,
        rootfs_path: PathBuf::from("/var/lib/cave/rootfs"),
        log_path: PathBuf::from("/var/log/cave/container.log"),
        health: None,
    }
}

fn make_image(reference: &str) -> OciImage {
    OciImage {
        reference: reference.into(),
        digest: "sha256:abcdef".into(),
        layers: vec![OciLayer {
            digest: "sha256:layer1".into(),
            size: 1024,
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
            local_path: None,
        }],
        config: ImageConfig::default(),
        size_bytes: 1024,
        pulled_at: Utc::now(),
    }
}

fn make_sandbox(name: &str) -> Sandbox {
    Sandbox {
        id: Uuid::new_v4(),
        spec: SandboxSpec {
            name: name.into(),
            namespace: "default".into(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            hostname: None,
            dns_config: None,
            port_mappings: vec![],
            log_directory: None,
            cgroup_parent: None,
            runtime_handler: None,
            user_namespace_mode: UserNamespaceMode::default(),
        },
        state: SandboxState::Ready,
        created_at: Utc::now(),
        network_ip: None,
    }
}

// ---------------------------------------------------------------------------
// CriError
// ---------------------------------------------------------------------------

#[test]
fn cri_error_display_includes_context() {
    assert!(CriError::NotFound("abc".into()).to_string().contains("abc"));
    assert!(CriError::InvalidState("paused".into()).to_string().contains("paused"));
    assert!(CriError::Namespace("netns".into()).to_string().contains("netns"));
    assert!(CriError::Cgroup("memory.limit".into()).to_string().contains("memory.limit"));
    assert!(CriError::Registry("404".into()).to_string().contains("404"));
    assert!(CriError::Rootfs("ENOSPC".into()).to_string().contains("ENOSPC"));
    assert!(CriError::Runtime("oom".into()).to_string().contains("oom"));
    assert!(CriError::Image("digest mismatch".into()).to_string().contains("digest mismatch"));
    assert!(CriError::Sandbox("uid clash".into()).to_string().contains("uid clash"));
    assert!(CriError::Snapshot("locked".into()).to_string().contains("locked"));
    assert!(CriError::Network("CNI failed".into()).to_string().contains("CNI failed"));
    assert!(CriError::Exec("nonzero".into()).to_string().contains("nonzero"));
}

#[test]
fn cri_error_io_wrapping() {
    let io_err = std::io::Error::other("disk full");
    let err: CriError = io_err.into();
    assert!(matches!(err, CriError::Io(_)));
    assert!(err.to_string().contains("disk full"));
}

#[test]
fn cri_result_ok_and_err() {
    fn ok() -> CriResult<i32> { Ok(1) }
    fn bad() -> CriResult<i32> { Err(CriError::NotFound("x".into())) }
    assert_eq!(ok().unwrap(), 1);
    assert!(bad().is_err());
}

// ---------------------------------------------------------------------------
// ContainerStore
// ---------------------------------------------------------------------------

#[test]
fn container_store_insert_get_remove() {
    let s = ContainerStore::new();
    let c = make_container("nginx");
    let id = c.id;
    s.insert(c.clone());
    assert_eq!(s.count(), 1);
    assert_eq!(s.get(&id).unwrap().spec.name, "nginx");
    s.remove(&id);
    assert!(s.get(&id).is_none());
    assert_eq!(s.count(), 0);
}

#[test]
fn container_store_get_unknown_returns_none() {
    let s = ContainerStore::new();
    assert!(s.get(&Uuid::new_v4()).is_none());
}

#[test]
fn container_store_update_replaces_in_place() {
    let s = ContainerStore::new();
    let mut c = make_container("app");
    s.insert(c.clone());
    c.status = ContainerStatus::Running;
    s.update(c.clone());
    let got = s.get(&c.id).unwrap();
    assert_eq!(got.status, ContainerStatus::Running);
}

#[test]
fn container_store_list_returns_all() {
    let s = ContainerStore::new();
    for n in &["a", "b", "c"] {
        s.insert(make_container(n));
    }
    let all = s.list();
    assert_eq!(all.len(), 3);
}

#[test]
fn container_store_default_equiv_new() {
    let a = ContainerStore::default();
    let b = ContainerStore::new();
    assert_eq!(a.count(), b.count());
}

#[test]
fn container_store_remove_returns_container() {
    let s = ContainerStore::new();
    let c = make_container("x");
    let id = c.id;
    s.insert(c);
    let removed = s.remove(&id).unwrap();
    assert_eq!(removed.id, id);
    assert!(s.remove(&id).is_none(), "second remove returns None");
}

// ---------------------------------------------------------------------------
// ImageStore
// ---------------------------------------------------------------------------

#[test]
fn image_store_insert_get_remove() {
    let s = ImageStore::new();
    s.insert(make_image("nginx:latest"));
    assert!(s.get("nginx:latest").is_some());
    let removed = s.remove("nginx:latest").unwrap();
    assert_eq!(removed.reference, "nginx:latest");
    assert!(s.get("nginx:latest").is_none());
}

#[test]
fn image_store_list_returns_all_images() {
    let s = ImageStore::new();
    s.insert(make_image("a"));
    s.insert(make_image("b"));
    assert_eq!(s.list().len(), 2);
}

// ---------------------------------------------------------------------------
// SandboxStore
// ---------------------------------------------------------------------------

#[test]
fn sandbox_store_insert_get_list() {
    let s = SandboxStore::new();
    let sb = make_sandbox("pod1");
    let id = sb.id;
    s.insert(sb);
    assert!(s.get(&id).is_some());
    assert_eq!(s.list().len(), 1);
}

// ---------------------------------------------------------------------------
// ImageReference::parse
// ---------------------------------------------------------------------------

#[test]
fn image_reference_parse_bare_name_defaults_to_docker_io_library() {
    let r = ImageReference::parse("nginx");
    assert_eq!(r.registry, "docker.io");
    assert_eq!(r.repository, "library/nginx");
    assert!(r.tag.is_none());
    assert!(r.digest.is_none());
}

#[test]
fn image_reference_parse_with_tag() {
    let r = ImageReference::parse("nginx:1.25");
    assert_eq!(r.registry, "docker.io");
    assert_eq!(r.repository, "library/nginx");
    assert_eq!(r.tag.as_deref(), Some("1.25"));
}

#[test]
fn image_reference_parse_with_namespace() {
    let r = ImageReference::parse("library/nginx:latest");
    assert_eq!(r.registry, "docker.io");
    assert_eq!(r.repository, "library/nginx");
    assert_eq!(r.tag.as_deref(), Some("latest"));
}

#[test]
fn image_reference_parse_with_full_registry() {
    let r = ImageReference::parse("ghcr.io/owner/repo:v1");
    assert_eq!(r.registry, "ghcr.io");
    assert_eq!(r.repository, "owner/repo");
    assert_eq!(r.tag.as_deref(), Some("v1"));
}

#[test]
fn image_reference_parse_with_digest() {
    let r = ImageReference::parse("nginx@sha256:abc123");
    assert_eq!(r.repository, "library/nginx");
    assert_eq!(r.digest.as_deref(), Some("sha256:abc123"));
    assert!(r.tag.is_none());
}

#[test]
fn image_reference_parse_with_tag_and_digest() {
    let r = ImageReference::parse("nginx:1.25@sha256:abc");
    assert_eq!(r.tag.as_deref(), Some("1.25"));
    assert_eq!(r.digest.as_deref(), Some("sha256:abc"));
}

#[test]
fn image_reference_parse_localhost_is_registry() {
    let r = ImageReference::parse("localhost/mylib/myimg:v2");
    assert_eq!(r.registry, "localhost");
    assert_eq!(r.repository, "mylib/myimg");
    assert_eq!(r.tag.as_deref(), Some("v2"));
}

#[test]
fn image_reference_full_reference_roundtrip() {
    let original = "ghcr.io/o/r:v1";
    let r = ImageReference::parse(original);
    assert_eq!(r.full_reference(), original);
}

#[test]
fn image_reference_full_reference_appends_digest() {
    let r = ImageReference::parse("nginx@sha256:abc");
    let full = r.full_reference();
    assert!(full.contains("@sha256:abc"));
}

// ---------------------------------------------------------------------------
// Lease resource kinds
// ---------------------------------------------------------------------------

#[test]
fn resource_kind_as_str_stable() {
    assert_eq!(ResourceKind::Content.as_str(), "content");
    assert_eq!(ResourceKind::Snapshot.as_str(), "snapshot");
    assert_eq!(ResourceKind::Ingest.as_str(), "ingest");
}

#[test]
fn resource_snapshot_carries_id() {
    let r = Resource::snapshot("snap-7");
    assert_eq!(r.kind, ResourceKind::Snapshot);
    assert_eq!(r.id, "snap-7");
    assert!(r.content_digest().is_none());
}

#[test]
fn resource_ingest_carries_reference() {
    let r = Resource::ingest("pull-1");
    assert_eq!(r.kind, ResourceKind::Ingest);
    assert!(r.content_digest().is_none());
}

// ---------------------------------------------------------------------------
// ContainerStatus / NetworkMode / RestartPolicy serde + Default
// ---------------------------------------------------------------------------

#[test]
fn container_status_failed_carries_reason() {
    let s = ContainerStatus::Failed("OOM".into());
    let j = serde_json::to_string(&s).unwrap();
    assert!(j.contains("OOM"));
}

#[test]
fn network_mode_default_is_bridge() {
    assert!(matches!(NetworkMode::default(), NetworkMode::Bridge));
}

#[test]
fn restart_policy_default_is_never() {
    assert!(matches!(RestartPolicy::default(), RestartPolicy::Never));
}

#[test]
fn user_namespace_mode_default_is_host() {
    assert_eq!(UserNamespaceMode::default(), UserNamespaceMode::Host);
}

#[test]
fn sandbox_state_serializes_as_string_variant() {
    assert_eq!(serde_json::to_string(&SandboxState::Ready).unwrap(), "\"Ready\"");
    assert_eq!(serde_json::to_string(&SandboxState::NotReady).unwrap(), "\"NotReady\"");
}
