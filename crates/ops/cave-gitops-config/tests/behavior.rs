// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral coverage for cave-gitops-config wired engine/store logic.
//!
//! These tests exercise portable-coverage gaps: public behaviors the crate
//! already implements but does not yet test. Several port upstream
//! Argo CD v3.4.2 semantics (desired-vs-live comparison, validation/skip
//! propagation on a failed pipeline stage), while the rest cover the
//! crate's Kratix-style Promise pipeline and in-memory store.

use cave_gitops_config::engine::PipelineEngine;
use cave_gitops_config::models::{
    ClusterDestination, ClusterStatus, PipelineRun, PipelineRunStatus, PipelineStage,
    PipelineStageType, Promise, PromiseStatus, ResourceRequest, ResourceRequestStatus, StageStatus,
};
use cave_gitops_config::store::GitOpsStore;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ─── Builders ───────────────────────────────────────────────────────────────

fn make_promise(name: &str, stages: Vec<PipelineStage>) -> Promise {
    Promise {
        id: Uuid::new_v4(),
        name: name.to_string(),
        version: "1.0.0".to_string(),
        description: "test promise".to_string(),
        api_schema: serde_json::json!({}),
        pipeline: stages,
        dependencies: vec![],
        destination_selectors: vec![],
        status: PromiseStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_request_with_spec(promise_name: &str, spec: serde_json::Value) -> ResourceRequest {
    ResourceRequest {
        id: Uuid::new_v4(),
        promise_name: promise_name.to_string(),
        promise_version: "1.0.0".to_string(),
        namespace: "default".to_string(),
        name: "my-db".to_string(),
        spec,
        requester: Uuid::new_v4(),
        status: ResourceRequestStatus::Pending,
        pipeline_run: None,
        destinations: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_stage(
    name: &str,
    stage_type: PipelineStageType,
    config: serde_json::Value,
) -> PipelineStage {
    PipelineStage {
        name: name.to_string(),
        description: "test stage".to_string(),
        stage_type,
        config,
        order: 0,
    }
}

fn make_cluster(name: &str, status: ClusterStatus) -> ClusterDestination {
    ClusterDestination {
        name: name.to_string(),
        api_server: format!("https://{name}.k8s.example.com"),
        labels: HashMap::new(),
        status,
        registered_at: Utc::now(),
    }
}

fn make_pipeline_run(id: Uuid, resource_request_id: Uuid, status: PipelineRunStatus) -> PipelineRun {
    PipelineRun {
        id,
        resource_request_id,
        promise_name: "postgresql".to_string(),
        stages: vec![],
        status,
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
    }
}

// ─── Engine: run_pipeline failure + skip propagation ────────────────────────

/// A failing Validate stage marks the run Failed and every later stage Skipped.
/// An empty-object spec drives `validate_spec_basic` to its non-empty-object
/// error branch (`"spec must not be empty"`), so the first stage fails and
/// `run_pipeline` short-circuits the rest into `Skipped`. Mirrors Argo CD's
/// incomplete-state handling where downstream work does not run.
#[test]
fn test_run_pipeline_validate_failure_skips_remaining() {
    let stages = vec![
        make_stage("validate", PipelineStageType::Validate, serde_json::json!({})),
        make_stage("deploy", PipelineStageType::Deploy, serde_json::json!({})),
        make_stage("notify", PipelineStageType::Notify, serde_json::json!({})),
    ];
    let promise = make_promise("postgresql", stages);
    // Empty object spec => validate_spec_basic => Err(["spec must not be empty"]).
    let request = make_request_with_spec("postgresql", serde_json::json!({}));

    let run = PipelineEngine::run_pipeline(&promise, &request);

    assert_eq!(run.status, PipelineRunStatus::Failed);
    assert_eq!(run.stages.len(), 3);
    assert_eq!(run.stages[0].status, StageStatus::Failed);
    assert_eq!(
        run.stages[0].error.as_deref(),
        Some("spec must not be empty")
    );
    assert_eq!(run.stages[1].status, StageStatus::Skipped);
    assert_eq!(run.stages[2].status, StageStatus::Skipped);
    // Skipped stages carry a null output and no error.
    assert_eq!(run.stages[1].output, serde_json::Value::Null);
    assert!(run.stages[1].error.is_none());
}

/// The Configure stage *overwrites* existing keys (it uses `insert`), unlike
/// Transform which keeps existing keys (`or_insert`). With a request spec of
/// `{"replicas": 1}` and a Configure config of `{"replicas": 5}`, the stage
/// output must reflect the config value (5), not the original (1).
#[test]
fn test_configure_stage_overwrites_keys() {
    let stages = vec![make_stage(
        "configure",
        PipelineStageType::Configure,
        serde_json::json!({"replicas": 5}),
    )];
    let promise = make_promise("postgresql", stages);
    let request = make_request_with_spec("postgresql", serde_json::json!({"replicas": 1}));

    let run = PipelineEngine::run_pipeline(&promise, &request);

    assert_eq!(run.status, PipelineRunStatus::Completed);
    assert_eq!(run.stages.len(), 1);
    assert_eq!(run.stages[0].status, StageStatus::Completed);
    assert_eq!(run.stages[0].output["replicas"], 5);
}

/// The Deploy stage emits `{path, deployed: true}` where `path` is the
/// canonical state-store path computed from the hardcoded `"default-cluster"`,
/// the request's promise name, namespace, and resource name.
#[test]
fn test_deploy_stage_outputs_state_path() {
    let stages = vec![make_stage(
        "deploy",
        PipelineStageType::Deploy,
        serde_json::json!({}),
    )];
    let promise = make_promise("postgresql", stages);
    // Non-empty object spec so this stage is reached; namespace=default, name=my-db.
    let request = make_request_with_spec("postgresql", serde_json::json!({"storage": "10Gi"}));

    let run = PipelineEngine::run_pipeline(&promise, &request);

    assert_eq!(run.stages.len(), 1);
    assert_eq!(run.stages[0].status, StageStatus::Completed);
    assert_eq!(run.stages[0].output["deployed"], true);
    assert_eq!(
        run.stages[0].output["path"],
        "clusters/default-cluster/postgresql/default/my-db.yaml"
    );
    // And that path matches the public path builder for the same inputs.
    assert_eq!(
        run.stages[0].output["path"].as_str().unwrap(),
        PipelineEngine::state_store_path("default-cluster", "postgresql", "default", "my-db")
    );
}

/// The Notify stage emits `{notified: true}` and completes successfully.
#[test]
fn test_notify_stage_outputs_notified() {
    let stages = vec![make_stage(
        "notify",
        PipelineStageType::Notify,
        serde_json::json!({}),
    )];
    let promise = make_promise("postgresql", stages);
    let request = make_request_with_spec("postgresql", serde_json::json!({"storage": "10Gi"}));

    let run = PipelineEngine::run_pipeline(&promise, &request);

    assert_eq!(run.stages.len(), 1);
    assert_eq!(run.stages[0].status, StageStatus::Completed);
    assert_eq!(run.stages[0].output["notified"], true);
}

// ─── Engine: validate_spec non-object rejection ─────────────────────────────

/// `validate_spec` rejects a non-object spec up front with a single
/// `"spec must be a JSON object"` error, regardless of the promise schema.
#[test]
fn test_validate_spec_rejects_non_object() {
    let promise = make_promise("postgresql", vec![]);

    // A JSON array is not an object.
    let arr = serde_json::json!([1, 2, 3]);
    let err = PipelineEngine::validate_spec(&promise, &arr).unwrap_err();
    assert_eq!(err, vec!["spec must be a JSON object".to_string()]);

    // A JSON string is not an object either.
    let s = serde_json::json!("not-an-object");
    let err = PipelineEngine::validate_spec(&promise, &s).unwrap_err();
    assert_eq!(err, vec!["spec must be a JSON object".to_string()]);
}

// ─── Engine: select_destinations with empty selectors ───────────────────────

/// A promise with no destination selectors matches *all* Ready clusters (the
/// `.all()` over an empty selector list is vacuously true), while NotReady
/// clusters are still excluded.
#[test]
fn test_select_destinations_empty_selectors_matches_all_ready() {
    let promise = make_promise("postgresql", vec![]); // no destination_selectors
    let clusters = vec![
        make_cluster("ready-a", ClusterStatus::Ready),
        make_cluster("not-ready", ClusterStatus::NotReady),
        make_cluster("ready-b", ClusterStatus::Ready),
    ];

    let selected = PipelineEngine::select_destinations(&promise, &clusters);

    assert_eq!(selected, vec!["ready-a".to_string(), "ready-b".to_string()]);
}

// ─── Store: update_resource_request_status ──────────────────────────────────

/// `update_resource_request_status` persists a new status, attaches the given
/// pipeline run, replaces destinations, and bumps `updated_at`; it returns
/// `false` for an unknown id without mutating anything.
#[test]
fn test_update_resource_request_status_sets_run_and_destinations() {
    let store = GitOpsStore::new();
    let req = make_request_with_spec("postgresql", serde_json::json!({}));
    let id = req.id;
    let original_updated_at = req.updated_at;
    store.create_resource_request(req);

    let run = make_pipeline_run(Uuid::new_v4(), id, PipelineRunStatus::Completed);
    let ok = store.update_resource_request_status(
        id,
        ResourceRequestStatus::Ready,
        Some(run.clone()),
        Some(vec!["prod-a".to_string(), "prod-b".to_string()]),
    );
    assert!(ok);

    let fetched = store.get_resource_request(id).unwrap();
    assert_eq!(fetched.status, ResourceRequestStatus::Ready);
    assert_eq!(fetched.pipeline_run.as_ref().unwrap().id, run.id);
    assert_eq!(
        fetched.destinations,
        vec!["prod-a".to_string(), "prod-b".to_string()]
    );
    assert!(fetched.updated_at >= original_updated_at);

    // Unknown id => false.
    assert!(!store.update_resource_request_status(
        Uuid::new_v4(),
        ResourceRequestStatus::Failed,
        None,
        None,
    ));
}

// ─── Store: delete_resource_request ─────────────────────────────────────────

/// `delete_resource_request` returns `true` and removes the request when it
/// exists, and `false` for an unknown id.
#[test]
fn test_delete_resource_request() {
    let store = GitOpsStore::new();
    let req = make_request_with_spec("postgresql", serde_json::json!({}));
    let id = req.id;
    store.create_resource_request(req);

    assert!(store.delete_resource_request(id));
    assert!(store.get_resource_request(id).is_none());

    // Deleting again (now unknown) => false.
    assert!(!store.delete_resource_request(id));
}

// ─── Store: update_pipeline_run ─────────────────────────────────────────────

/// `update_pipeline_run` replaces an existing run (matched by run id) so a
/// subsequent lookup by resource_request_id reflects the new status; an
/// unknown run id returns `false`.
#[test]
fn test_update_pipeline_run_replaces_existing() {
    let store = GitOpsStore::new();
    let rr_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let run = make_pipeline_run(run_id, rr_id, PipelineRunStatus::Running);
    store.add_pipeline_run(run);

    let updated = make_pipeline_run(run_id, rr_id, PipelineRunStatus::Completed);
    assert!(store.update_pipeline_run(run_id, updated));

    let fetched = store.get_pipeline_run(rr_id).unwrap();
    assert_eq!(fetched.status, PipelineRunStatus::Completed);

    // Unknown run id => false.
    let stray = make_pipeline_run(Uuid::new_v4(), rr_id, PipelineRunStatus::Failed);
    assert!(!store.update_pipeline_run(Uuid::new_v4(), stray));
}
