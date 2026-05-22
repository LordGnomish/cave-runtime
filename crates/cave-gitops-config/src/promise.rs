// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Promise engine — register, validate, and fulfil platform capability requests.
//!
//! The engine mirrors a Kubernetes controller loop: `reconcile()` continuously
//! drives actual state toward desired state for every active `PromiseRequest`.

use crate::{
    models::{
        ClaimStatus, ComplianceRule, ComplianceSeverity, CreateCapabilityRequest,
        CreatePromiseRequest, PromiseRequest, PromiseStatus, RequestStatus,
    },
    AppState,
};
use chrono::Utc;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Promise registry
// ---------------------------------------------------------------------------

/// Register a new Promise in the platform catalog.
pub async fn register_promise(
    state: Arc<AppState>,
    req: CreatePromiseRequest,
) -> Result<crate::models::Promise, EngineError> {
    let promise = crate::models::Promise {
        id: Uuid::new_v4(),
        name: req.name.clone(),
        description: req.description,
        version: req.version,
        api_group: req.api_group,
        input_schema: req.input_schema,
        pipeline: req.pipeline,
        status: PromiseStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let mut store = state.promises.lock().await;
    if store.iter().any(|p| p.name == promise.name) {
        return Err(EngineError::AlreadyExists(format!(
            "Promise '{}' already registered",
            promise.name
        )));
    }
    store.push(promise.clone());
    info!(promise = %promise.name, version = %promise.version, "Promise registered");
    Ok(promise)
}

/// Return all registered Promises.
pub async fn list_promises(state: Arc<AppState>) -> Vec<crate::models::Promise> {
    state.promises.lock().await.clone()
}

/// Look up a Promise by name.
pub async fn get_promise(
    state: Arc<AppState>,
    name: &str,
) -> Result<crate::models::Promise, EngineError> {
    state
        .promises
        .lock()
        .await
        .iter()
        .find(|p| p.name == name)
        .cloned()
        .ok_or_else(|| EngineError::NotFound(format!("Promise '{name}' not found")))
}

// ---------------------------------------------------------------------------
// Request fulfilment
// ---------------------------------------------------------------------------

/// Accept a developer capability request, validate it, then hand off to the
/// composition engine for provisioning.
pub async fn fulfill_request(
    state: Arc<AppState>,
    req: CreateCapabilityRequest,
) -> Result<PromiseRequest, EngineError> {
    // 1. Locate the Promise.
    let promise = get_promise(Arc::clone(&state), &req.promise_name).await?;

    if promise.status != PromiseStatus::Active {
        return Err(EngineError::Unavailable(format!(
            "Promise '{}' is not active",
            promise.name
        )));
    }

    // 2. Build the request record.
    let mut pr = PromiseRequest {
        id: Uuid::new_v4(),
        promise_id: promise.id,
        promise_name: promise.name.clone(),
        environment: req.environment.clone(),
        parameters: req.parameters.clone(),
        requested_by: req.requested_by.clone(),
        status: RequestStatus::Validating,
        message: None,
        claim_ids: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    {
        let mut requests = state.requests.lock().await;
        requests.push(pr.clone());
    }

    // 3. Validate.
    match validate_request(Arc::clone(&state), &promise, &pr).await {
        Ok(()) => {}
        Err(e) => {
            update_request_status(
                Arc::clone(&state),
                pr.id,
                RequestStatus::Failed,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    }

    // 4. Kick off asynchronous provisioning.
    update_request_status(
        Arc::clone(&state),
        pr.id,
        RequestStatus::Provisioning,
        Some("Pipeline started".into()),
    )
    .await;

    let state_clone = Arc::clone(&state);
    let request_id = pr.id;
    let steps = promise.pipeline.clone();
    tokio::spawn(async move {
        crate::composition::execute_pipeline(state_clone, request_id, steps).await;
    });

    pr.status = RequestStatus::Provisioning;
    Ok(pr)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a `PromiseRequest` against:
/// - the Promise's JSON Schema (`input_schema`)
/// - all active `ComplianceCheck`s
pub async fn validate_request(
    state: Arc<AppState>,
    promise: &crate::models::Promise,
    req: &PromiseRequest,
) -> Result<(), EngineError> {
    // JSON Schema validation (structural check against declared schema).
    validate_against_schema(&promise.input_schema, &req.parameters)?;

    // Compliance checks.
    let checks = state.compliance_checks.lock().await.clone();
    for check in &checks {
        // Filter by promise applicability.
        if !check.applies_to_promises.is_empty()
            && !check
                .applies_to_promises
                .contains(&promise.name)
        {
            continue;
        }
        // Filter by environment applicability.
        if !check.applies_to_environments.is_empty()
            && !check
                .applies_to_environments
                .contains(&req.environment)
        {
            continue;
        }

        match evaluate_compliance_rule(&check.rule, req) {
            Ok(passed) => {
                if !passed {
                    let msg = format!(
                        "Compliance check '{}' failed: {}",
                        check.name, check.description
                    );
                    if check.severity == ComplianceSeverity::Error {
                        return Err(EngineError::ComplianceViolation(msg));
                    } else {
                        warn!("{msg}");
                    }
                }
            }
            Err(e) => {
                warn!(check = %check.name, error = %e, "Compliance check evaluation error");
            }
        }
    }

    Ok(())
}

fn validate_against_schema(
    schema: &serde_json::Value,
    params: &serde_json::Value,
) -> Result<(), EngineError> {
    // Lightweight structural validation: check that every property listed in
    // the schema's `required` array is present in `params`.
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        for field in required {
            let key = field.as_str().unwrap_or_default();
            if params.get(key).is_none() {
                return Err(EngineError::ValidationFailed(format!(
                    "Required parameter '{key}' is missing"
                )));
            }
        }
    }
    Ok(())
}

fn evaluate_compliance_rule(
    rule: &ComplianceRule,
    req: &PromiseRequest,
) -> Result<bool, EngineError> {
    match rule {
        ComplianceRule::MaxValue { field, max_value } => {
            let val = req.parameters.get(field).and_then(|v| v.as_u64());
            Ok(val.map_or(true, |v| v <= *max_value))
        }
        ComplianceRule::AllowedValues {
            field,
            allowed_values,
        } => {
            let val = req.parameters.get(field).and_then(|v| v.as_str());
            Ok(val.map_or(true, |v| allowed_values.iter().any(|a| a == v)))
        }
        ComplianceRule::EnvironmentTier { allowed_tiers } => {
            // Resolve environment → tier via PlatformConfig.
            // For now accept any environment; real implementation would look up
            // the environment in state.platform_config.
            let _ = allowed_tiers;
            Ok(true)
        }
        ComplianceRule::CostLimit {
            max_cents_per_hour,
        } => {
            let cost = req
                .parameters
                .get("estimated_cost_cents_per_hour")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Ok(cost <= *max_cents_per_hour)
        }
        ComplianceRule::JsonPath { expression } => {
            // Placeholder — a full implementation would use a JSONPath crate.
            info!(expr = %expression, "JSONPath compliance rule (stub — always passes)");
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Reconciliation (controller loop)
// ---------------------------------------------------------------------------

/// Drive all non-terminal `PromiseRequest`s toward their desired state.
///
/// Call this periodically from a background task to implement the Kubernetes
/// controller loop pattern.
pub async fn reconcile(state: Arc<AppState>) {
    let requests = state.requests.lock().await.clone();
    for req in requests {
        match req.status {
            RequestStatus::Failed | RequestStatus::Ready | RequestStatus::Deleted => continue,
            RequestStatus::Provisioning => {
                // Check if all claims are ready.
                let claims = state.claims.lock().await;
                let req_claims: Vec<_> = claims
                    .iter()
                    .filter(|c| c.request_id == req.id)
                    .collect();

                if req_claims.is_empty() {
                    continue;
                }

                let all_ready = req_claims
                    .iter()
                    .all(|c| c.status == ClaimStatus::Ready);
                let any_failed = req_claims
                    .iter()
                    .any(|c| c.status == ClaimStatus::Failed);

                drop(claims);

                if any_failed {
                    update_request_status(
                        Arc::clone(&state),
                        req.id,
                        RequestStatus::Failed,
                        Some("One or more pipeline steps failed".into()),
                    )
                    .await;
                } else if all_ready {
                    update_request_status(
                        Arc::clone(&state),
                        req.id,
                        RequestStatus::Ready,
                        Some("All resources provisioned".into()),
                    )
                    .await;
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

pub(crate) async fn update_request_status(
    state: Arc<AppState>,
    id: Uuid,
    status: RequestStatus,
    message: Option<String>,
) {
    let mut requests = state.requests.lock().await;
    if let Some(r) = requests.iter_mut().find(|r| r.id == id) {
        r.status = status;
        r.message = message;
        r.updated_at = Utc::now();
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Compliance violation: {0}")]
    ComplianceViolation(String),

    #[error("Promise unavailable: {0}")]
    Unavailable(String),

    #[error("Pipeline error: {0}")]
    Pipeline(String),
}
