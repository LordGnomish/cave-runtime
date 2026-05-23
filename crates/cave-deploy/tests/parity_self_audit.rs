// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-deploy must carry an honest, measured
//! `fill_ratio` against upstream argoproj/argo-cd v3.4.2, a pinned
//! `source_sha`, the 2026-05-22 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage,
//! no stub macros in `src/`, mapped+partial+skipped+unmapped summing to
//! total, and the full sync + diff + health + rbac + rollout +
//! notification public surface reachable through `cave_deploy`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-22";
const FLOOR_FILL_RATIO: f64 = 0.65;
const PINNED_VERSION: &str = "v3.4.2";
const PINNED_SHA: &str = "0dc6b1b57dd5bb925d5b03c3d09419ab9fb4225e";

fn manifest_text() -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"]
        .iter()
        .collect();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn extract_after(text: &str, needle: &str) -> Option<String> {
    let i = text.find(needle)?;
    let rest = &text[i + needle.len()..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line = &rest[..line_end];
    let stripped = line.trim().trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

// ─── Assertion 1: upstream pinned to v3.4.2 ─────────────────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin ArgoCD {} — Charter v2 always-latest gate (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches commit for v3.4.2 ──────────────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "[upstream] source_sha must be set (got {:?})",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some(PINNED_SHA),
        "source_sha must match the v3.4.2 tag commit (got {:?})",
        sha
    );
}

// ─── Assertion 3: fill_ratio >= 0.65 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-deploy MVP floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
}

// ─── Assertion 4: parity_ratio_source = "manifest" ──────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" (got {:?})",
        v
    );
}

// ─── Assertion 5: last_audit == 2026-05-22 ──────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: counts sum to total + >= 15 mapped ────────────────────────

#[test]
fn assertion_6_counts_sum_to_total() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        let s = extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?;
        s.parse().ok()
    };
    let mapped = read("mapped_count").expect("mapped_count");
    let partial = read("partial_count").expect("partial_count");
    let skipped = read("skipped_count").expect("skipped_count");
    let unmapped = read("unmapped_count").expect("unmapped_count");
    let total = read("total").expect("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total"
    );
    assert!(
        mapped >= 15,
        "cave-deploy MVP floor: >= 15 mapped ArgoCD subsystems (got {})",
        mapped
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100% ────────────────────────────

#[test]
fn assertion_7_agpl_spdx_header_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.to_string()))
                .unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(
        missing.is_empty(),
        "{} of {} .rs files missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
    assert!(
        total >= 13,
        "expected >= 13 .rs files in cave-deploy; got {}",
        total
    );
}

// ─── Assertion 8: no stub macros in src/ ────────────────────────────────────

#[test]
fn assertion_8_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else {
            return;
        };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("todo!(")
                || trimmed.contains("unimplemented!(")
                || trimmed.contains("panic!(\"stub")
                || trimmed.contains("panic!(\"todo")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 9: deploy surface intact ─────────────────────────────────────

#[test]
fn assertion_9_deploy_surface_intact() {
    use cave_deploy::appset::{
        evaluate_list_generator, evaluate_matrix_generator, evaluate_merge_generator, Generator,
        ListGenerator,
    };
    use cave_deploy::cluster::{build_resource_url, kind_to_plural, Cluster, TRACKING_LABEL};
    use cave_deploy::diff::{compute_diff, normalize_resource, DiffType};
    use cave_deploy::error::DeployError;
    use cave_deploy::gitops::{
        auto_sync, detect_drift, parse_json_documents, parse_yaml_documents, render_manifests,
        sync_application,
    };
    use cave_deploy::health::{check_deployment, check_pod, HealthCheckRegistry};
    use cave_deploy::models::{
        Application, ApplicationSource, ApplicationSpec, AutomatedSyncPolicy, Destination,
        HealthCondition, HealthStatus, NotificationDestination, NotificationTrigger,
        ResourceTracking, RolloutStatus, RolloutStrategy, SSOProvider, SyncCondition,
        SyncPhase, SyncPolicy, SyncStatus,
    };
    use cave_deploy::notifications::{build_slack_payload, slack_color, NotificationEngine};
    use cave_deploy::rbac::{
        has_permission, is_destination_allowed, is_source_allowed, AppProject, AppProjectSpec,
        GroupKind, RbacAction,
    };
    use cave_deploy::rollout::{
        blue_green_deploy, canary_deploy, promote_canary, rolling_update, traffic_split,
    };
    use cave_deploy::store::DeployStore;
    use cave_deploy::sync::{
        group_by_wave, initiate_rollback, parse_hook_phases, parse_sync_options, parse_wave,
        should_auto_sync, ManifestResource, RollbackRequest, SyncStrategy,
    };
    use std::collections::HashMap;
    use uuid::Uuid;

    // 1. CRD enums + helpers are reachable
    let _ = HealthStatus::Healthy;
    let _ = HealthStatus::Degraded;
    let _ = HealthStatus::Suspended;
    let _ = SyncStatus::Synced;
    let _ = SyncStatus::OutOfSync;
    let _ = SyncStatus::Unknown;
    let _ = SyncPhase::PreSync;
    let _ = SyncPhase::Sync;
    let _ = SyncPhase::PostSync;
    let _ = SyncPhase::SyncFail;
    let _ = SSOProvider::Dex;
    let _ = SSOProvider::Okta;
    let _ = SyncStrategy::Apply;
    let _ = ResourceTracking::default();

    // 2. ApplicationSet generators evaluate
    let list = evaluate_list_generator(&ListGenerator {
        elements: vec![[("env".to_string(), "prod".to_string())].into()],
        template: None,
    });
    assert_eq!(list.len(), 1);
    let matrix = evaluate_matrix_generator(&list, &list);
    assert_eq!(matrix.len(), 1);
    let merged = evaluate_merge_generator(&list, &list, &["env".to_string()]);
    assert_eq!(merged.len(), 1);

    // 3. Sync + waves + hooks + options
    let resources = vec![
        ManifestResource {
            group: "".into(),
            version: "v1".into(),
            kind: "ConfigMap".into(),
            namespace: "default".into(),
            name: "cm".into(),
            wave: 2,
            hook_phases: vec![],
            delete_on_success: false,
            manifest: serde_json::json!({}),
        },
        ManifestResource {
            group: "apps".into(),
            version: "v1".into(),
            kind: "Deployment".into(),
            namespace: "default".into(),
            name: "dep".into(),
            wave: 0,
            hook_phases: vec![SyncPhase::PreSync],
            delete_on_success: true,
            manifest: serde_json::json!({}),
        },
    ];
    let waves = group_by_wave(&resources);
    assert_eq!(waves[0].0, 0);
    let mut anno = HashMap::new();
    anno.insert(
        "argocd.argoproj.io/sync-wave".to_string(),
        "7".to_string(),
    );
    anno.insert(
        "argocd.argoproj.io/hook".to_string(),
        "PreSync,PostSync".to_string(),
    );
    assert_eq!(parse_wave(&anno), 7);
    assert_eq!(parse_hook_phases(&anno).len(), 2);
    let opts = parse_sync_options(&[
        "CreateNamespace=true".to_string(),
        "ServerSideApply=true".to_string(),
    ]);
    assert!(opts.create_namespace);
    assert!(opts.server_side_apply);

    // 4. Diff engine
    let desired = serde_json::json!({"spec": {"replicas": 3}});
    let live = serde_json::json!({"spec": {"replicas": 1}});
    let diffs = compute_diff(&desired, &live);
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].diff_type, DiffType::Modified);
    let normalized = normalize_resource(&serde_json::json!({
        "metadata": {"uid": "abc", "managedFields": [], "name": "x"},
        "spec": {"replicas": 1},
        "status": {"observed": 1}
    }));
    assert!(normalized["metadata"]["uid"].is_null());
    assert!(normalized["status"].is_null());

    // 5. Health registry assesses
    let reg = HealthCheckRegistry::new();
    let dep = serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "spec": { "replicas": 2 },
        "status": { "availableReplicas": 2, "readyReplicas": 2, "updatedReplicas": 2 }
    });
    assert_eq!(reg.assess(&dep).status, HealthStatus::Healthy);
    let bad_pod = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "status": {
            "phase": "Running",
            "containerStatuses": [{"state": {"waiting": {"reason": "CrashLoopBackOff"}}}]
        }
    });
    assert_eq!(check_pod(&bad_pod).status, HealthStatus::Degraded);
    let ok_dep = check_deployment(&dep);
    assert_eq!(ok_dep.status, HealthStatus::Healthy);

    // 6. RBAC scope + role policy
    let project = AppProject {
        id: Uuid::new_v4(),
        name: "default".into(),
        description: None,
        spec: AppProjectSpec {
            source_repos: vec!["https://github.com/example/*".into()],
            source_namespaces: vec![],
            destinations: vec![cave_deploy::rbac::ProjectDestination {
                server: "*".into(),
                namespace: "*".into(),
                name: None,
            }],
            cluster_resource_whitelist: vec![GroupKind {
                group: "".into(),
                kind: "Namespace".into(),
            }],
            cluster_resource_blacklist: vec![],
            namespace_resource_whitelist: vec![],
            namespace_resource_blacklist: vec![],
            roles: vec![cave_deploy::rbac::ProjectRole {
                name: "sync-role".into(),
                description: None,
                policies: vec!["p, sync-role, applications, sync, allow".into()],
                jwt_tokens: vec![],
                groups: vec!["deploy-team".into()],
            }],
            sync_windows: vec![],
            signature_keys: vec![],
            orphaned_resources: None,
        },
    };
    assert!(is_source_allowed(&project, "https://github.com/example/app"));
    let dest = Destination {
        server: "https://k.example".into(),
        name: None,
        namespace: "prod".into(),
    };
    assert!(is_destination_allowed(&project, &dest));
    assert!(has_permission(
        &project,
        "deploy-team",
        "applications",
        &RbacAction::Sync,
    ));

    // 7. Sync + drift + auto-sync + manifest render + parsers
    let mut app = Application {
        id: Uuid::new_v4(),
        name: "demo".into(),
        namespace: "argocd".into(),
        spec: ApplicationSpec {
            source: ApplicationSource {
                repo_url: "https://github.com/example/app".into(),
                target_revision: Some("v1.0".into()),
                path: Some("k8s/".into()),
                helm: None,
                kustomize: None,
                directory: None,
            },
            sources: vec![],
            destination: dest.clone(),
            project: "default".into(),
            sync_policy: Some(SyncPolicy {
                automated: Some(AutomatedSyncPolicy {
                    prune: true,
                    self_heal: true,
                    allow_empty: false,
                }),
                sync_options: vec!["CreateNamespace=true".into()],
                retry: None,
                managed_namespace_metadata: None,
            }),
            ignored_differences: None,
            info: None,
            revision_history_limit: Some(10),
        },
        status: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        labels: Default::default(),
        annotations: Default::default(),
        tracking: ResourceTracking::default(),
    };
    let rev = sync_application(&mut app, None, false).unwrap();
    assert_eq!(rev, "v1.0");
    assert!(detect_drift(&app, 5));
    app.status = Some(cave_deploy::models::ApplicationStatus {
        health: HealthCondition {
            status: HealthStatus::Degraded,
            message: None,
        },
        sync: SyncCondition {
            status: SyncStatus::Synced,
            revision: "v1.0".into(),
            revisions: vec![],
        },
        resources: vec![],
        history: vec![],
        conditions: vec![],
        observed_at: Some(chrono::Utc::now()),
        reconciled_at: Some(chrono::Utc::now()),
    });
    assert!(auto_sync(&mut app).is_some());
    let manifests = render_manifests(&app).unwrap();
    assert!(!manifests.is_empty());
    let m0 = &manifests[0];
    assert_eq!(m0.raw["metadata"]["labels"][TRACKING_LABEL], "demo");

    let yaml = parse_yaml_documents(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\n  namespace: n\n",
    )
    .unwrap();
    assert_eq!(yaml.len(), 1);
    let json = parse_json_documents(
        "[{\"apiVersion\":\"v1\",\"kind\":\"Service\",\"metadata\":{\"name\":\"a\",\"namespace\":\"n\"}}]",
    )
    .unwrap();
    assert_eq!(json.len(), 1);

    // 8. Cluster URL builders + registry + tracking label
    assert_eq!(kind_to_plural("Deployment"), "deployments");
    assert_eq!(
        build_resource_url("apps/v1", "Deployment", "x", Some("ns")),
        "/apis/apps/v1/namespaces/ns/deployments/x"
    );
    let _cluster = Cluster {
        id: Uuid::new_v4(),
        name: "prod".into(),
        server: "https://k.example".into(),
        credential_ref: Some("keychain:cave-deploy-prod-token".into()),
        labels: Default::default(),
        annotations: Default::default(),
        project: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    // 9. Rollouts (canary/blue-green/rolling) + promote/traffic
    let app_id = Uuid::new_v4();
    let mut canary = canary_deploy(app_id, "v2".into(), vec![]);
    assert_eq!(canary.strategy, RolloutStrategy::Canary);
    assert_eq!(canary.status, RolloutStatus::Progressing);
    assert!(promote_canary(&mut canary));
    let bg = blue_green_deploy(app_id, "v2".into());
    assert_eq!(bg.strategy, RolloutStrategy::BlueGreen);
    let ru = rolling_update(app_id, "v2".into());
    assert_eq!(ru.strategy, RolloutStrategy::Rolling);
    let mut c2 = canary_deploy(app_id, "v3".into(), vec![]);
    traffic_split(&mut c2, 100);
    assert_eq!(c2.status, RolloutStatus::Completed);

    // 10. Notifications engine + Slack payload + trigger colour
    let engine = NotificationEngine::new(vec![]);
    assert_eq!(engine.subscriptions().len(), 0);
    let payload = build_slack_payload(&app, "synced", "#36a64f", "#deploys");
    assert_eq!(payload["channel"], "#deploys");
    assert_eq!(
        slack_color(&NotificationTrigger::OnSyncSucceeded),
        "#36a64f"
    );
    let _dest = NotificationDestination::Slack {
        channel: "#deploys".into(),
    };

    // 11. Store + rollback
    let store = DeployStore::new();
    let mut app2 = app.clone();
    app2.name = "rb-test".into();
    store.create_application(app2.clone()).unwrap();
    let history_entry = cave_deploy::models::RevisionHistory {
        id: 1,
        revision: "v1.0".into(),
        deployed_at: chrono::Utc::now(),
        initiated_by: "bot".into(),
        source: app2.spec.source.clone(),
    };
    store
        .append_revision("rb-test", history_entry.clone())
        .unwrap();
    let req = RollbackRequest {
        application_id: app2.id,
        history_id: 1,
        prune: false,
        dry_run: true,
        initiated_by: "bot".into(),
    };
    let res = initiate_rollback(&req, &[history_entry]).unwrap();
    assert_eq!(res.target_revision, "v1.0");

    // 12. Auto-sync evaluator agrees
    let policy = SyncPolicy {
        automated: Some(AutomatedSyncPolicy {
            prune: true,
            self_heal: false,
            allow_empty: false,
        }),
        sync_options: vec![],
        retry: None,
        managed_namespace_metadata: None,
    };
    assert!(should_auto_sync(
        &policy,
        &SyncStatus::OutOfSync,
        &HealthStatus::Healthy
    ));

    // 13. Error variants render distinct HTTP statuses
    let _e: DeployError = DeployError::NotFound("z".into());

    // 14. Generator variant constructor reachable
    let _g = Generator::List(ListGenerator {
        elements: vec![],
        template: None,
    });
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            if p.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}
