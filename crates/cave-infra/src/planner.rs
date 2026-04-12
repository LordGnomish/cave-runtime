//! LLM-powered planner — generate, optimize, cost-estimate, risk-score plans.
//!
//! In production `generate_plan` sends the intent + state diff to a local LLM
//! (ollama / llama.cpp) and receives a structured ExecutionPlan. Here we implement
//! a deterministic rule-based planner so the crate compiles without a GPU.

use crate::intent::{diff_state, ChangesetEntry};
use crate::models::{
    CostEstimate, CostItem, ExecutionPlan, InfraIntent, InfraState, McpProvider, PlanStatus,
    PlanStep, StepAction,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

/// Convert an intent into an ExecutionPlan using state diff + heuristic rules.
pub fn generate_plan(
    intent: &InfraIntent,
    current_state: &InfraState,
    providers: &[McpProvider],
) -> ExecutionPlan {
    let changeset = diff_state(intent, current_state);
    let steps = changeset_to_steps(&changeset, providers);
    let rollback_steps = generate_rollback(&steps);
    let cost = estimate_cost_from_steps(&steps);
    let risk = assess_risk_score(&steps, current_state);
    let explanation = explain(&steps, intent);

    ExecutionPlan {
        id: Uuid::new_v4(),
        intent_id: intent.id,
        steps,
        rollback_steps,
        cost_estimate: cost,
        risk_score: risk,
        explanation,
        created_at: Utc::now(),
        status: PlanStatus::Draft,
    }
}

/// Parallelize independent steps by clearing unnecessary sequential ordering.
pub fn optimize_plan(plan: &mut ExecutionPlan) {
    let all_ids: Vec<Uuid> = plan.steps.iter().map(|s| s.id).collect();
    for step in &mut plan.steps {
        // Remove any dependency that doesn't exist in the plan (defensive clean-up).
        step.depends_on
            .retain(|dep| all_ids.contains(dep) && *dep != step.id);
    }
}

/// Estimate the cost impact of an execution plan.
pub fn estimate_cost(plan: &ExecutionPlan) -> CostEstimate {
    estimate_cost_from_steps(&plan.steps)
}

/// Score risk 0–100 based on destructiveness, irreversibility, and blast radius.
pub fn assess_risk(plan: &ExecutionPlan, state: &InfraState) -> u8 {
    assess_risk_score(&plan.steps, state)
}

/// Generate a human-readable explanation of what the plan will do.
pub fn explain_plan(plan: &ExecutionPlan, intent: &InfraIntent) -> String {
    explain(&plan.steps, intent)
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn changeset_to_steps(changeset: &[ChangesetEntry], providers: &[McpProvider]) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let mut prev: Option<Uuid> = None;

    for entry in changeset {
        if entry.action == StepAction::NoOp {
            continue;
        }

        let provider_ok = providers.iter().any(|p| p.name == entry.provider && p.healthy);
        let mcp_tool = format!("{}_{}", action_verb(&entry.action), entry.resource_type);

        let mut params = HashMap::new();
        params.insert(
            "resource_type".to_string(),
            serde_json::Value::String(entry.resource_type.clone()),
        );
        params.insert(
            "name".to_string(),
            serde_json::Value::String(entry.resource_name.clone()),
        );
        params.insert(
            "provider_available".to_string(),
            serde_json::Value::Bool(provider_ok),
        );
        params.insert(
            "mcp_tool".to_string(),
            serde_json::Value::String(mcp_tool),
        );

        let id = Uuid::new_v4();
        let reversible = entry.action != StepAction::Delete;

        steps.push(PlanStep {
            id,
            action: entry.action.clone(),
            provider: entry.provider.clone(),
            resource_name: entry.resource_name.clone(),
            resource_type: entry.resource_type.clone(),
            params,
            depends_on: prev.map(|p| vec![p]).unwrap_or_default(),
            estimated_duration_secs: duration_secs(&entry.resource_type),
            reversible,
        });
        prev = Some(id);
    }
    steps
}

fn generate_rollback(steps: &[PlanStep]) -> Vec<PlanStep> {
    steps
        .iter()
        .rev()
        .filter(|s| s.reversible)
        .map(|s| {
            let action = match s.action {
                StepAction::Create => StepAction::Delete,
                StepAction::Delete => StepAction::Create,
                StepAction::Update | StepAction::NoOp => s.action.clone(),
            };
            PlanStep {
                id: Uuid::new_v4(),
                action,
                provider: s.provider.clone(),
                resource_name: s.resource_name.clone(),
                resource_type: s.resource_type.clone(),
                params: s.params.clone(),
                depends_on: vec![],
                estimated_duration_secs: s.estimated_duration_secs,
                reversible: true,
            }
        })
        .collect()
}

fn estimate_cost_from_steps(steps: &[PlanStep]) -> CostEstimate {
    let breakdown: Vec<CostItem> = steps
        .iter()
        .filter(|s| s.action != StepAction::Delete && s.action != StepAction::NoOp)
        .map(|s| {
            let monthly = monthly_cost(&s.resource_type);
            CostItem {
                resource_name: s.resource_name.clone(),
                resource_type: s.resource_type.clone(),
                monthly_usd: monthly,
            }
        })
        .collect();

    let total: f64 = breakdown.iter().map(|c| c.monthly_usd).sum();
    CostEstimate {
        monthly_usd: total,
        hourly_usd: total / (30.0 * 24.0),
        breakdown,
        currency: "USD".to_string(),
    }
}

fn assess_risk_score(steps: &[PlanStep], state: &InfraState) -> u8 {
    let mut score: u32 = 0;

    for step in steps {
        score += match step.action {
            StepAction::Delete => 40,
            StepAction::Update => 15,
            StepAction::Create => 5,
            StepAction::NoOp => 0,
        };
        if !step.reversible {
            score += 20;
        }
    }

    // Higher blast radius when destroying resources in a large state.
    let has_deletes = steps.iter().any(|s| s.action == StepAction::Delete);
    if has_deletes && state.resources.len() > 5 {
        score += 10;
    }

    score.min(100) as u8
}

fn explain(steps: &[PlanStep], intent: &InfraIntent) -> String {
    if steps.is_empty() {
        return format!(
            "No changes required — current state already satisfies intent: \"{}\"",
            intent.description
        );
    }
    let mut lines = vec![format!(
        "Plan for: \"{}\"\n{} step(s) to execute:\n",
        intent.description,
        steps.len()
    )];
    for (i, s) in steps.iter().enumerate() {
        lines.push(format!(
            "  {}. {:?} {} ({}) via {} [~{}s]",
            i + 1,
            s.action,
            s.resource_name,
            s.resource_type,
            s.provider,
            s.estimated_duration_secs
        ));
    }
    lines.join("\n")
}

fn action_verb(action: &StepAction) -> &'static str {
    match action {
        StepAction::Create => "create",
        StepAction::Update => "update",
        StepAction::Delete => "delete",
        StepAction::NoOp => "noop",
    }
}

fn duration_secs(resource_type: &str) -> u32 {
    match resource_type {
        "rds_cluster" | "kubernetes_cluster" => 600,
        "virtual_machine" => 120,
        "object_storage" => 15,
        _ => 60,
    }
}

fn monthly_cost(resource_type: &str) -> f64 {
    match resource_type {
        "rds_cluster" => 180.0,
        "kubernetes_cluster" => 350.0,
        "virtual_machine" => 25.0,
        "object_storage" => 5.0,
        _ => 20.0,
//! LLM infrastructure planner — turns a parsed intent into an actionable plan.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::providers::ResourceType;
// ── PlannedResource ───────────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedResource {
    /// Logical ID within this plan.
    pub id: String,
    pub resource_type: ResourceType,
    pub provider: String,
    pub name: String,
    pub spec: serde_json::Value,
    /// Logical IDs of resources this one depends on.
    pub depends_on: Vec<String>,
    pub estimated_cost_usd: Option<f64>,
}
// ── PlanStatus ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    Draft,
    PendingApproval,
    Approved,
    Rejected,
    Applying,
    Applied,
    Failed(String),
}
// ── InfraPlan ─────────────────────────────────────────────────────────────────
/// A complete infrastructure change-set derived from an intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraPlan {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub tenant_id: String,
    pub resources_to_create: Vec<PlannedResource>,
    pub resources_to_update: Vec<PlannedResource>,
    /// IDs of resources that should be destroyed.
    pub resources_to_destroy: Vec<String>,
    pub estimated_cost_usd: Option<f64>,
    pub status: PlanStatus,
    pub created_at: DateTime<Utc>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
}
impl InfraPlan {
    pub fn new(intent_id: Uuid, tenant_id: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            tenant_id: tenant_id.to_string(),
            resources_to_create: Vec::new(),
            resources_to_update: Vec::new(),
            resources_to_destroy: Vec::new(),
            estimated_cost_usd: None,
            status: PlanStatus::Draft,
            created_at: Utc::now(),
            approved_by: None,
            approved_at: None,
        }
    }
    /// Sum the estimated costs of all resources in this plan.
    pub fn total_estimated_cost(&self) -> f64 {
        let create: f64 = self
            .resources_to_create
            .iter()
            .filter_map(|r| r.estimated_cost_usd)
            .sum();
        let update: f64 = self
            .resources_to_update
            .iter()
            .filter_map(|r| r.estimated_cost_usd)
            .sum();
        create + update
    }
    /// Total number of resource operations planned.
    pub fn resource_count(&self) -> usize {
        self.resources_to_create.len()
            + self.resources_to_update.len()
            + self.resources_to_destroy.len()
    }
}
// ── PlannerError ──────────────────────────────────────────────────────────────
#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    #[error("LLM unavailable: {0}")]
    LlmUnavailable(String),
    #[error("Cannot plan: {0}")]
    CannotPlan(String),
    #[error("Rate limited")]
    RateLimited,
}
// ── InfraPlanner trait ────────────────────────────────────────────────────────
/// Planners turn an intent into a concrete `InfraPlan`.
#[async_trait::async_trait]
pub trait InfraPlanner: Send + Sync {
    async fn generate_plan(
        &self,
        intent: &crate::intent::InfraIntent,
    ) -> Result<InfraPlan, PlannerError>;
}
// ── LlmPlanner ────────────────────────────────────────────────────────────────
/// HTTP-based LLM planner (calls Claude API or compatible endpoint).
pub struct LlmPlanner {
    pub api_endpoint: String,
    pub api_key: String,
    client: reqwest::Client,
}
impl LlmPlanner {
    pub fn new(api_endpoint: &str, api_key: &str) -> Self {
        Self {
            api_endpoint: api_endpoint.to_string(),
            api_key: api_key.to_string(),
            client: reqwest::Client::new(),
        }
    }
}
#[async_trait::async_trait]
impl InfraPlanner for LlmPlanner {
    async fn generate_plan(
        &self,
        intent: &crate::intent::InfraIntent,
    ) -> Result<InfraPlan, PlannerError> {
        let body = serde_json::json!({
            "model": "claude-3-5-sonnet-latest",
            "messages": [{
                "role": "user",
                "content": format!(
                    "Generate an infrastructure plan for: {}",
                    intent.description
                )
            }]
        });
        let resp = self
            .client
            .post(&self.api_endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PlannerError::LlmUnavailable(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(PlannerError::RateLimited);
        }
        if !resp.status().is_success() {
            return Err(PlannerError::LlmUnavailable(format!(
                "HTTP {}",
                resp.status()
            )));
        }
        // A real implementation would parse the LLM response; here we return
        // an empty draft plan to keep the HTTP path compiling.
        Ok(InfraPlan::new(intent.id, &intent.tenant_id))
    }
}
// ── MockPlanner ───────────────────────────────────────────────────────────────
/// Deterministic mock planner for tests.
pub struct MockPlanner {
    pub resources: Vec<PlannedResource>,
}
impl MockPlanner {
    pub fn new(resources: Vec<PlannedResource>) -> Self {
        Self { resources }
    }
}
#[async_trait::async_trait]
impl InfraPlanner for MockPlanner {
    async fn generate_plan(
        &self,
        intent: &crate::intent::InfraIntent,
    ) -> Result<InfraPlan, PlannerError> {
        let mut plan = InfraPlan::new(intent.id, &intent.tenant_id);
        plan.resources_to_create = self.resources.clone();
        // Populate the aggregate cost field.
        let total = plan.total_estimated_cost();
        if total > 0.0 {
            plan.estimated_cost_usd = Some(total);
        }
        Ok(plan)
    }
}
// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::InfraIntent;
    fn make_planned_resource(id: &str, cost: f64) -> PlannedResource {
        PlannedResource {
            id: id.to_string(),
            resource_type: ResourceType::Vm,
            provider: "mock".to_string(),
            name: format!("resource-{id}"),
            spec: serde_json::json!({}),
            depends_on: vec![],
            estimated_cost_usd: Some(cost),
        }
    }
    #[tokio::test]
    async fn test_mock_planner_generates_plan() {
        let resources = vec![
            make_planned_resource("r1", 10.0),
            make_planned_resource("r2", 20.0),
        ];
        let planner = MockPlanner::new(resources);
        let user = uuid::Uuid::new_v4();
        let intent = InfraIntent::new("deploy two vms", "t1", user);
        let plan = planner.generate_plan(&intent).await.unwrap();
        assert_eq!(plan.intent_id, intent.id);
        assert_eq!(plan.tenant_id, "t1");
        assert_eq!(plan.resources_to_create.len(), 2);
    }
    #[tokio::test]
    async fn test_plan_resource_count_and_cost() {
        let resources = vec![
            make_planned_resource("a", 5.0),
            make_planned_resource("b", 15.0),
        ];
        let planner = MockPlanner::new(resources);
        let user = uuid::Uuid::new_v4();
        let intent = InfraIntent::new("two small vms", "t2", user);
        let plan = planner.generate_plan(&intent).await.unwrap();
        assert_eq!(plan.resource_count(), 2);
        assert!((plan.total_estimated_cost() - 20.0).abs() < f64::EPSILON);
    }
//! LLM-powered execution plan generation and optimization.
use crate::intent::diff_state;
use crate::models::{
    CostEstimate, ExecutionPlan, InfraIntent, InfraState, PlanStep, PolicyCheck, PolicySeverity,
    StepOperation,
};
use anyhow::Result;
use std::collections::HashMap;
use uuid::Uuid;
/// Generate an `ExecutionPlan` from an intent + current state.
///
/// In production this would call an LLM via MCP or HTTP.  Here we implement
/// the deterministic skeleton: diff → steps → rollback → cost → risk.
pub fn generate_plan(intent: &InfraIntent, state: &InfraState) -> Result<ExecutionPlan> {
    let mut plan = ExecutionPlan::new(intent.id);
    let (to_create, to_update, to_delete) = diff_state(state);
    // Build forward steps.
    for r in &to_create {
        let tool = format!("{}_create_{}", r.provider, r.resource_type);
        let mut step = PlanStep::new(
            StepOperation::Create,
            &r.name,
            &r.provider,
            &r.resource_type,
            &tool,
            format!("Create {} '{}' via {}", r.resource_type, r.name, r.provider),
        );
        step.provider_params = r
            .config
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Map resource dep IDs → step IDs (best-effort; same name).
        plan.steps.push(step);
    }
    for r in &to_update {
        let tool = format!("{}_update_{}", r.provider, r.resource_type);
        let mut step = PlanStep::new(
            StepOperation::Update,
            &r.name,
            &r.provider,
            &r.resource_type,
            &tool,
            format!("Update {} '{}' via {}", r.resource_type, r.name, r.provider),
        );
        step.provider_params = r
            .config
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        plan.steps.push(step);
    }
    for r in &to_delete {
        let tool = format!("{}_delete_{}", r.provider, r.resource_type);
        let step = PlanStep::new(
            StepOperation::Delete,
            &r.name,
            &r.provider,
            &r.resource_type,
            &tool,
            format!("Delete {} '{}' from {}", r.resource_type, r.name, r.provider),
        );
        plan.steps.push(step);
    }
    // Wire step-level depends_on by matching resource dependency names to step IDs.
    let step_ids_by_resource: HashMap<String, Uuid> = plan
        .steps
        .iter()
        .map(|s| (s.resource_name.clone(), s.id))
        .collect();
    // For each desired resource, propagate its deps to the corresponding step.
    for desired in &state.desired {
        if let Some(&step_id) = step_ids_by_resource.get(&desired.name) {
            let dep_step_ids: Vec<Uuid> = desired
                .dependencies
                .iter()
                .filter_map(|dep_rid| {
                    state
                        .desired
                        .iter()
                        .find(|r| r.id == *dep_rid)
                        .and_then(|r| step_ids_by_resource.get(&r.name))
                        .copied()
                })
                .collect();
            if let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id) {
                step.depends_on = dep_step_ids;
            }
        }
    }
    // Rollback = reverse delete-what-was-created, create-what-was-deleted.
    plan.rollback_steps = plan
        .steps
        .iter()
        .rev()
        .map(|s| {
            let inverse_op = match s.operation {
                StepOperation::Create => StepOperation::Delete,
                StepOperation::Delete => StepOperation::Create,
                StepOperation::Update | StepOperation::NoOp => StepOperation::NoOp,
            };
            let tool = format!("{}_rollback_{}", s.provider, s.resource_type);
            let mut rb = PlanStep::new(
                inverse_op,
                &s.resource_name,
                &s.provider,
                &s.resource_type,
                &tool,
                format!("Rollback: {}", s.description),
            );
            rb.provider_params = s.provider_params.clone();
            rb
        })
        .collect();
    plan.cost_estimate = Some(estimate_cost(&plan));
    plan.risk_score = assess_risk(&plan, state);
    plan.explanation = explain_plan(&plan, intent);
    Ok(optimize_plan(plan))
}
/// Reorder / mark parallelizable steps that have no shared dependencies.
pub fn optimize_plan(mut plan: ExecutionPlan) -> ExecutionPlan {
    // Group steps by depth level (topological layers).
    let mut depth: HashMap<Uuid, usize> = HashMap::new();
    for step in &plan.steps {
        compute_depth(step.id, &plan.steps, &mut depth);
    }
    // Steps at the same depth with no shared deps can run in parallel.
    let mut depth_counts: HashMap<usize, usize> = HashMap::new();
    for &d in depth.values() {
        *depth_counts.entry(d).or_default() += 1;
    }
    for step in &mut plan.steps {
        let d = depth.get(&step.id).copied().unwrap_or(0);
        step.parallelizable = depth_counts.get(&d).copied().unwrap_or(1) > 1;
    }
    plan
}
fn compute_depth(id: Uuid, steps: &[PlanStep], depth: &mut HashMap<Uuid, usize>) -> usize {
    if let Some(&d) = depth.get(&id) {
        return d;
    }
    let step = match steps.iter().find(|s| s.id == id) {
        Some(s) => s,
        None => return 0,
    };
    let d = if step.depends_on.is_empty() {
        0
    } else {
        step.depends_on
            .iter()
            .map(|&dep| compute_depth(dep, steps, depth) + 1)
            .max()
            .unwrap_or(0)
    };
    depth.insert(id, d);
    d
}
/// Heuristic cost estimate — $10/month per resource, adjusted by type.
pub fn estimate_cost(plan: &ExecutionPlan) -> CostEstimate {
    let mut breakdown = HashMap::new();
    let mut total = 0.0;
    for step in &plan.steps {
        if step.operation == StepOperation::Delete {
            continue;
        }
        let base = match step.resource_type.as_str() {
            t if t.contains("cluster") || t.contains("node") => 200.0,
            t if t.contains("database") || t.contains("rds") || t.contains("sql") => 80.0,
            t if t.contains("load_balancer") || t.contains("alb") || t.contains("nlb") => 20.0,
            t if t.contains("bucket") || t.contains("storage") => 5.0,
            _ => 10.0,
        };
        breakdown.insert(step.resource_name.clone(), base);
        total += base;
    }
    CostEstimate {
        monthly_usd: total,
        breakdown,
        confidence: 0.6,
        currency: "USD".into(),
        notes: vec![
            "Estimate based on resource type heuristics".into(),
            "Actual cost depends on usage, region, and provider pricing".into(),
        ],
    }
}
/// Compute blast-radius risk score (0.0 = safe, 1.0 = very risky).
pub fn assess_risk(plan: &ExecutionPlan, state: &InfraState) -> f64 {
    let total = plan.steps.len() as f64;
    if total == 0.0 {
        return 0.0;
    }
    let deletes = plan
        .steps
        .iter()
        .filter(|s| s.operation == StepOperation::Delete)
        .count() as f64;
    let prod_penalty = if state.desired.iter().any(|_| true) {
        // Simple heuristic: if any resource name contains "prod" bump risk.
        let prod_resources = state
            .desired
            .iter()
            .filter(|r| r.name.contains("prod") || r.name.contains("production"))
            .count() as f64;
        (prod_resources / state.desired.len().max(1) as f64) * 0.3
    } else {
        0.0
    };
    let delete_ratio = (deletes / total) * 0.5;
    let size_penalty = (total / 50.0).min(0.2);
    (delete_ratio + size_penalty + prod_penalty).min(1.0)
}
/// Evaluate OPA-style policies against a plan.
pub fn evaluate_policies(plan: &ExecutionPlan) -> Vec<PolicyCheck> {
    let mut checks = Vec::new();
    // Policy: no more than 10 deletes in a single plan.
    let delete_count = plan
        .steps
        .iter()
        .filter(|s| s.operation == StepOperation::Delete)
        .count();
    checks.push(PolicyCheck {
        policy_name: "max-deletes-per-plan".into(),
        passed: delete_count <= 10,
        violations: if delete_count > 10 {
            vec![format!(
                "Plan contains {} delete operations, max is 10",
                delete_count
            )]
        } else {
            vec![]
        },
        severity: PolicySeverity::Error,
    });
    // Policy: high-risk plans require explicit approval.
    let risk = plan.risk_score;
    checks.push(PolicyCheck {
        policy_name: "high-risk-approval-required".into(),
        passed: risk < 0.7,
        violations: if risk >= 0.7 {
            vec![format!(
                "Risk score {:.2} exceeds threshold 0.70 — manual approval required",
                risk
            )]
        } else {
            vec![]
        },
        severity: PolicySeverity::Critical,
    });
    // Policy: rollback steps must exist if there are creates.
    let has_creates = plan
        .steps
        .iter()
        .any(|s| s.operation == StepOperation::Create);
    checks.push(PolicyCheck {
        policy_name: "rollback-plan-required".into(),
        passed: !has_creates || !plan.rollback_steps.is_empty(),
        violations: if has_creates && plan.rollback_steps.is_empty() {
            vec!["Plan has create operations but no rollback steps".into()]
        } else {
            vec![]
        },
        severity: PolicySeverity::Warning,
    });
    checks
}
/// Generate a human-readable explanation of the plan.
pub fn explain_plan(plan: &ExecutionPlan, intent: &InfraIntent) -> String {
    let creates = plan
        .steps
        .iter()
        .filter(|s| s.operation == StepOperation::Create)
        .count();
    let updates = plan
        .steps
        .iter()
        .filter(|s| s.operation == StepOperation::Update)
        .count();
    let deletes = plan
        .steps
        .iter()
        .filter(|s| s.operation == StepOperation::Delete)
        .count();
    let cost_str = plan
        .cost_estimate
        .as_ref()
        .map(|c| format!("${:.2}/month", c.monthly_usd))
        .unwrap_or_else(|| "unknown".into());
    let intent_desc = intent
        .natural_language
        .as_deref()
        .unwrap_or(intent.name.as_str());
    format!(
        "Plan for intent '{}' (env: {}):\n\
         • {} resource(s) to create\n\
         • {} resource(s) to update\n\
         • {} resource(s) to delete\n\
         • Estimated cost: {}\n\
         • Risk score: {:.2}/1.00\n\
         • {} rollback step(s) available",
        intent_desc,
        intent.environment,
        creates,
        updates,
        deletes,
        cost_str,
        plan.risk_score,
        plan.rollback_steps.len(),
    )
}
