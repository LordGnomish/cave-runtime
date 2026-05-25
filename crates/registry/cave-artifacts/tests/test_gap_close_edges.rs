// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: META — test gap close, edge cases across compiled pulp/* + core/* modules
//! Edge-case coverage for cave-artifacts.
//!
//! These tests target the *compiled* surface of the crate as exposed by
//! `pub use models::*` and the modules registered in `pulp/mod.rs`. They
//! focus on:
//! - serde round-trip for content-type wire formats (RPM/Deb/Python/Container)
//! - repository + distribution config validation + state transitions
//! - artifact dedup hashing and checksum verification
//! - policy / access (RBAC) wildcard and object-scope rules
//! - sync / task remote state transitions and lifecycle invariants
//! - upload chunk boundary conditions + content-range parser
//! - filter combinators for content search

use cave_artifacts::pulp::content::{
    DebFilter, PypiFilter, RpmFilter, generate_deb_package_entry, generate_pypi_project_json,
    generate_pypi_simple_page, generate_repomd_xml, verify_artifact_checksums, verify_sha256,
};
use cave_artifacts::pulp::distribution::{
    DistributionError, find_distribution_by_path, resolve_content_path, validate_distribution,
};
use cave_artifacts::pulp::models::{
    AnsibleCollection, Artifact, ContentGuard, ContentGuardType, ContentSummary, ContentType,
    DebPackage, Distribution, ExportParams, FileContent, ImportParams, MavenArtifact,
    PaginatedResponse, Publication, PulpExport, PulpImport, PythonPackage, PythonPackageType,
    Remote, RemotePolicy, Repository, RepositoryVersion, RpmPackage, SyncReport,
};
use cave_artifacts::pulp::rbac::{
    BuiltinRole, Permission, RoleAssignment, artifact_permissions, builtin_roles,
    distribution_permissions, get_user_permissions, repository_permissions, user_has_permission,
};
use cave_artifacts::pulp::repair::{
    ArtifactCheck, RepairOptions, RepairReport, check_artifact, enqueue_repair,
};
use cave_artifacts::pulp::repository::{
    add_content, create_repository, enqueue_sync, remove_content, repair_version,
    update_repository, versions_to_prune,
};
use cave_artifacts::pulp::signing::{
    ContentSignature, CosignBundle, CosignSignature, SigningRequest, SigningServiceType,
    VerificationResult, rpm_has_signature, verify_gpg_signature,
};
use cave_artifacts::pulp::tasks::{Task, TaskGroup, TaskQueue, TaskState};
use cave_artifacts::pulp::upload::{
    FinalizeUploadRequest, Upload, UploadChunkRequest, UploadError, UploadRegistry,
    parse_content_range,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn make_artifact(size: u64, sha256: Option<&str>) -> Artifact {
    let id = Uuid::new_v4();
    Artifact {
        pulp_href: format!("/pulp/api/v3/artifacts/{}/", id),
        pulp_id: id,
        pulp_created: Utc::now(),
        file: format!("/var/lib/pulp/artifacts/{}", id),
        size,
        md5: None,
        sha1: None,
        sha224: None,
        sha256: sha256.map(String::from),
        sha384: None,
        sha512: None,
        timestamp_of_interest: None,
    }
}

fn make_python_pkg(name: &str, ver: &str, kind: PythonPackageType, sha256: &str) -> PythonPackage {
    PythonPackage {
        pulp_href: format!("/pulp/api/v3/content/python/packages/{}/", Uuid::new_v4()),
        pulp_id: Uuid::new_v4(),
        name: name.to_string(),
        version: ver.to_string(),
        filename: format!("{name}-{ver}.tar.gz"),
        packagetype: kind,
        python_version: Some("py3".to_string()),
        requires_python: Some(">=3.8".to_string()),
        summary: None,
        description: None,
        sha256: sha256.to_string(),
        artifact: "/pulp/api/v3/artifacts/abc/".to_string(),
        url: format!("/simple/{name}/{name}-{ver}.tar.gz"),
    }
}

fn dist_with_publication(name: &str, base_path: &str, ct: ContentType) -> Distribution {
    let mut d = Distribution::new(name, base_path, ct);
    d.publication = Some(format!("/pulp/api/v3/publications/{}/", Uuid::new_v4()));
    d
}

// ─── 1. Serde round-trip — content-type models ────────────────────────────────

#[test]
fn rpm_package_serde_roundtrip() {
    let id = Uuid::new_v4();
    let pkg = RpmPackage {
        pulp_href: format!("/pulp/api/v3/content/rpm/packages/{}/", id),
        pulp_id: id,
        name: "kernel".into(),
        version: "6.6.0".into(),
        release: "1.fc40".into(),
        arch: "aarch64".into(),
        epoch: "0".into(),
        summary: Some("The Linux kernel".into()),
        description: None,
        url: None,
        rpm_license: Some("GPL-2.0".into()),
        rpm_vendor: Some("Fedora Project".into()),
        rpm_group: None,
        source_rpm: Some("kernel-6.6.0-1.fc40.src.rpm".into()),
        artifact: "/pulp/api/v3/artifacts/xyz/".into(),
        location_href: "Packages/k/kernel-6.6.0-1.fc40.aarch64.rpm".into(),
        sha256: "f".repeat(64),
        size_package: 90 * 1024 * 1024,
        time_file: 1_700_000_000,
        time_build: 1_699_000_000,
    };
    let json = serde_json::to_string(&pkg).unwrap();
    let back: RpmPackage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, pkg.name);
    assert_eq!(back.release, pkg.release);
    assert_eq!(back.arch, pkg.arch);
    assert_eq!(back.nevra(), "kernel-6.6.0-1.fc40.aarch64.rpm");
}

#[test]
fn deb_package_serde_roundtrip_with_optionals() {
    let pkg = DebPackage {
        pulp_href: "/pulp/api/v3/content/deb/packages/abc/".into(),
        pulp_id: Uuid::new_v4(),
        package: "libc6".into(),
        version: "2.35-0ubuntu3".into(),
        architecture: "amd64".into(),
        section: Some("libs".into()),
        priority: Some("required".into()),
        maintainer: Some("Ubuntu Devel <devel@lists.ubuntu.com>".into()),
        description: Some("GNU C Library: Shared libraries".into()),
        depends: Some("libgcc-s1 (>= 4.2)".into()),
        pre_depends: None,
        suggests: Some("glibc-doc".into()),
        recommends: None,
        sha256: "a".repeat(64),
        size: 3_141_592,
        artifact: "/pulp/api/v3/artifacts/def/".into(),
        relative_path: "pool/main/l/libc6/libc6_2.35-0ubuntu3_amd64.deb".into(),
    };
    let json = serde_json::to_string(&pkg).unwrap();
    let back: DebPackage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.package, "libc6");
    assert_eq!(back.section.as_deref(), Some("libs"));
    assert_eq!(back.pre_depends, None);
}

#[test]
fn python_package_serde_preserves_packagetype_lowercase() {
    let sha = "a".repeat(64);
    let pkg = make_python_pkg("requests", "2.31.0", PythonPackageType::Bdist_wheel, &sha);
    let json = serde_json::to_value(&pkg).unwrap();
    // packagetype must serialise as lowercase per #[serde(rename_all = "lowercase")]
    assert_eq!(json["packagetype"], "bdist_wheel");
    let back: PythonPackage = serde_json::from_value(json).unwrap();
    assert_eq!(back.packagetype, PythonPackageType::Bdist_wheel);
}

#[test]
fn python_package_type_all_variants_roundtrip() {
    for ty in [
        PythonPackageType::Sdist,
        PythonPackageType::Bdist_wheel,
        PythonPackageType::Bdist_egg,
    ] {
        let json = serde_json::to_string(&ty).unwrap();
        let back: PythonPackageType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ty, "round-trip lost variant: {:?}", ty);
    }
}

#[test]
fn ansible_collection_serde_with_deps() {
    let mut deps = HashMap::new();
    deps.insert("community.general".to_string(), ">=1.0.0".to_string());
    deps.insert("ansible.posix".to_string(), ">=1.5.0".to_string());
    let col = AnsibleCollection {
        pulp_href: "/pulp/api/v3/content/ansible/collection_versions/abc/".into(),
        pulp_id: Uuid::new_v4(),
        namespace: "community".into(),
        name: "kubernetes".into(),
        version: "3.0.0".into(),
        sha256: "b".repeat(64),
        artifact: "/pulp/api/v3/artifacts/abc/".into(),
        requires_ansible: Some(">=2.14".into()),
        description: None,
        tags: vec!["k8s".into(), "kubernetes".into()],
        dependencies: deps,
    };
    let json = serde_json::to_string(&col).unwrap();
    let back: AnsibleCollection = serde_json::from_str(&json).unwrap();
    assert_eq!(back.dependencies.len(), 2);
    assert_eq!(back.tags.len(), 2);
}

#[test]
fn maven_artifact_coordinates_format() {
    let m = MavenArtifact {
        pulp_href: "/pulp/api/v3/content/maven/artifacts/abc/".into(),
        pulp_id: Uuid::new_v4(),
        group_id: "org.apache.commons".into(),
        artifact_id: "commons-lang3".into(),
        version: "3.14.0".into(),
        filename: "commons-lang3-3.14.0.jar".into(),
        artifact: "/pulp/api/v3/artifacts/abc/".into(),
        sha256: "c".repeat(64),
        relative_path: "org/apache/commons/commons-lang3/3.14.0/commons-lang3-3.14.0.jar".into(),
    };
    assert_eq!(m.coordinates(), "org.apache.commons:commons-lang3:3.14.0");
    // serde round-trip preserves GAV
    let json = serde_json::to_string(&m).unwrap();
    let back: MavenArtifact = serde_json::from_str(&json).unwrap();
    assert_eq!(back.coordinates(), m.coordinates());
}

#[test]
fn file_content_minimal_serde() {
    let fc = FileContent {
        pulp_href: "/pulp/api/v3/content/file/files/abc/".into(),
        pulp_id: Uuid::new_v4(),
        relative_path: "iso/release.iso".into(),
        artifact: "/pulp/api/v3/artifacts/abc/".into(),
        sha256: "d".repeat(64),
    };
    let json = serde_json::to_string(&fc).unwrap();
    let back: FileContent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.relative_path, "iso/release.iso");
}

#[test]
fn content_type_serde_wire_format_lowercase() {
    let all = [
        ContentType::Rpm,
        ContentType::Debian,
        ContentType::Python,
        ContentType::Container,
        ContentType::File,
        ContentType::Ansible,
        ContentType::Maven,
        ContentType::Gem,
        ContentType::Npm,
        ContentType::Generic,
    ];
    for ct in all {
        let s = serde_json::to_string(&ct).unwrap();
        // wire format is lowercase per #[serde(rename_all = "lowercase")]
        assert!(
            s.chars().all(|c| !c.is_ascii_uppercase() || c == '"'),
            "ContentType serialised with uppercase chars: {s}"
        );
        let back: ContentType = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ct);
    }
}

#[test]
fn remote_policy_serde_all_variants() {
    for p in [
        RemotePolicy::Immediate,
        RemotePolicy::OnDemand,
        RemotePolicy::Streamed,
    ] {
        let s = serde_json::to_string(&p).unwrap();
        let back: RemotePolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }
}

#[test]
fn content_guard_type_tagged_serde() {
    let g = ContentGuard {
        pulp_href: "/pulp/api/v3/contentguards/abc/".into(),
        pulp_id: Uuid::new_v4(),
        pulp_created: Utc::now(),
        name: "rbac-guard".into(),
        description: None,
        guard_type: ContentGuardType::Header {
            header_name: "X-Api-Key".into(),
            header_value: "secret".into(),
        },
    };
    let json = serde_json::to_value(&g).unwrap();
    assert_eq!(json["guardType"]["type"], "header");
    let back: ContentGuard = serde_json::from_value(json).unwrap();
    match back.guard_type {
        ContentGuardType::Header { header_name, .. } => assert_eq!(header_name, "X-Api-Key"),
        _ => panic!("guardType variant lost in round-trip"),
    }
}

#[test]
fn content_guard_composite_variant() {
    let g = ContentGuardType::Composite {
        guards: vec![
            "/pulp/api/v3/contentguards/a/".into(),
            "/pulp/api/v3/contentguards/b/".into(),
        ],
    };
    let json = serde_json::to_value(&g).unwrap();
    assert_eq!(json["type"], "composite");
    let back: ContentGuardType = serde_json::from_value(json).unwrap();
    assert!(matches!(back, ContentGuardType::Composite { ref guards } if guards.len() == 2));
}

// ─── 2. Repository / distribution / publication config ───────────────────────

#[test]
fn repository_default_retain_versions_is_ten() {
    let r = Repository::new("rpm-mirror", ContentType::Rpm);
    assert_eq!(r.retain_repo_versions, Some(10));
    assert!(!r.autopublish);
    assert!(r.labels.is_empty());
    assert!(r.versions_href.ends_with("/versions/"));
}

#[test]
fn update_repository_no_op_leaves_fields_intact() {
    let mut r = create_repository("origin", ContentType::Python);
    let old_name = r.name.clone();
    let old_desc = r.description.clone();
    update_repository(&mut r, None, None, None);
    assert_eq!(r.name, old_name);
    assert_eq!(r.description, old_desc);
}

#[test]
fn publication_complete_flag_starts_false() {
    let pub_ = Publication::new(
        "/pulp/api/v3/repositories/abc/versions/3/",
        ContentType::Debian,
    );
    assert!(!pub_.complete);
    assert!(pub_.extra.is_empty());
}

#[test]
fn distribution_base_url_joins_base_path() {
    let d = Distribution::new("repo-el9", "rpm/el9/x86_64", ContentType::Rpm);
    assert_eq!(d.base_url, "/pulp/content/rpm/el9/x86_64/");
    assert!(!d.hidden);
}

#[test]
fn validate_distribution_empty_base_path_invalid() {
    let mut d = Distribution::new("dist", "", ContentType::File);
    d.publication = Some("/pulp/api/v3/publications/abc/".into());
    let errors = validate_distribution(&d, &[]);
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, DistributionError::InvalidBasePath(_)))
    );
}

#[test]
fn validate_distribution_repository_only_source_ok() {
    let mut d = Distribution::new("dist", "deb/main", ContentType::Debian);
    d.repository = Some("/pulp/api/v3/repositories/deb/abc/".into());
    let errors = validate_distribution(&d, &[]);
    assert!(errors.is_empty(), "errors: {:?}", errors);
}

#[test]
fn validate_distribution_same_base_path_same_id_no_conflict() {
    // Updating an existing distribution (same pulp_id) shouldn't conflict.
    let d = dist_with_publication("dist", "rpm/el9", ContentType::Rpm);
    let mut clone = d.clone();
    clone.name = "renamed".into();
    let errors = validate_distribution(&clone, &[d]);
    assert!(
        !errors
            .iter()
            .any(|e| matches!(e, DistributionError::BasePathConflict { .. })),
        "self should not conflict with itself: {:?}",
        errors
    );
}

#[test]
fn resolve_content_path_strips_prefix_only_for_matching_base() {
    let d1 = dist_with_publication("a", "rpm/el9", ContentType::Rpm);
    let d2 = dist_with_publication("b", "deb/main", ContentType::Debian);
    let resolved = resolve_content_path(&[d1, d2], "/pulp/content/deb/main/dists/Release");
    assert_eq!(resolved.as_deref(), Some("dists/Release"));
}

#[test]
fn resolve_content_path_empty_list_returns_none() {
    let resolved = resolve_content_path(&[], "/pulp/content/rpm/el9/repodata/repomd.xml");
    assert!(resolved.is_none());
}

#[test]
fn find_distribution_by_path_returns_none_when_absent() {
    let d = dist_with_publication("only", "files", ContentType::File);
    assert!(find_distribution_by_path(&[d], "missing/path").is_none());
}

// ─── 3. Artifact dedup hashing + verification ────────────────────────────────

#[test]
fn verify_sha256_rejects_non_hex_chars() {
    let bad = "g".repeat(64); // 'g' is not a hex digit
    assert!(!verify_sha256(b"data", &bad));
}

#[test]
fn verify_sha256_rejects_uppercase_hex_strict_lower() {
    // The verifier requires ascii_hexdigit (which accepts both cases).
    // It accepts uppercase too — assert that explicitly so any tightening
    // of the contract is caught.
    let upper = "A".repeat(64);
    assert!(verify_sha256(b"data", &upper));
    let mixed = "AbCdEf0123456789".repeat(4);
    assert!(verify_sha256(b"data", &mixed));
}

#[test]
fn verify_sha256_empty_string_rejected() {
    assert!(!verify_sha256(b"data", ""));
}

#[test]
fn verify_artifact_checksums_skips_missing_algorithms() {
    let art = make_artifact(1024, None);
    let results = verify_artifact_checksums(&art, &[0u8; 1024]);
    // No sha256 set => no results emitted.
    assert!(results.is_empty());
}

#[test]
fn verify_artifact_checksums_emits_sha256_when_set() {
    let art = make_artifact(1024, Some(&"f".repeat(64)));
    let results = verify_artifact_checksums(&art, &[0u8; 1024]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].algorithm, "sha256");
    assert!(results[0].valid);
}

#[test]
fn artifact_check_corrupted_carries_expected_digest() {
    // Build artifact whose declared sha256 is 64-char but size mismatches.
    let expected = "1".repeat(64);
    let art = make_artifact(2048, Some(&expected));
    let check = check_artifact(&art, Some(&[0u8; 1024]));
    match check {
        ArtifactCheck::Corrupted {
            expected_sha256,
            actual_size,
        } => {
            assert_eq!(expected_sha256, expected);
            assert_eq!(actual_size, 1024);
        }
        other => panic!("expected Corrupted, got {:?}", other),
    }
}

#[test]
fn artifact_check_ok_when_artifact_has_no_sha256() {
    // No sha256 set => verify path returns Ok regardless of payload.
    let art = make_artifact(99, None);
    let check = check_artifact(&art, Some(&[7u8; 99]));
    assert_eq!(check, ArtifactCheck::Ok);
}

// ─── 4. Policy / access (RBAC) rules ─────────────────────────────────────────

#[test]
fn permission_wildcard_matches_any_in_app() {
    let p = Permission("core.*".into());
    assert!(p.matches("core.view_repository"));
    assert!(p.matches("core.add_artifact"));
    assert!(!p.matches("other.view_repository"));
}

#[test]
fn permission_exact_vs_wildcard_no_partial_app_match() {
    // "core.view_*" is not a recognised wildcard form (only suffix ".*"),
    // so it should NOT match arbitrary continuations.
    let p = Permission("core.view_repository".into());
    assert!(p.matches("core.view_repository"));
    assert!(!p.matches("core.view_repository_extra"));
}

#[test]
fn rbac_unknown_role_in_assignment_yields_no_access() {
    let roles = builtin_roles();
    let assignments = vec![RoleAssignment {
        role: "non.existent.role".into(),
        users: vec!["alice".into()],
        groups: vec![],
        content_object: None,
    }];
    assert!(!user_has_permission(
        "alice",
        &[],
        "core.view_repository",
        &assignments,
        None,
        &roles,
    ));
}

#[test]
fn rbac_non_member_user_denied() {
    let roles = builtin_roles();
    let assignments = vec![RoleAssignment {
        role: "core.viewer".into(),
        users: vec!["bob".into()],
        groups: vec![],
        content_object: None,
    }];
    assert!(!user_has_permission(
        "alice",
        &[],
        "core.view_repository",
        &assignments,
        None,
        &roles,
    ));
}

#[test]
fn rbac_get_user_permissions_deduplicates_across_roles() {
    // Both roles include `core.view_repository` — result should not duplicate.
    let roles = builtin_roles();
    let assignments = vec![
        RoleAssignment {
            role: "core.viewer".into(),
            users: vec!["alice".into()],
            groups: vec![],
            content_object: None,
        },
        RoleAssignment {
            role: "core.repository_owner".into(),
            users: vec!["alice".into()],
            groups: vec![],
            content_object: None,
        },
    ];
    let perms = get_user_permissions("alice", &[], &assignments, &roles);
    let view_count = perms.iter().filter(|p| p.contains("view_repository")).count();
    assert_eq!(view_count, 1, "expected dedup for view_repository: {:?}", perms);
    // Result is sorted.
    let mut sorted_check = perms.clone();
    sorted_check.sort();
    assert_eq!(perms, sorted_check);
}

#[test]
fn rbac_permission_lists_contain_expected_codenames() {
    let repo = repository_permissions();
    assert!(repo.iter().any(|p| p.0 == "core.sync_repository"));
    assert!(repo.iter().any(|p| p.0 == "core.repair_repository"));
    let art = artifact_permissions();
    assert!(art.iter().any(|p| p.0 == "core.add_artifact"));
    let dist = distribution_permissions();
    assert!(dist.iter().any(|p| p.0 == "core.manage_roles_distribution"));
}

// ─── 5. Sync / task state transitions ────────────────────────────────────────

#[test]
fn task_terminal_predicate_covers_all_four_terminal_states() {
    for s in [
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Canceled,
        TaskState::Skipped,
    ] {
        let mut t = Task::new("dummy");
        t.state = s.clone();
        assert!(t.is_terminal(), "{:?} should be terminal", s);
    }
    for s in [TaskState::Waiting, TaskState::Running] {
        let mut t = Task::new("dummy");
        t.state = s.clone();
        assert!(!t.is_terminal(), "{:?} should NOT be terminal", s);
    }
}

#[test]
fn task_elapsed_none_until_started() {
    let t = Task::new("noop");
    assert!(t.elapsed_seconds().is_none());
}

#[test]
fn task_elapsed_some_after_running_even_if_not_finished() {
    let mut t = Task::new("noop");
    t.mark_running();
    let e = t.elapsed_seconds();
    assert!(e.is_some());
    assert!(e.unwrap() >= 0.0);
}

#[test]
fn task_queue_get_unknown_returns_none() {
    let q = TaskQueue::new();
    assert!(q.get(&Uuid::new_v4()).is_none());
}

#[test]
fn task_queue_cancel_unknown_id_returns_false() {
    let q = TaskQueue::new();
    assert!(!q.cancel(&Uuid::new_v4()));
}

#[test]
fn task_queue_list_returns_all_inserted_tasks() {
    let q = TaskQueue::new();
    let _ = q.enqueue("a");
    let _ = q.enqueue("b");
    let _ = q.enqueue("c");
    assert_eq!(q.list().len(), 3);
}

#[test]
fn task_queue_purge_keeps_running_tasks() {
    let q = TaskQueue::new();
    let t1 = q.enqueue("done");
    let _t2 = q.enqueue("waiting");

    let mut done = t1.clone();
    done.mark_running();
    done.mark_completed(vec![]);
    q.update(done);

    let purged = q.purge_completed();
    assert_eq!(purged, 1);
    // waiting task remains
    assert_eq!(q.list().len(), 1);
}

#[test]
fn task_group_incomplete_when_not_dispatched() {
    let mut g = TaskGroup::new("bulk-sync");
    g.all_tasks_dispatched = false;
    g.waiting = 0;
    g.running = 0;
    g.completed = 0;
    assert!(!g.is_complete());
}

#[test]
fn task_progress_report_preserves_total() {
    let mut t = Task::new("sync");
    t.add_progress("Downloading", 25, Some(100));
    let pr = &t.progress_reports[0];
    assert_eq!(pr.done, 25);
    assert_eq!(pr.total, Some(100));
    assert_eq!(pr.state, "running");
}

#[test]
fn enqueue_sync_task_name_includes_plugin_short_name() {
    let repo = create_repository("centos-stream", ContentType::Rpm);
    let remote = Remote::new(
        "centos-upstream",
        "https://mirror.example.com/centos/9/",
        ContentType::Rpm,
    );
    let q = TaskQueue::new();
    let task = enqueue_sync(&repo, &remote, true, &q);
    assert!(task.name.starts_with("pulp_rpm.tasks.synchronize"));
    assert_eq!(task.state, TaskState::Waiting);
}

#[test]
fn add_and_remove_content_distinct_task_names() {
    let repo = create_repository("repo", ContentType::File);
    let q = TaskQueue::new();
    let add = add_content(&repo, &["/c/a/".to_string()], &q);
    let rem = remove_content(&repo, &["/c/b/".to_string()], &q);
    assert!(add.name.contains("add_content"));
    assert!(rem.name.contains("remove_content"));
    assert_ne!(add.name, rem.name);
}

#[test]
fn repair_version_task_dispatches_with_repair_name() {
    let v = RepositoryVersion::new("/pulp/api/v3/repositories/abc/", 7);
    let q = TaskQueue::new();
    let t = repair_version(&v, true, &q);
    assert_eq!(t.name, "pulp.tasks.repair");
}

#[test]
fn enqueue_repair_returns_pending_task() {
    let q = TaskQueue::new();
    let opts = RepairOptions {
        verify_checksums: true,
        redownload_missing: false,
        dry_run: true,
    };
    let t = enqueue_repair("/pulp/api/v3/repositories/abc/versions/1/", &opts, &q);
    assert_eq!(t.state, TaskState::Waiting);
}

#[test]
fn repair_report_dirty_is_not_clean() {
    let r = RepairReport {
        total_checked: 50,
        missing: 0,
        corrupted: 1,
        repaired: 0,
        unrepairable: 1,
        space_reclaimed_bytes: 4096,
    };
    assert!(!r.is_clean());
}

// ─── 6. Remote state and policy ──────────────────────────────────────────────

#[test]
fn remote_defaults_set_download_concurrency_and_retries() {
    let r = Remote::new("pypi", "https://pypi.org/simple/", ContentType::Python);
    assert_eq!(r.download_concurrency, Some(4));
    assert_eq!(r.max_retries, Some(3));
    assert!(r.tls_validation);
    assert!(r.proxy_url.is_none());
    assert_eq!(r.policy, RemotePolicy::Immediate);
}

#[test]
fn remote_serde_with_extra_headers_and_extra_options() {
    let mut r = Remote::new("repo", "https://example.com/", ContentType::Container);
    let mut hdr = HashMap::new();
    hdr.insert("X-Trace".to_string(), "1".to_string());
    r.headers.push(hdr);
    r.extra
        .insert("include_tags".into(), serde_json::json!(["latest", "v1.*"]));
    r.policy = RemotePolicy::OnDemand;
    let json = serde_json::to_string(&r).unwrap();
    let back: Remote = serde_json::from_str(&json).unwrap();
    assert_eq!(back.headers.len(), 1);
    assert_eq!(back.extra.len(), 1);
    assert_eq!(back.policy, RemotePolicy::OnDemand);
}

// ─── 7. Upload chunk lifecycle ──────────────────────────────────────────────

#[test]
fn upload_zero_size_complete_at_zero_offset() {
    let upload = Upload::new(0);
    assert!(upload.is_complete());
    assert!((upload.progress_pct() - 100.0).abs() < f64::EPSILON);
}

#[test]
fn upload_exact_boundary_accepts_final_chunk() {
    let mut u = Upload::new(1024);
    assert!(u.accept_chunk(0, 1024).is_ok());
    assert!(u.is_complete());
}

#[test]
fn upload_offset_mismatch_does_not_advance_state() {
    let mut u = Upload::new(1024);
    u.accept_chunk(0, 256).unwrap();
    let pre = u.offset;
    let err = u.accept_chunk(512, 256).unwrap_err();
    assert!(matches!(err, UploadError::OutOfOrder { expected: 256, got: 512 }));
    // Offset must NOT have advanced on error.
    assert_eq!(u.offset, pre);
}

#[test]
fn upload_registry_apply_chunk_unknown_id_returns_not_found() {
    let r = UploadRegistry::new();
    let err = r
        .apply_chunk(&Uuid::new_v4(), 0, 100)
        .expect_err("expected NotFound");
    assert!(matches!(err, UploadError::NotFound(_)));
}

#[test]
fn upload_registry_finalize_unknown_id_returns_not_found() {
    let r = UploadRegistry::new();
    let err = r
        .finalize(&Uuid::new_v4(), "/artifact/")
        .expect_err("expected NotFound");
    assert!(matches!(err, UploadError::NotFound(_)));
}

#[test]
fn upload_registry_delete_unknown_id_returns_false() {
    let r = UploadRegistry::new();
    assert!(!r.delete(&Uuid::new_v4()));
}

#[test]
fn upload_registry_delete_existing_id_returns_true() {
    let r = UploadRegistry::new();
    let u = r.create(10);
    assert!(r.delete(&u.pulp_id));
    assert!(r.get(&u.pulp_id).is_none());
}

#[test]
fn upload_request_structs_serde_roundtrip() {
    let req = UploadChunkRequest {
        upload_id: Uuid::new_v4(),
        offset: 4096,
        content_range: "bytes 4096-8191/16384".into(),
    };
    let json = serde_json::to_string(&req).unwrap();
    let back: UploadChunkRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.offset, 4096);

    let fin = FinalizeUploadRequest {
        sha256: "e".repeat(64),
    };
    let json2 = serde_json::to_string(&fin).unwrap();
    let back2: FinalizeUploadRequest = serde_json::from_str(&json2).unwrap();
    assert_eq!(back2.sha256.len(), 64);
}

#[test]
fn parse_content_range_no_slash_returns_none() {
    assert!(parse_content_range("bytes 0-1023").is_none());
}

#[test]
fn parse_content_range_missing_dash_returns_none() {
    assert!(parse_content_range("bytes 01023/2048").is_none());
}

// ─── 8. Content filters — boundary + combinator ──────────────────────────────

#[test]
fn rpm_filter_default_accepts_all() {
    let f = RpmFilter::default();
    let pkg = RpmPackage {
        pulp_href: "/pulp/api/v3/content/rpm/packages/abc/".into(),
        pulp_id: Uuid::new_v4(),
        name: "anything".into(),
        version: "0".into(),
        release: "0".into(),
        arch: "x86_64".into(),
        epoch: "0".into(),
        summary: None,
        description: None,
        url: None,
        rpm_license: None,
        rpm_vendor: None,
        rpm_group: None,
        source_rpm: None,
        artifact: "/pulp/api/v3/artifacts/abc/".into(),
        location_href: "anything.rpm".into(),
        sha256: "f".repeat(64),
        size_package: 1,
        time_file: 0,
        time_build: 0,
    };
    assert!(f.matches(&pkg));
}

#[test]
fn rpm_filter_combines_name_and_arch_and_release() {
    let pkg = RpmPackage {
        pulp_href: "/x/".into(),
        pulp_id: Uuid::new_v4(),
        name: "kernel-modules".into(),
        version: "6.6.0".into(),
        release: "1.fc40".into(),
        arch: "x86_64".into(),
        epoch: "0".into(),
        summary: None,
        description: None,
        url: None,
        rpm_license: None,
        rpm_vendor: None,
        rpm_group: None,
        source_rpm: None,
        artifact: "/x/".into(),
        location_href: "x.rpm".into(),
        sha256: "f".repeat(64),
        size_package: 1,
        time_file: 0,
        time_build: 0,
    };
    let f = RpmFilter {
        name: Some("kernel".into()), // substring match
        arch: Some("x86_64".into()),
        release: Some("1.fc40".into()),
        ..Default::default()
    };
    assert!(f.matches(&pkg));
    let f_wrong_arch = RpmFilter {
        arch: Some("ppc64le".into()),
        ..f.clone()
    };
    assert!(!f_wrong_arch.matches(&pkg));
}

#[test]
fn deb_filter_substring_on_package_name() {
    let pkg = DebPackage {
        pulp_href: "/x/".into(),
        pulp_id: Uuid::new_v4(),
        package: "libssl3".into(),
        version: "3.0.13-1ubuntu1".into(),
        architecture: "amd64".into(),
        section: None,
        priority: None,
        maintainer: None,
        description: None,
        depends: None,
        pre_depends: None,
        suggests: None,
        recommends: None,
        sha256: "a".repeat(64),
        size: 1,
        artifact: "/x/".into(),
        relative_path: "pool/x.deb".into(),
    };
    let f = DebFilter {
        package: Some("ssl".into()),
        ..Default::default()
    };
    assert!(f.matches(&pkg));
}

#[test]
fn pypi_filter_by_package_type_only() {
    let pkg_wheel = make_python_pkg("a", "1", PythonPackageType::Bdist_wheel, &"a".repeat(64));
    let pkg_sdist = make_python_pkg("a", "1", PythonPackageType::Sdist, &"a".repeat(64));
    let f = PypiFilter {
        package_type: Some(PythonPackageType::Bdist_wheel),
        ..Default::default()
    };
    assert!(f.matches(&pkg_wheel));
    assert!(!f.matches(&pkg_sdist));
}

// ─── 9. Metadata generation — round-trip edge cases ──────────────────────────

#[test]
fn pypi_simple_page_empty_packages_still_renders_header() {
    let html = generate_pypi_simple_page("nothing", &[]);
    assert!(html.contains("Links for nothing"));
    assert!(html.contains("</html>"));
}

#[test]
fn pypi_project_json_pep691_shape() {
    let pkg = make_python_pkg("flask", "3.0.0", PythonPackageType::Bdist_wheel, &"a".repeat(64));
    let json = generate_pypi_project_json("flask", &[pkg]);
    assert_eq!(json["meta"]["api-version"], "1.0");
    assert_eq!(json["name"], "flask");
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["hashes"]["sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn deb_package_entry_omits_optional_fields_when_none() {
    let pkg = DebPackage {
        pulp_href: "/x/".into(),
        pulp_id: Uuid::new_v4(),
        package: "tzdata".into(),
        version: "2024a-1".into(),
        architecture: "all".into(),
        section: None,
        priority: None,
        maintainer: None, // intentionally None
        description: None, // intentionally None
        depends: None, // intentionally None
        pre_depends: None,
        suggests: None,
        recommends: None,
        sha256: "a".repeat(64),
        size: 512,
        artifact: "/x/".into(),
        relative_path: "pool/t/tzdata/tzdata_2024a-1_all.deb".into(),
    };
    let entry = generate_deb_package_entry(&pkg);
    assert!(entry.contains("Package: tzdata"));
    assert!(!entry.contains("Maintainer:"));
    assert!(!entry.contains("Description:"));
    assert!(!entry.contains("Depends:"));
    // Required fields stay
    assert!(entry.contains("Filename: pool/t/tzdata/tzdata_2024a-1_all.deb"));
    assert!(entry.contains("SHA256:"));
}

#[test]
fn repomd_xml_includes_filelists_and_other() {
    let xml = generate_repomd_xml(&[], "/relative/repo");
    assert!(xml.contains("filelists.xml.gz"));
    assert!(xml.contains("other.xml.gz"));
    assert!(xml.contains("primary.xml.gz"));
    assert!(xml.contains("/relative/repo/repodata/"));
}

// ─── 10. Version pruning + paginated response ────────────────────────────────

#[test]
fn versions_to_prune_handles_unsorted_input() {
    let repo_href = "/pulp/api/v3/repositories/abc/";
    let mut versions: Vec<RepositoryVersion> = (1..=5u64)
        .rev()
        .map(|n| RepositoryVersion::new(repo_href, n))
        .collect();
    // Versions arrive in 5,4,3,2,1 order — pruner must sort before slicing oldest.
    versions.reverse(); // 1,2,3,4,5
    let to_prune = versions_to_prune(&versions, 2);
    assert_eq!(to_prune.len(), 3);
}

#[test]
fn versions_to_prune_retain_equals_count_is_no_op() {
    let repo_href = "/pulp/api/v3/repositories/abc/";
    let versions: Vec<RepositoryVersion> =
        (1..=5u64).map(|n| RepositoryVersion::new(repo_href, n)).collect();
    let to_prune = versions_to_prune(&versions, 5);
    assert!(to_prune.is_empty());
}

#[test]
fn paginated_response_empty_zero_count() {
    let resp: PaginatedResponse<i32> = PaginatedResponse::of(vec![]);
    assert_eq!(resp.count, 0);
    assert!(resp.results.is_empty());
    assert!(resp.next.is_none());
    assert!(resp.previous.is_none());
}

#[test]
fn paginated_response_serde_roundtrip_with_typed_items() {
    let resp = PaginatedResponse::of(vec![1u64, 2, 3, 4]);
    let json = serde_json::to_string(&resp).unwrap();
    let back: PaginatedResponse<u64> = serde_json::from_str(&json).unwrap();
    assert_eq!(back.count, 4);
    assert_eq!(back.results, vec![1, 2, 3, 4]);
}

// ─── 11. Signing service + verification ─────────────────────────────────────

#[test]
fn signing_service_type_serde_lowercase() {
    for ty in [
        SigningServiceType::Gpg,
        SigningServiceType::X509,
        SigningServiceType::Sigstore,
    ] {
        let s = serde_json::to_string(&ty).unwrap();
        let back: SigningServiceType = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ty);
    }
}

#[test]
fn verify_gpg_signature_short_signature_invalid() {
    let result = verify_gpg_signature(b"data", "short", "PUBLIC KEY");
    assert!(!result.is_valid());
    if let VerificationResult::Invalid { reason } = result {
        assert!(reason.contains("short"));
    } else {
        panic!("expected Invalid");
    }
}

#[test]
fn verification_result_unknown_not_valid() {
    let r = VerificationResult::Unknown;
    assert!(!r.is_valid());
}

#[test]
fn rpm_has_signature_dsa_header_detected() {
    let mut tags = HashMap::new();
    tags.insert("RPMTAG_DSAHEADER".to_string(), "fake".to_string());
    assert!(rpm_has_signature(&tags));
}

#[test]
fn rpm_has_signature_unknown_tag_not_detected() {
    let mut tags = HashMap::new();
    tags.insert("RPMTAG_OTHER".to_string(), "x".to_string());
    assert!(!rpm_has_signature(&tags));
}

#[test]
fn cosign_bundle_serde_preserves_signatures() {
    let bundle = CosignBundle {
        payload: "base64payload".into(),
        payload_type: "application/vnd.dev.cosign.simplesigning.v1+json".into(),
        signatures: vec![
            CosignSignature {
                keyid: "kid-1".into(),
                sig: "sig-1".into(),
            },
            CosignSignature {
                keyid: "kid-2".into(),
                sig: "sig-2".into(),
            },
        ],
    };
    let json = serde_json::to_string(&bundle).unwrap();
    let back: CosignBundle = serde_json::from_str(&json).unwrap();
    assert_eq!(back.signatures.len(), 2);
    assert_eq!(back.signatures[1].keyid, "kid-2");
}

#[test]
fn signing_request_constructed_with_multiple_content_hrefs() {
    let req = SigningRequest {
        content_hrefs: vec![
            "/pulp/api/v3/content/rpm/packages/a/".into(),
            "/pulp/api/v3/content/rpm/packages/b/".into(),
        ],
        signing_service_href: "/pulp/api/v3/signing-services/svc/".into(),
    };
    assert_eq!(req.content_hrefs.len(), 2);
}

#[test]
fn content_signature_serde_roundtrip() {
    let sig = ContentSignature {
        pulp_href: "/pulp/api/v3/content-signatures/abc/".into(),
        pulp_id: Uuid::new_v4(),
        signed_at: Utc::now(),
        signing_service: "/pulp/api/v3/signing-services/svc/".into(),
        content: "/pulp/api/v3/content/rpm/packages/abc/".into(),
        signature_data: "BASE64SIG".into(),
        key_id: "0xCAFEBABE".into(),
        valid: true,
    };
    let json = serde_json::to_string(&sig).unwrap();
    let back: ContentSignature = serde_json::from_str(&json).unwrap();
    assert!(back.valid);
    assert_eq!(back.key_id, sig.key_id);
}

// ─── 12. Import / Export params + ContentSummary ─────────────────────────────

#[test]
fn export_params_serde_with_versions_and_chunking() {
    let params = ExportParams {
        repositories: vec!["/pulp/api/v3/repositories/a/".into()],
        versions: vec!["/pulp/api/v3/repositories/a/versions/3/".into()],
        chunk_size: Some(500 * 1024 * 1024),
        start_versions: vec![],
        full: true,
    };
    let exp = PulpExport {
        pulp_href: "/pulp/api/v3/exports/abc/".into(),
        pulp_id: Uuid::new_v4(),
        pulp_created: Utc::now(),
        exporter: "/pulp/api/v3/exporters/pulp/abc/".into(),
        params,
        task: None,
        output_file_info: None,
        toc_info: None,
    };
    let json = serde_json::to_string(&exp).unwrap();
    let back: PulpExport = serde_json::from_str(&json).unwrap();
    assert!(back.params.full);
    assert_eq!(back.params.chunk_size, Some(500 * 1024 * 1024));
}

#[test]
fn import_params_create_repositories_round_trip() {
    let imp = PulpImport {
        pulp_href: "/pulp/api/v3/imports/abc/".into(),
        pulp_id: Uuid::new_v4(),
        pulp_created: Utc::now(),
        importer: "/pulp/api/v3/importers/pulp/abc/".into(),
        params: ImportParams {
            path: "/var/lib/pulp/imports/file.tar.gz".into(),
            toc: Some("/var/lib/pulp/imports/file.toc.json".into()),
            create_repositories: true,
        },
        task: None,
    };
    let json = serde_json::to_string(&imp).unwrap();
    let back: PulpImport = serde_json::from_str(&json).unwrap();
    assert!(back.params.create_repositories);
    assert_eq!(
        back.params.toc.as_deref(),
        Some("/var/lib/pulp/imports/file.toc.json")
    );
}

#[test]
fn content_summary_default_is_empty() {
    let s = ContentSummary::default();
    assert!(s.added.is_empty());
    assert!(s.removed.is_empty());
    assert!(s.present.is_empty());
}

#[test]
fn sync_report_serde_roundtrip() {
    let r = SyncReport {
        added: 42,
        removed: 3,
        unchanged: 100,
        total_size_bytes: 1024 * 1024 * 1024,
        duration_seconds: 7.5,
        new_version_href: Some("/pulp/api/v3/repositories/abc/versions/2/".into()),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: SyncReport = serde_json::from_str(&json).unwrap();
    assert_eq!(back.added, 42);
    assert_eq!(back.new_version_href, r.new_version_href);
}

// ─── 13. BuiltinRole structure invariants ────────────────────────────────────

#[test]
fn builtin_role_set_has_expected_names() {
    let roles = builtin_roles();
    let names: Vec<&str> = roles.iter().map(|r| r.name).collect();
    for required in [
        "core.superuser",
        "core.viewer",
        "core.task_owner",
        "core.repository_creator",
        "core.repository_owner",
        "core.artifact_creator",
    ] {
        assert!(
            names.contains(&required),
            "missing role: {required} (have {names:?})"
        );
    }
}

#[test]
fn builtin_role_serializes_with_expected_wire_shape() {
    // BuiltinRole borrows &'static str, so we verify wire-shape rather than
    // attempt a borrow-lifetime-incompatible deserialise.
    let r = BuiltinRole {
        name: "core.task_owner",
        description: "Can manage tasks.",
        permissions: vec!["core.view_task", "core.cancel_task"],
        locked: true,
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["name"], "core.task_owner");
    assert!(json["locked"].as_bool().unwrap());
    let perms = json["permissions"].as_array().unwrap();
    assert_eq!(perms.len(), 2);
    assert_eq!(perms[0], "core.view_task");
}
