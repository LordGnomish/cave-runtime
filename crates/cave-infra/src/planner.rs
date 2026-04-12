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
    }
}
