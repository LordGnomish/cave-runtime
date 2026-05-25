// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-deploy integration smoke — ApplicationSet fixture, sync engine
//! dry-run, RBAC evaluation, and the PreSync → Sync → PostSync hook
//! lifecycle. These tests exercise the MVP boundary without touching a
//! real Kubernetes cluster.

use cave_deploy::appset::{
    evaluate_git_directory_generator, evaluate_list_generator, evaluate_matrix_generator,
    evaluate_merge_generator, ApplicationSet, ApplicationSetSpec, ApplicationSetTemplate,
    ApplicationSetTemplateMetadata, GitDirectoryFilter, GitGenerator, Generator, ListGenerator,
};
use cave_deploy::cluster::{build_resource_url, kind_to_plural, TRACKING_LABEL};
use cave_deploy::diff::{apply_ignored_differences, compute_diff, normalize_resource, DiffType, IgnoredDiff};
use cave_deploy::gitops::{
    auto_sync, detect_drift, parse_json_documents, parse_yaml_documents, render_manifests,
    sync_application,
};
use cave_deploy::health::{check_deployment, check_ingress, check_service, HealthCheckRegistry};
use cave_deploy::models::*;
use cave_deploy::notifications::{build_slack_payload, slack_color, NotificationEngine};
use cave_deploy::rbac::{
    has_permission, is_destination_allowed, is_source_allowed, validate_application, AppProject,
    AppProjectSpec, GroupKind, ProjectDestination, ProjectRole, ProjectViolation, RbacAction,
};
use cave_deploy::rollout::{
    blue_green_deploy, canary_deploy, promote_canary, rollback, rolling_update, traffic_split,
};
use cave_deploy::store::DeployStore;
use cave_deploy::sync::{
    group_by_wave, initiate_rollback, parse_delete_on_success, parse_hook_phases,
    parse_sync_options, parse_wave, should_auto_sync, should_prune, ManifestResource,
    RollbackRequest,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ─── helpers ────────────────────────────────────────────────────────────────

fn make_app(name: &str, project: &str, repo: &str, namespace: &str) -> Application {
    let now = Utc::now();
    Application {
        id: Uuid::new_v4(),
        name: name.into(),
        namespace: "argocd".into(),
        spec: ApplicationSpec {
            source: ApplicationSource {
                repo_url: repo.into(),
                target_revision: Some("main".into()),
                path: Some("manifests/".into()),
                helm: None,
                kustomize: None,
                directory: None,
            },
            sources: vec![],
            destination: Destination {
                server: "https://kubernetes.default.svc".into(),
                name: None,
                namespace: namespace.into(),
            },
            project: project.into(),
            sync_policy: None,
            ignored_differences: None,
            info: None,
            revision_history_limit: Some(10),
        },
        status: None,
        created_at: now,
        updated_at: now,
        labels: Default::default(),
        annotations: Default::default(),
        tracking: ResourceTracking::default(),
    }
}

fn make_project(name: &str) -> AppProject {
    AppProject {
        id: Uuid::new_v4(),
        name: name.into(),
        description: Some("smoke fixture".into()),
        spec: AppProjectSpec {
            source_repos: vec!["https://github.com/example/*".into()],
            source_namespaces: vec![],
            destinations: vec![ProjectDestination {
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
            roles: vec![ProjectRole {
                name: "sync-role".into(),
                description: None,
                policies: vec![
                    "p, sync-role, applications, sync, allow".into(),
                    "p, sync-role, applications, get, allow".into(),
                ],
                jwt_tokens: vec![],
                groups: vec!["deploy-team".into()],
            }],
            sync_windows: vec![],
            signature_keys: vec![],
            orphaned_resources: None,
        },
    }
}

// ─── ApplicationSet fixture ─────────────────────────────────────────────────

#[test]
fn applicationset_list_generator_produces_apps() {
    let elements = vec![
        [("env".to_string(), "staging".to_string()), ("ns".to_string(), "stg".to_string())].into(),
        [("env".to_string(), "prod".to_string()), ("ns".to_string(), "prod".to_string())].into(),
    ];
    let params = evaluate_list_generator(&ListGenerator {
        elements,
        template: None,
    });
    assert_eq!(params.len(), 2);
    assert_eq!(params[0]["env"], "staging");
    assert_eq!(params[1]["env"], "prod");
}

#[test]
fn applicationset_matrix_x_list_explodes() {
    let regions = vec![
        [("region".to_string(), "us-east-1".to_string())].into(),
        [("region".to_string(), "eu-west-1".to_string())].into(),
    ];
    let envs = vec![
        [("env".to_string(), "staging".to_string())].into(),
        [("env".to_string(), "prod".to_string())].into(),
    ];
    let exploded = evaluate_matrix_generator(&regions, &envs);
    assert_eq!(exploded.len(), 4);
    assert!(exploded.iter().any(|p| p["region"] == "us-east-1" && p["env"] == "prod"));
}

#[test]
fn applicationset_merge_overrides_by_key() {
    let base = vec![
        [
            ("cluster".to_string(), "prod".to_string()),
            ("replicas".to_string(), "1".to_string()),
        ]
        .into(),
    ];
    let overrides = vec![[
        ("cluster".to_string(), "prod".to_string()),
        ("replicas".to_string(), "5".to_string()),
    ]
    .into()];
    let merged = evaluate_merge_generator(&base, &overrides, &["cluster".to_string()]);
    assert_eq!(merged[0]["replicas"], "5");
}

#[test]
fn applicationset_git_directory_generator_filters() {
    let gg = GitGenerator {
        repo_url: "https://github.com/example/config".into(),
        revision: Some("main".into()),
        directories: vec![GitDirectoryFilter {
            path: "clusters/*".into(),
            exclude: false,
        }],
        files: vec![],
        values: HashMap::new(),
        template: None,
        requeue_after_seconds: Some(180),
    };
    let paths = ["clusters/prod", "clusters/staging", "other/stuff"];
    let params = evaluate_git_directory_generator(&gg, &paths);
    assert_eq!(params.len(), 2);
    assert!(params
        .iter()
        .any(|p| p["path.basename"] == "prod"));
}

#[test]
fn applicationset_full_fixture_roundtrips() {
    let set = ApplicationSet {
        id: Uuid::new_v4(),
        name: "platform-apps".into(),
        namespace: "argocd".into(),
        spec: ApplicationSetSpec {
            generators: vec![Generator::List(ListGenerator {
                elements: vec![[("env".to_string(), "prod".to_string())].into()],
                template: None,
            })],
            template: ApplicationSetTemplate {
                metadata: ApplicationSetTemplateMetadata {
                    name: "{{env}}-app".into(),
                    namespace: Some("argocd".into()),
                    labels: Default::default(),
                    annotations: Default::default(),
                    finalizers: vec![],
                },
                spec: make_app("placeholder", "default", "https://github.com/example/app", "{{env}}")
                    .spec,
            },
            sync_policy: None,
            ignore_application_differences: vec![],
            template_patch: None,
            go_template: None,
            preserve_resources_on_deletion: false,
        },
        status: None,
    };
    let json = serde_json::to_string(&set).unwrap();
    let back: ApplicationSet = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "platform-apps");
    assert_eq!(back.spec.generators.len(), 1);
}

// ─── Sync engine dry-run ────────────────────────────────────────────────────

#[test]
fn sync_dry_run_does_not_touch_unrelated_status() {
    let store = DeployStore::new();
    let mut app = make_app("svc-a", "default", "https://github.com/example/svc-a", "prod");
    store.create_application(app.clone()).unwrap();
    let rev = sync_application(&mut app, Some("abc123".into()), true).unwrap();
    assert_eq!(rev, "abc123");
    // Store-side state remains untouched because the engine only mutates the
    // local copy; only update_application_status persists.
    assert!(store.get_application("svc-a").unwrap().status.is_none());
}

#[test]
fn manifest_render_carries_tracking_label() {
    let mut app = make_app("svc-b", "default", "https://github.com/example/svc-b", "prod");
    app.spec.source.helm = Some(HelmSource {
        value_files: vec![],
        values: String::new(),
        parameters: vec![],
        file_parameters: vec![],
        release_name: Some("svc-b".into()),
        chart: Some("my-chart".into()),
        skip_crds: false,
        pass_credentials: false,
    });
    let manifests = render_manifests(&app).unwrap();
    assert!(!manifests.is_empty());
    for m in &manifests {
        assert_eq!(m.raw["metadata"]["labels"][TRACKING_LABEL], "svc-b");
    }
}

#[test]
fn yaml_multi_doc_parser_handles_separators() {
    let yaml = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\n  namespace: ns\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: b\n  namespace: ns\n";
    let docs = parse_yaml_documents(yaml).unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].kind, "ConfigMap");
    assert_eq!(docs[1].kind, "Service");
}

#[test]
fn json_array_parser_yields_each_manifest() {
    let json = serde_json::json!([
        {"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"a","namespace":"n"}},
        {"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"b","namespace":"n"}},
    ]);
    let docs = parse_json_documents(&json.to_string()).unwrap();
    assert_eq!(docs.len(), 2);
}

#[test]
fn drift_then_auto_sync_round_trip() {
    let mut app = make_app("svc-c", "default", "https://github.com/example/svc-c", "prod");
    app.spec.sync_policy = Some(SyncPolicy {
        automated: Some(AutomatedSyncPolicy {
            prune: true,
            self_heal: true,
            allow_empty: false,
        }),
        sync_options: vec!["CreateNamespace=true".into()],
        retry: None,
        managed_namespace_metadata: None,
    });
    app.status = Some(ApplicationStatus {
        health: HealthCondition {
            status: HealthStatus::Healthy,
            message: None,
        },
        sync: SyncCondition {
            status: SyncStatus::OutOfSync,
            revision: "stale".into(),
            revisions: vec![],
        },
        resources: vec![],
        history: vec![],
        conditions: vec![],
        observed_at: Some(Utc::now()),
        reconciled_at: Some(Utc::now()),
    });
    assert!(detect_drift(&app, 1));
    let rev = auto_sync(&mut app).expect("auto-sync should trigger on OutOfSync");
    assert_eq!(rev, "main");
}

#[test]
fn diff_with_ignored_differences_filter() {
    let entries = compute_diff(
        &serde_json::json!({"spec": {"replicas": 3, "selector": {"app": "x"}}}),
        &serde_json::json!({"spec": {"replicas": 2, "selector": {"app": "y"}}}),
    );
    assert_eq!(entries.len(), 2);
    let ignored = vec![IgnoredDiff {
        json_pointer: Some("/spec/selector".into()),
        jq_expression: None,
    }];
    let filtered = apply_ignored_differences(entries, &ignored);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].path, "spec.replicas");
    assert_eq!(filtered[0].diff_type, DiffType::Modified);
}

// ─── RBAC evaluation ────────────────────────────────────────────────────────

#[test]
fn rbac_scope_repo_and_destination_pass() {
    let project = make_project("default");
    let dest = Destination {
        server: "https://kubernetes.default.svc".into(),
        name: None,
        namespace: "prod".into(),
    };
    assert!(is_source_allowed(&project, "https://github.com/example/svc"));
    assert!(is_destination_allowed(&project, &dest));
    let violations = validate_application(
        &project,
        "https://github.com/example/svc",
        &dest,
    );
    assert!(violations.is_empty());
}

#[test]
fn rbac_blocks_unknown_source_repo() {
    let project = make_project("default");
    let dest = Destination {
        server: "https://kubernetes.default.svc".into(),
        name: None,
        namespace: "prod".into(),
    };
    let v = validate_application(&project, "https://evil.com/repo", &dest);
    assert_eq!(v.len(), 1);
    assert!(matches!(v[0], ProjectViolation::SourceRepoNotAllowed { .. }));
}

#[test]
fn rbac_role_policy_allows_only_specified_actions() {
    let project = make_project("default");
    assert!(has_permission(
        &project,
        "deploy-team",
        "applications",
        &RbacAction::Sync
    ));
    assert!(has_permission(
        &project,
        "deploy-team",
        "applications",
        &RbacAction::Get
    ));
    assert!(!has_permission(
        &project,
        "deploy-team",
        "applications",
        &RbacAction::Delete
    ));
}

// ─── PreSync → Sync → PostSync hook lifecycle ───────────────────────────────

#[test]
fn hook_lifecycle_full_three_phases() {
    let mut presync_anno = HashMap::new();
    presync_anno.insert("argocd.argoproj.io/hook".into(), "PreSync".into());
    presync_anno.insert("argocd.argoproj.io/hook-delete-policy".into(), "HookSucceeded".into());

    let mut sync_anno = HashMap::new();
    sync_anno.insert("argocd.argoproj.io/sync-wave".into(), "1".into());

    let mut postsync_anno = HashMap::new();
    postsync_anno.insert("argocd.argoproj.io/hook".into(), "PostSync".into());

    let presync_phases = parse_hook_phases(&presync_anno);
    let sync_phases = parse_hook_phases(&sync_anno);
    let postsync_phases = parse_hook_phases(&postsync_anno);

    assert_eq!(presync_phases, vec![SyncPhase::PreSync]);
    assert!(sync_phases.is_empty()); // No hook annotation = regular Sync wave member
    assert_eq!(postsync_phases, vec![SyncPhase::PostSync]);

    assert!(parse_delete_on_success(&presync_anno));
    assert_eq!(parse_wave(&sync_anno), 1);

    let resources = vec![
        ManifestResource {
            group: "batch".into(),
            version: "v1".into(),
            kind: "Job".into(),
            namespace: "ns".into(),
            name: "db-migrate".into(),
            wave: -10,
            hook_phases: presync_phases,
            delete_on_success: true,
            manifest: serde_json::json!({}),
        },
        ManifestResource {
            group: "apps".into(),
            version: "v1".into(),
            kind: "Deployment".into(),
            namespace: "ns".into(),
            name: "api".into(),
            wave: 1,
            hook_phases: vec![],
            delete_on_success: false,
            manifest: serde_json::json!({}),
        },
        ManifestResource {
            group: "batch".into(),
            version: "v1".into(),
            kind: "Job".into(),
            namespace: "ns".into(),
            name: "smoke-test".into(),
            wave: 10,
            hook_phases: postsync_phases,
            delete_on_success: false,
            manifest: serde_json::json!({}),
        },
    ];

    // PreSync hook is detectable
    assert!(resources[0].is_hook());
    assert!(resources[0].is_in_phase(&SyncPhase::PreSync));
    // Sync wave member is not a hook
    assert!(!resources[1].is_hook());
    // PostSync hook is detectable
    assert!(resources[2].is_hook());
    assert!(resources[2].is_in_phase(&SyncPhase::PostSync));

    // Group-by-wave puts the PreSync Job first (wave -10), Deployment next
    // (wave 1), PostSync Job last (wave 10)
    let waves = group_by_wave(&resources);
    assert_eq!(waves.len(), 3);
    assert_eq!(waves[0].0, -10);
    assert_eq!(waves[0].1[0].name, "db-migrate");
    assert_eq!(waves[1].0, 1);
    assert_eq!(waves[1].1[0].name, "api");
    assert_eq!(waves[2].0, 10);
    assert_eq!(waves[2].1[0].name, "smoke-test");
}

#[test]
fn syncfail_phase_parses() {
    let mut anno = HashMap::new();
    anno.insert("argocd.argoproj.io/hook".into(), "SyncFail".into());
    let phases = parse_hook_phases(&anno);
    assert_eq!(phases, vec![SyncPhase::SyncFail]);
}

// ─── Rollback path ──────────────────────────────────────────────────────────

#[test]
fn rollback_walks_history_to_target_revision() {
    let store = DeployStore::new();
    let app = make_app("rb", "default", "https://github.com/example/rb", "prod");
    let source = app.spec.source.clone();
    let app_id = app.id;
    store.create_application(app).unwrap();

    let h1 = RevisionHistory {
        id: 1,
        revision: "v1.0.0".into(),
        deployed_at: Utc::now(),
        initiated_by: "ci-bot".into(),
        source: source.clone(),
    };
    let h2 = RevisionHistory {
        id: 2,
        revision: "v2.0.0".into(),
        deployed_at: Utc::now(),
        initiated_by: "alice".into(),
        source: source.clone(),
    };
    store.append_revision("rb", h1.clone()).unwrap();
    store.append_revision("rb", h2.clone()).unwrap();

    let req = RollbackRequest {
        application_id: app_id,
        history_id: 1,
        prune: true,
        dry_run: false,
        initiated_by: "alice".into(),
    };
    let res = initiate_rollback(&req, &[h1, h2]).unwrap();
    assert_eq!(res.target_revision, "v1.0.0");
    assert_eq!(res.application_id, app_id);

    let target = store.rollback_to_history_id("rb", 1).unwrap();
    assert_eq!(target.revision, "v1.0.0");
}

// ─── Rollout strategies ─────────────────────────────────────────────────────

#[test]
fn canary_then_promote_then_traffic_full() {
    let mut r = canary_deploy(Uuid::new_v4(), "v2".into(), vec![]);
    assert_eq!(r.strategy, RolloutStrategy::Canary);
    assert_eq!(r.status, RolloutStatus::Progressing);
    assert!(promote_canary(&mut r));
    assert_eq!(r.status, RolloutStatus::Promoting);
    traffic_split(&mut r, 100);
    assert_eq!(r.status, RolloutStatus::Completed);
    assert_eq!(r.stable_revision, "v2");
}

#[test]
fn rollback_aborts_in_flight_rollout() {
    let mut r = blue_green_deploy(Uuid::new_v4(), "v2".into());
    r.traffic_weight = 50;
    rollback(&mut r);
    assert_eq!(r.status, RolloutStatus::Aborting);
    assert_eq!(r.traffic_weight, 0);
}

#[test]
fn rolling_update_has_four_step_progression() {
    let r = rolling_update(Uuid::new_v4(), "v3".into());
    assert_eq!(r.strategy, RolloutStrategy::Rolling);
    assert_eq!(r.steps.len(), 4);
    assert_eq!(r.steps[0].weight, 25);
    assert_eq!(r.steps[3].weight, 100);
}

// ─── Health assessor ────────────────────────────────────────────────────────

#[test]
fn health_registry_routes_by_kind() {
    let reg = HealthCheckRegistry::new();
    let dep = serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "spec": { "replicas": 3 },
        "status": { "availableReplicas": 3, "readyReplicas": 3, "updatedReplicas": 3 }
    });
    assert_eq!(reg.assess(&dep).status, HealthStatus::Healthy);

    let ingress_no_lb = serde_json::json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "Ingress",
        "status": {}
    });
    assert_eq!(check_ingress(&ingress_no_lb).status, HealthStatus::Progressing);

    let svc_with_lb = serde_json::json!({
        "spec": { "type": "LoadBalancer" },
        "status": { "loadBalancer": { "ingress": [{"ip": "1.2.3.4"}] } }
    });
    assert_eq!(check_service(&svc_with_lb).status, HealthStatus::Healthy);
}

#[test]
fn deployment_progressing_when_replicas_short() {
    let dep = serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "spec": { "replicas": 5 },
        "status": { "availableReplicas": 2, "readyReplicas": 2, "updatedReplicas": 2 }
    });
    assert_eq!(check_deployment(&dep).status, HealthStatus::Progressing);
}

// ─── Notifications ──────────────────────────────────────────────────────────

#[test]
fn notification_engine_filters_by_trigger() {
    let app = make_app("svc-d", "default", "https://github.com/example/svc-d", "prod");
    let subs = vec![NotificationConfig {
        id: Uuid::new_v4(),
        name: "on-success".into(),
        triggers: vec![NotificationTrigger::OnSyncSucceeded],
        destination: NotificationDestination::Slack {
            channel: "#deploys".into(),
        },
        template: String::new(),
    }];
    let engine = NotificationEngine::new(subs);
    assert_eq!(engine.subscriptions().len(), 1);
    let payload = build_slack_payload(&app, "synced ok", "#36a64f", "#deploys");
    assert_eq!(payload["channel"], "#deploys");
    assert_eq!(payload["attachments"][0]["color"], "#36a64f");
    assert_eq!(slack_color(&NotificationTrigger::OnHealthDegraded), "#ff0000");
}

// ─── Cluster URL builders ──────────────────────────────────────────────────

#[test]
fn cluster_resource_url_round_trip() {
    assert_eq!(kind_to_plural("CronJob"), "cronjobs");
    assert_eq!(kind_to_plural("Ingress"), "ingresses");
    assert_eq!(
        build_resource_url(
            "networking.k8s.io/v1",
            "Ingress",
            "frontend",
            Some("prod")
        ),
        "/apis/networking.k8s.io/v1/namespaces/prod/ingresses/frontend"
    );
}

// ─── Sync options ───────────────────────────────────────────────────────────

#[test]
fn sync_options_full_set_parses() {
    let opts = parse_sync_options(&[
        "CreateNamespace=true".to_string(),
        "ServerSideApply=true".to_string(),
        "PruneLast=true".to_string(),
        "ApplyOutOfSyncOnly=true".to_string(),
        "Replace=true".to_string(),
        "RespectIgnoreDifferences=true".to_string(),
        "Validate=false".to_string(),
    ]);
    assert!(opts.create_namespace);
    assert!(opts.server_side_apply);
    assert!(opts.prune_last);
    assert!(opts.apply_out_of_sync_only);
    assert!(opts.replace);
    assert!(opts.respect_ignore_differences);
    assert!(!opts.validate);
}

#[test]
fn auto_sync_evaluator_respects_policy_shape() {
    let no_policy = SyncPolicy::default();
    assert!(!should_auto_sync(
        &no_policy,
        &SyncStatus::OutOfSync,
        &HealthStatus::Healthy
    ));

    let auto = SyncPolicy {
        automated: Some(AutomatedSyncPolicy {
            prune: true,
            self_heal: false,
            allow_empty: false,
        }),
        sync_options: vec![],
        retry: None,
        managed_namespace_metadata: None,
    };
    let res = ResourceStatus {
        group: "".into(),
        version: "v1".into(),
        kind: "ConfigMap".into(),
        namespace: "n".into(),
        name: "leftover".into(),
        status: SyncStatus::Synced,
        health: None,
        hook: false,
        require_pruning: true,
    };
    assert!(should_prune(&auto, &res));
}

// ─── Diff normalization ─────────────────────────────────────────────────────

#[test]
fn normalize_drops_server_fields() {
    let raw = serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "x",
            "namespace": "ns",
            "uid": "abc",
            "resourceVersion": "1",
            "creationTimestamp": "2024",
            "managedFields": [],
            "generation": 1,
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "...",
                "user.annotation": "keep"
            }
        },
        "spec": { "replicas": 1 },
        "status": { "availableReplicas": 1 }
    });
    let n = normalize_resource(&raw);
    let meta = &n["metadata"];
    assert!(meta["uid"].is_null());
    assert!(meta["resourceVersion"].is_null());
    assert!(meta["managedFields"].is_null());
    assert!(meta["annotations"]["kubectl.kubernetes.io/last-applied-configuration"].is_null());
    assert_eq!(meta["annotations"]["user.annotation"], "keep");
    assert!(n["status"].is_null());
}
