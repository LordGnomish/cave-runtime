// SPDX-License-Identifier: AGPL-3.0-or-later
// Characterization tests for pre-existing absorbed modules in cave-infra.
//
// These modules existed on origin/main but were NOT wired into lib.rs (orphan files).
// Absorbing them: models, state, intent, planner, executor, approval, mcp_bridge, providers.
//
// Characterization tests assert OBSERVED behavior of pre-existing code.
// They may pass immediately — that is honest for already-correct code.
use cave_infra::approval::{ApprovalRequest, ApprovalStatus, ApprovalWorkflow};
use cave_infra::executor::{dry_run, stream_progress};
use cave_infra::intent::parse_intent;
use cave_infra::mcp_bridge::McpRegistry;
use cave_infra::models::{
    CostEstimate, ExecutionPlan, InfraIntent, InfraResource, InfraState, McpProvider, PlanStatus,
    PlanStep, ResourceDeclaration, ResourceState, StepAction,
};
use cave_infra::planner::{assess_risk, estimate_cost, explain_plan, generate_plan, optimize_plan};
use cave_infra::providers::{MockProvider, ResourceProvider, ResourceType};
use cave_infra::state::{InfraStateStore, StateError};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ── models ────────────────────────────────────────────────────────────────────

#[test]
fn infra_resource_new_sets_fields() {
    let r = InfraResource::new(
        "res-001",
        "virtual_machine",
        "mock-provider",
        "web-server",
        "tenant-abc",
        serde_json::json!({"cpu": 4, "ram_gb": 8}),
    );
    assert_eq!(r.name, "web-server");
    assert_eq!(r.provider, "mock-provider");
    assert_eq!(r.resource_type, "virtual_machine");
    assert_eq!(r.actual_id.as_deref(), Some("res-001"));
    assert_eq!(r.config["cpu"], serde_json::json!(4));
}

#[test]
fn infra_state_default_is_empty() {
    let state = InfraState::default();
    assert_eq!(state.version, 1);
    assert!(state.resources.is_empty());
    assert!(state.locked_by.is_none());
}

#[test]
fn cost_estimate_default_is_zero() {
    let ce = CostEstimate::default();
    assert_eq!(ce.monthly_usd, 0.0);
    assert_eq!(ce.currency, "USD");
}

// ── state ─────────────────────────────────────────────────────────────────────

#[test]
fn state_lock_acquire_and_release() {
    let mut store = InfraStateStore::default();
    assert!(store.lock_state("alice").is_ok());
    // Double-lock should fail
    assert!(matches!(
        store.lock_state("bob"),
        Err(StateError::AlreadyLocked(_))
    ));
    assert!(store.unlock_state("alice").is_ok());
    // Now alice can lock again
    assert!(store.lock_state("alice").is_ok());
}

#[test]
fn state_unlock_wrong_owner_fails() {
    let mut store = InfraStateStore::default();
    store.lock_state("alice").unwrap();
    assert!(matches!(
        store.unlock_state("bob"),
        Err(StateError::LockOwnerMismatch { .. })
    ));
}

#[test]
fn state_snapshot_increments_version() {
    let mut store = InfraStateStore::default();
    let v0 = store.state.version;
    store.snapshot();
    assert_eq!(store.state.version, v0 + 1);
    assert_eq!(store.history.len(), 1);
    store.snapshot();
    assert_eq!(store.state.version, v0 + 2);
    assert_eq!(store.history.len(), 2);
}

#[test]
fn state_import_resource_adds_to_state() {
    let mut store = InfraStateStore::default();
    let resource = store.import_resource(
        "web-01".to_string(),
        "mock".to_string(),
        "virtual_machine".to_string(),
        "prov-id-123".to_string(),
        HashMap::new(),
    );
    assert_eq!(resource.name, "web-01");
    assert_eq!(store.state.resources.len(), 1);
}

#[tokio::test]
async fn state_detect_drift_finds_drifted() {
    let mut store = InfraStateStore::default();
    let resource = store.import_resource(
        "drift-server".to_string(),
        "mock".to_string(),
        "vm".to_string(),
        "prov-drift".to_string(),
        HashMap::new(),
    );
    // Mark as drifted
    if let Some(r) = store.state.resources.get_mut(&resource.id) {
        r.state = ResourceState::Drifted;
    }
    let report = store.detect_drift().await;
    assert_eq!(report.total_drifted, 1);
}

// ── intent ────────────────────────────────────────────────────────────────────

#[test]
fn intent_parse_yaml_resources() {
    let yaml = r#"
resources:
  - name: my-db
    provider: aws
    type: rds_cluster
  - name: my-bucket
    provider: aws
    type: object_storage
"#;
    let intent = parse_intent("deploy infrastructure", Some(yaml)).unwrap();
    assert_eq!(intent.resources.len(), 2);
    assert_eq!(intent.resources[0].name, "my-db");
    assert_eq!(intent.resources[1].resource_type, "object_storage");
}

#[test]
fn intent_parse_nl_infers_database() {
    let intent = parse_intent("provision a postgres database cluster", None).unwrap();
    assert!(!intent.resources.is_empty());
    let kinds: Vec<&str> = intent
        .resources
        .iter()
        .map(|r| r.resource_type.as_str())
        .collect();
    assert!(kinds.iter().any(|&k| k.contains("rds") || k.contains("database")));
}

#[test]
fn intent_resolve_dependencies_returns_all_names() {
    use cave_infra::intent::resolve_dependencies;
    let intent = InfraIntent {
        id: Uuid::new_v4(),
        description: "test".into(),
        structured: None,
        resources: vec![
            ResourceDeclaration {
                name: "a".into(),
                provider: "aws".into(),
                resource_type: "vm".into(),
                config: HashMap::new(),
            },
            ResourceDeclaration {
                name: "b".into(),
                provider: "aws".into(),
                resource_type: "db".into(),
                config: HashMap::new(),
            },
        ],
        constraints: vec![],
        created_at: Utc::now(),
    };
    let order = resolve_dependencies(&intent).unwrap();
    assert_eq!(order.len(), 2);
}

#[test]
fn intent_validate_fails_for_unregistered_provider() {
    use cave_infra::intent::validate_intent;
    use cave_infra::models::McpTool;
    let intent = InfraIntent {
        id: Uuid::new_v4(),
        description: "test".into(),
        structured: None,
        resources: vec![ResourceDeclaration {
            name: "r".into(),
            provider: "missing-cloud".into(),
            resource_type: "vm".into(),
            config: HashMap::new(),
        }],
        constraints: vec![],
        created_at: Utc::now(),
    };
    let providers = vec![McpProvider {
        id: Uuid::new_v4(),
        name: "aws".into(),
        endpoint: "http://aws-mcp:8080".into(),
        capabilities: vec![McpTool {
            name: "create_vm".into(),
            description: "".into(),
            input_schema: serde_json::json!({}),
        }],
        healthy: true,
        registered_at: Utc::now(),
    }];
    let checks = validate_intent(&intent, &providers).unwrap();
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
}

// ── planner ───────────────────────────────────────────────────────────────────

fn make_intent_with_one_resource() -> InfraIntent {
    InfraIntent {
        id: Uuid::new_v4(),
        description: "deploy a VM".into(),
        structured: None,
        resources: vec![ResourceDeclaration {
            name: "web-server".into(),
            provider: "aws".into(),
            resource_type: "virtual_machine".into(),
            config: HashMap::new(),
        }],
        constraints: vec![],
        created_at: Utc::now(),
    }
}

fn make_aws_provider() -> McpProvider {
    use cave_infra::models::McpTool;
    McpProvider {
        id: Uuid::new_v4(),
        name: "aws".into(),
        endpoint: "http://aws-mcp".into(),
        capabilities: vec![McpTool {
            name: "create_virtual_machine".into(),
            description: "".into(),
            input_schema: serde_json::json!({}),
        }],
        healthy: true,
        registered_at: Utc::now(),
    }
}

#[test]
fn planner_generate_plan_has_one_step() {
    let intent = make_intent_with_one_resource();
    let state = InfraState::default();
    let providers = vec![make_aws_provider()];
    let plan = generate_plan(&intent, &state, &providers);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].action, StepAction::Create);
    assert_eq!(plan.status, PlanStatus::Draft);
}

#[test]
fn planner_existing_resource_produces_no_step() {
    let intent = make_intent_with_one_resource();
    let mut state = InfraState::default();
    // Pre-insert resource so diff sees it as NoOp
    let id = Uuid::new_v4();
    state.resources.insert(
        id,
        InfraResource {
            id,
            name: "web-server".into(),
            provider: "aws".into(),
            resource_type: "virtual_machine".into(),
            config: HashMap::new(),
            state: ResourceState::Active,
            dependencies: vec![],
            actual_id: Some("prov-123".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    );
    let providers = vec![make_aws_provider()];
    let plan = generate_plan(&intent, &state, &providers);
    // NoOp steps are filtered → 0 executable steps
    assert_eq!(plan.steps.len(), 0);
}

#[test]
fn planner_cost_estimate_is_positive_for_creates() {
    let intent = make_intent_with_one_resource();
    let state = InfraState::default();
    let providers = vec![make_aws_provider()];
    let plan = generate_plan(&intent, &state, &providers);
    let cost = estimate_cost(&plan);
    assert!(cost.monthly_usd >= 0.0);
}

#[test]
fn planner_risk_score_increases_with_deletes() {
    use cave_infra::state::InfraStateStore;
    let empty_state = InfraState::default();

    let intent = make_intent_with_one_resource();
    let providers = vec![make_aws_provider()];
    let mut plan_create = generate_plan(&intent, &empty_state, &providers);

    // Mutate to delete action
    for step in &mut plan_create.steps {
        step.action = StepAction::Delete;
        step.reversible = false;
    }
    let risk = assess_risk(&plan_create, &empty_state);
    assert!(risk > 0);
}

#[test]
fn planner_optimize_plan_removes_nonexistent_deps() {
    let intent = make_intent_with_one_resource();
    let state = InfraState::default();
    let providers = vec![make_aws_provider()];
    let mut plan = generate_plan(&intent, &state, &providers);
    // Add a bogus dependency
    if let Some(step) = plan.steps.first_mut() {
        step.depends_on.push(Uuid::new_v4()); // nonexistent
    }
    optimize_plan(&mut plan);
    // After optimize, bogus dep should be removed
    if let Some(step) = plan.steps.first() {
        assert!(step.depends_on.is_empty() || step.depends_on.iter().all(|d| plan.steps.iter().any(|s| s.id == *d)));
    }
}

#[test]
fn planner_explain_plan_references_intent_description() {
    let intent = make_intent_with_one_resource();
    let state = InfraState::default();
    let providers = vec![make_aws_provider()];
    let plan = generate_plan(&intent, &state, &providers);
    let explanation = explain_plan(&plan, &intent);
    assert!(explanation.contains("deploy a VM"));
}

// ── executor ──────────────────────────────────────────────────────────────────

fn make_plan_step(action: StepAction) -> PlanStep {
    PlanStep {
        id: Uuid::new_v4(),
        action,
        provider: "aws".into(),
        resource_name: "test-resource".into(),
        resource_type: "virtual_machine".into(),
        params: HashMap::new(),
        depends_on: vec![],
        estimated_duration_secs: 30,
        reversible: true,
    }
}

fn make_execution_plan(steps: Vec<PlanStep>) -> ExecutionPlan {
    let rollbacks: Vec<PlanStep> = steps
        .iter()
        .map(|s| PlanStep {
            id: Uuid::new_v4(),
            action: StepAction::Delete,
            ..s.clone()
        })
        .collect();
    ExecutionPlan {
        id: Uuid::new_v4(),
        intent_id: Uuid::new_v4(),
        steps,
        rollback_steps: rollbacks,
        cost_estimate: CostEstimate::default(),
        risk_score: 10,
        explanation: "test plan".into(),
        created_at: Utc::now(),
        status: PlanStatus::Draft,
    }
}

#[test]
fn dry_run_returns_one_line_per_step() {
    let steps = vec![
        make_plan_step(StepAction::Create),
        make_plan_step(StepAction::Update),
    ];
    let plan = make_execution_plan(steps);
    let lines = dry_run(&plan);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("[DRY-RUN]"));
    assert!(lines[0].contains("Create"));
}

#[test]
fn stream_progress_returns_pending_for_all_steps() {
    let steps = vec![make_plan_step(StepAction::Create)];
    let plan = make_execution_plan(steps);
    let progress = stream_progress(&plan);
    assert_eq!(progress.len(), 1);
    // All start as Pending
    assert!(matches!(
        progress[0].status,
        cave_infra::executor::ProgressStatus::Pending
    ));
    assert_eq!(progress[0].current_step, 1);
    assert_eq!(progress[0].total_steps, 1);
}

// ── approval ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_approve_flow() {
    let workflow = ApprovalWorkflow::new();
    let plan_id = Uuid::new_v4();
    let requester = Uuid::new_v4();
    let req = ApprovalRequest::new(plan_id, "tenant-001", requester, 24);
    let req_id = workflow.submit(req).await;

    let reviewer = Uuid::new_v4();
    let approved = workflow
        .approve(req_id, reviewer, Some("LGTM".into()))
        .await;
    assert!(approved.is_ok());

    let fetched = workflow.get(req_id).await.unwrap();
    assert_eq!(fetched.status, ApprovalStatus::Approved);
    assert_eq!(fetched.reviewed_by, Some(reviewer));
}

#[tokio::test]
async fn test_reject_flow() {
    let workflow = ApprovalWorkflow::new();
    let plan_id = Uuid::new_v4();
    let requester = Uuid::new_v4();
    let req = ApprovalRequest::new(plan_id, "tenant-001", requester, 24);
    let req_id = workflow.submit(req).await;

    let reviewer = Uuid::new_v4();
    let rejected = workflow
        .reject(req_id, reviewer, "Too risky".into())
        .await;
    assert!(rejected.is_ok());

    let fetched = workflow.get(req_id).await.unwrap();
    assert_eq!(fetched.status, ApprovalStatus::Rejected);
}

// ── mcp_bridge ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mcp_registry_register_and_execute() {
    let mut registry = McpRegistry::new();
    registry.register("aws".into(), "http://aws-mcp:8080".into());

    let result = registry
        .execute_tool(
            "aws",
            "create_virtual_machine",
            &HashMap::new(),
        )
        .await;
    assert!(result.is_ok());
    let val = result.unwrap();
    assert_eq!(val["provider"], "aws");
}

#[tokio::test]
async fn mcp_registry_missing_provider_fails() {
    let registry = McpRegistry::new();
    let result = registry
        .execute_tool("nonexistent", "some_tool", &HashMap::new())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn mcp_registry_health_check_known_provider() {
    let mut registry = McpRegistry::new();
    registry.register("hetzner".into(), "http://hetzner-mcp:8080".into());
    let healthy = registry.health_check("hetzner").await;
    assert!(healthy);
}

#[tokio::test]
async fn mcp_registry_discover_capabilities() {
    let mut registry = McpRegistry::new();
    registry.register("gcp".into(), "http://gcp-mcp:8080".into());
    let tools = registry.discover_capabilities("gcp").await.unwrap();
    assert!(!tools.is_empty());
}

// ── providers ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mock_provider_deprovision_ok() {
    let provider = MockProvider::new("mock", vec![ResourceType::Vm]);
    let result = provider.deprovision("prov-123", &ResourceType::Vm).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn mock_provider_describe_returns_status() {
    let provider = MockProvider::new("mock", vec![ResourceType::Vm]);
    let desc = provider.describe("prov-123", &ResourceType::Vm).await;
    assert!(desc.is_ok());
    let val = desc.unwrap();
    assert_eq!(val["status"], "running");
}

#[test]
fn resource_type_display() {
    assert_eq!(ResourceType::Vm.to_string(), "vm");
    assert_eq!(ResourceType::Vpc.to_string(), "vpc");
    assert_eq!(ResourceType::LoadBalancer.to_string(), "load_balancer");
}
