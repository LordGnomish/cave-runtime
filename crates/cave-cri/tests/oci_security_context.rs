// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! deeper-002: real security-context injection into the OCI runtime spec.
//!
//! Upstream: containerd v2.2.3
//! `pkg/cri/server/container_create_linux.go::setOCISecurityContext` and
//! runc v1.4.2 `libcontainer/specconv/spec_linux.go`.

use cave_cri::models::{
    ContainerSpec, NetworkMode, ResourceLimits, RestartPolicy, SeccompProfile, SecurityContext,
};
use cave_cri::oci_spec::{apply_security_context, generate};
use std::path::PathBuf;

const TENANT: &str = "tenant-acme-prod";

fn base_spec() -> ContainerSpec {
    ContainerSpec {
        name: format!("{}-app", TENANT),
        image: "ghcr.io/org/app:v1".into(),
        command: vec!["/bin/app".into()],
        args: vec![],
        env: Default::default(),
        mounts: vec![],
        resources: ResourceLimits::default(),
        labels: [("tenant_id".into(), TENANT.into())].into(),
        working_dir: Some("/app".into()),
        user: None,
        hostname: Some("app".into()),
        network_mode: NetworkMode::Bridge,
        restart_policy: RestartPolicy::Never,
    }
}

/// Cite: containerd v2.2.3 `setOCISecurityContext` — `RunAsUser` /
/// `RunAsGroup` / `SupplementalGroups` map directly onto
/// `process.user.{uid,gid,additionalGids}` in the OCI spec.
#[test]
fn run_as_user_group_supplemental_groups_applied() {
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-001");
    apply_security_context(&mut spec, &SecurityContext {
        run_as_user: Some(1000),
        run_as_group: Some(1001),
        supplemental_groups: vec![100, 200, 300],
        ..Default::default()
    });
    assert_eq!(spec.process.user.uid, 1000);
    assert_eq!(spec.process.user.gid, 1001);
    assert_eq!(spec.process.user.additional_gids, vec![100, 200, 300]);
}

/// Cite: containerd `setOCISecurityContext` + runc v1.4.2
/// `libcontainer/specconv/spec_linux.go` — `ReadOnlyRootFilesystem` ⇒
/// `root.readonly = true`. `AllowPrivilegeEscalation = false` ⇒
/// `process.no_new_privileges = true`.
#[test]
fn readonly_rootfs_and_no_new_privs_flags_propagate() {
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-002");
    apply_security_context(&mut spec, &SecurityContext {
        readonly_rootfs: true,
        allow_privilege_escalation: false,
        ..Default::default()
    });
    assert!(spec.root.readonly);
    assert!(spec.process.no_new_privileges);

    // Explicit allow → no_new_privileges = false (matches Kubernetes default).
    apply_security_context(&mut spec, &SecurityContext {
        readonly_rootfs: false,
        allow_privilege_escalation: true,
        ..Default::default()
    });
    assert!(!spec.root.readonly);
    assert!(!spec.process.no_new_privileges);
}

/// Cite: containerd `setOCISecurityContext` capability translation —
/// `Add` extends the container default set; `Drop` removes from it.
/// `Drop` applies AFTER `Add` so an Add+Drop on the same capability
/// results in a drop.
#[test]
fn capabilities_add_then_drop_yields_drop() {
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-003");
    apply_security_context(&mut spec, &SecurityContext {
        capabilities_add: vec!["NET_ADMIN".into(), "SYS_PTRACE".into()],
        capabilities_drop: vec!["SYS_PTRACE".into(), "MKNOD".into()],
        ..Default::default()
    });
    let caps = &spec.process.capabilities.bounding;
    assert!(caps.contains(&"CAP_NET_ADMIN".to_string()), "Add applied");
    assert!(!caps.contains(&"CAP_SYS_PTRACE".to_string()), "Drop overrides Add");
    assert!(!caps.contains(&"CAP_MKNOD".to_string()), "Default cap dropped");
    // Drops must apply uniformly to all four cap sets that runc sets.
    assert_eq!(spec.process.capabilities.effective, *caps);
    assert_eq!(spec.process.capabilities.permitted, *caps);
}

/// Cite: containerd `setOCISecurityContext` — `Add: ["ALL"]` expands to
/// the full kernel capability set; `Drop: ["ALL"]` clears it.
#[test]
fn capabilities_add_all_expands_drop_all_clears() {
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-004");
    apply_security_context(&mut spec, &SecurityContext {
        capabilities_add: vec!["ALL".into()],
        ..Default::default()
    });
    assert!(spec.process.capabilities.bounding.len() >= 30,
        "ALL expands to ~40 caps, got {}", spec.process.capabilities.bounding.len());
    assert!(spec.process.capabilities.bounding.contains(&"CAP_SYS_ADMIN".to_string()));

    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-005");
    apply_security_context(&mut spec, &SecurityContext {
        capabilities_drop: vec!["ALL".into()],
        ..Default::default()
    });
    assert!(spec.process.capabilities.bounding.is_empty(),
        "ALL clears every cap");
}

/// Cite: containerd `setOCISecurityContext` privileged path — `Privileged
/// = true` grants every capability, clears masked / readonly paths and
/// disables seccomp. cave mirrors this verbatim.
#[test]
fn privileged_grants_all_caps_and_disables_confinement() {
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-006");
    // Pre-condition: default spec has masked paths + seccomp set
    assert!(!spec.linux.masked_paths.is_empty(), "default masked_paths populated");
    assert!(spec.linux.seccomp.is_some(), "default seccomp populated");

    apply_security_context(&mut spec, &SecurityContext {
        privileged: true,
        ..Default::default()
    });

    assert!(spec.process.capabilities.bounding.contains(&"CAP_SYS_ADMIN".to_string()));
    assert!(spec.process.capabilities.ambient.contains(&"CAP_SYS_ADMIN".to_string()),
        "ambient set granted in privileged mode");
    assert!(spec.linux.masked_paths.is_empty(), "privileged ⇒ no masked paths");
    assert!(spec.linux.readonly_paths.is_empty(), "privileged ⇒ no read-only paths");
    assert!(spec.linux.seccomp.is_none(), "privileged ⇒ seccomp disabled");
}

/// Cite: containerd `setOCISecurityContext` seccomp profile mapping —
/// `Unconfined` clears the seccomp filter; `RuntimeDefault` keeps the
/// runtime-default filter in place; `Localhost(path)` keeps the filter
/// (loaded later by the runtime).
#[test]
fn seccomp_profile_dispatch() {
    // Unconfined → no filter
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-007");
    assert!(spec.linux.seccomp.is_some(), "default starts populated");
    apply_security_context(&mut spec, &SecurityContext {
        seccomp_profile: Some(SeccompProfile::Unconfined),
        ..Default::default()
    });
    assert!(spec.linux.seccomp.is_none());

    // RuntimeDefault → filter stays / is restored
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-008");
    apply_security_context(&mut spec, &SecurityContext {
        seccomp_profile: Some(SeccompProfile::RuntimeDefault),
        ..Default::default()
    });
    assert!(spec.linux.seccomp.is_some());

    // Localhost(profile.json) → filter populated (runtime loads from path)
    let mut spec = generate(&base_spec(), &PathBuf::from("/merged"), "ctr-009");
    apply_security_context(&mut spec, &SecurityContext {
        seccomp_profile: Some(SeccompProfile::Localhost("/etc/seccomp/strict.json".into())),
        ..Default::default()
    });
    assert!(spec.linux.seccomp.is_some(),
        "Localhost profile ⇒ generator does NOT fall back to no-seccomp");
}
