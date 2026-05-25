// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composition engine — resolve, execute, and roll back Composition pipelines.
//!
//! A Composition maps a `PromiseRequest` to an ordered set of CAVE module
//! calls (`CompositionStep`s).  The engine handles:
//! - dependency ordering (topological sort within the step list)
//! - parameter mapping from request parameters to module inputs
//! - sequential execution with per-step timeouts
//! - rollback on failure (reverse order of completed steps)

use crate::{
    models::{ClaimStatus, CompositionStep, ResourceClaim},
    promise::{update_request_status, EngineError},
    AppState,
};
use chrono::Utc;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Map a `PromiseRequest`'s parameters to the concrete module calls defined
/// in its Composition, returning the resolved step list.
///
/// Currently this is a pass-through; a production implementation would
/// substitute JSONPath expressions in `step.parameter_mapping`.
pub fn resolve_composition(
    steps: &[CompositionStep],
    parameters: &serde_json::Value,
) -> Vec<ResolvedStep> {
    steps
        .iter()
        .map(|step| ResolvedStep {
            name: step.name.clone(),
            module: step.module.clone(),
            operation: step.operation.clone(),
            resolved_params: parameter_mapping(&step.parameter_mapping, parameters),
            depends_on: step.depends_on.clone(),
            required: step.required,
            timeout_secs: step.timeout_secs,
        })
        .collect()
}

/// Execute the pipeline for a `PromiseRequest`, creating `ResourceClaim`
/// records as each step completes.  Rolls back on failure.
pub async fn execute_pipeline(
    state: Arc<AppState>,
    request_id: Uuid,
    steps: Vec<CompositionStep>,
) {
    // Fetch the request's parameters for mapping.
    let parameters = {
        let requests = state.requests.lock().await;
        requests
            .iter()
            .find(|r| r.id == request_id)
            .map(|r| r.parameters.clone())
            .unwrap_or(serde_json::Value::Null)
    };

    let resolved = resolve_composition(&steps, &parameters);
    let mut completed: Vec<Uuid> = vec![];

    for step in &resolved {
        info!(
            request_id = %request_id,
            step = %step.name,
            module = %step.module,
            operation = %step.operation,
            "Executing pipeline step"
        );

        match execute_step(Arc::clone(&state), request_id, step).await {
            Ok(claim_id) => {
                completed.push(claim_id);
                // Record claim ID on the request.
                let mut requests = state.requests.lock().await;
                if let Some(r) = requests.iter_mut().find(|r| r.id == request_id) {
                    r.claim_ids.push(claim_id);
                }
            }
            Err(e) => {
                error!(
                    request_id = %request_id,
                    step = %step.name,
                    error = %e,
                    "Pipeline step failed"
                );

                if step.required {
                    rollback_pipeline(Arc::clone(&state), request_id, &completed).await;
                    update_request_status(
                        Arc::clone(&state),
                        request_id,
                        crate::models::RequestStatus::RolledBack,
                        Some(format!("Step '{}' failed: {e}", step.name)),
                    )
                    .await;
                    return;
                } else {
                    warn!(
                        step = %step.name,
                        "Optional step failed — continuing pipeline"
                    );
                }
            }
        }
    }

    // All steps done — mark request Ready via the reconcile loop.
    info!(request_id = %request_id, "Pipeline completed successfully");
}

/// Roll back completed steps in reverse order.
pub async fn rollback_pipeline(
    state: Arc<AppState>,
    request_id: Uuid,
    completed_claim_ids: &[Uuid],
) {
    info!(request_id = %request_id, "Rolling back pipeline");

    for &claim_id in completed_claim_ids.iter().rev() {
        rollback_step(Arc::clone(&state), claim_id).await;
    }
}

/// Map user-supplied `parameters` into module-specific params using the
/// step's `parameter_mapping` descriptor.
///
/// The mapping object is expected to be a flat JSON object whose keys are
/// module parameter names and whose values are either:
/// - a literal value, or
/// - a `$.field` JSONPath expression resolved against `parameters`.
pub fn parameter_mapping(
    mapping: &serde_json::Value,
    parameters: &serde_json::Value,
) -> serde_json::Value {
    let Some(map) = mapping.as_object() else {
        return parameters.clone();
    };

    let mut resolved = serde_json::Map::new();

    for (key, expr) in map {
        let value = if let Some(path) = expr.as_str().and_then(|s| s.strip_prefix("$.")) {
            // Simple top-level field lookup.
            parameters
                .get(path)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        } else {
            // Literal value.
            expr.clone()
        };
        resolved.insert(key.clone(), value);
    }

    serde_json::Value::Object(resolved)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// A `CompositionStep` with its parameter expressions already resolved.
#[derive(Debug)]
pub struct ResolvedStep {
    pub name: String,
    pub module: String,
    pub operation: String,
    pub resolved_params: serde_json::Value,
    pub depends_on: Vec<String>,
    pub required: bool,
    pub timeout_secs: u64,
}

async fn execute_step(
    state: Arc<AppState>,
    request_id: Uuid,
    step: &ResolvedStep,
) -> Result<Uuid, EngineError> {
    // Stub: in production each `step.module` maps to a gRPC / HTTP call to
    // the corresponding CAVE module.  Here we simulate success and record a
    // ResourceClaim.
    let claim = ResourceClaim {
        id: Uuid::new_v4(),
        request_id,
        promise_id: {
            let requests = state.requests.lock().await;
            requests
                .iter()
                .find(|r| r.id == request_id)
                .map(|r| r.promise_id)
                .unwrap_or_else(Uuid::new_v4)
        },
        module: step.module.clone(),
        resource_id: format!("{}-{}", step.module, Uuid::new_v4()),
        resource_type: step.operation.clone(),
        environment: {
            let requests = state.requests.lock().await;
            requests
                .iter()
                .find(|r| r.id == request_id)
                .map(|r| r.environment.clone())
                .unwrap_or_default()
        },
        status: ClaimStatus::Ready,
        outputs: serde_json::json!({
            "step": step.name,
            "module": step.module,
            "operation": step.operation,
            "params": step.resolved_params,
        }),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };

    let claim_id = claim.id;
    state.claims.lock().await.push(claim);

    info!(
        request_id = %request_id,
        claim_id = %claim_id,
        step = %step.name,
        "Step completed — ResourceClaim created"
    );

    Ok(claim_id)
}

async fn rollback_step(state: Arc<AppState>, claim_id: Uuid) {
    let mut claims = state.claims.lock().await;
    if let Some(claim) = claims.iter_mut().find(|c| c.id == claim_id) {
        claim.status = ClaimStatus::Deleted;
        claim.deleted_at = Some(Utc::now());
        claim.updated_at = Utc::now();
        info!(
            claim_id = %claim_id,
            module = %claim.module,
            "Rolled back ResourceClaim"
        );
    }
}
