// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Validating admission webhook — Chaos Mesh `api/v1alpha1/*_webhook.go` port.
//!
//! Rejects an experiment (or schedule) at *admission* time — before it is ever
//! stored — rather than only failing at execution. This is the production
//! safeguard that stops a dangerous experiment from entering the system at all:
//! safety-guard protected namespaces, blast-radius bounds, required parameters,
//! duration, and cron validity.

use serde::{Deserialize, Serialize};

use crate::engine::validate_experiment;
use crate::models::ChaosExperiment;
use crate::schedule::validate_cron_expression;

/// The webhook decision, mirroring Kubernetes `AdmissionResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmissionResponse {
    pub allowed: bool,
    pub message: String,
}

impl AdmissionResponse {
    fn allow() -> Self {
        AdmissionResponse { allowed: true, message: "admitted".to_string() }
    }
    fn deny(message: impl Into<String>) -> Self {
        AdmissionResponse { allowed: false, message: message.into() }
    }
}

/// Validate an experiment at admission time. The order matters: cheap structural
/// checks (name, blast radius, safety guard) precede the type-specific parameter
/// validation so the most actionable message surfaces first.
pub fn validate_experiment_admission(exp: &ChaosExperiment) -> AdmissionResponse {
    if exp.name.trim().is_empty() {
        return AdmissionResponse::deny("experiment name must not be empty");
    }

    let br = &exp.blast_radius;
    if !(br.max_pod_fraction > 0.0 && br.max_pod_fraction <= 1.0) {
        return AdmissionResponse::deny(format!(
            "blast radius max_pod_fraction must be in (0.0, 1.0], got {}",
            br.max_pod_fraction
        ));
    }
    if br.max_pods == Some(0) {
        return AdmissionResponse::deny("blast radius max_pods must be > 0");
    }

    let guard = &exp.safety_guard;
    if guard.enabled
        && guard
            .protected_namespaces
            .iter()
            .any(|n| n == &exp.target.namespace)
    {
        return AdmissionResponse::deny(format!(
            "namespace '{}' is protected by safety guard",
            exp.target.namespace
        ));
    }

    let errors = validate_experiment(exp);
    if !errors.is_empty() {
        return AdmissionResponse::deny(errors.join("; "));
    }

    AdmissionResponse::allow()
}

/// Validate a schedule's cron expression at admission time.
pub fn validate_schedule_admission(cron_expression: &str) -> AdmissionResponse {
    match validate_cron_expression(cron_expression) {
        Ok(_) => AdmissionResponse::allow(),
        Err(e) => AdmissionResponse::deny(e),
    }
}
