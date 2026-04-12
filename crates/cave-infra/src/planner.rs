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
