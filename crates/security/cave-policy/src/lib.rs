// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-policy — Policy engine replacing OPA Gatekeeper + OPAL + Kyverno.
//!
//! ## OPA Rego engine
//! Full Rego language: rules, functions, comprehensions, every/some/with/default,
//! 150+ built-in functions, partial evaluation, bundles, decision logging.
//!
//! ## Kyverno engine
//! ClusterPolicy/Policy: validate/mutate/generate/verifyImages, JMESPath variable
//! substitution, PolicyReports, CleanupPolicies, PolicyExceptions.
//!
//! ## K8s Admission Webhook
//! ValidatingWebhookConfiguration + MutatingWebhookConfiguration compatible.
//! AdmissionReview v1 API, namespace/object selectors, fail-open/fail-closed.

pub mod admission;
pub mod bundle;
pub mod decision_log;
pub mod error;
pub mod expansion;
pub mod http_send;
pub mod kyverno;
pub mod models;
pub mod rego;
pub mod routes;
pub mod store;

#[cfg(test)]
mod parity_self_audit;

use axum::Router;
use cave_db::CavePool;
use std::sync::{Arc, RwLock};

/// Shared state for the policy module.
pub struct PolicyState {
    /// Database connection pool.
    pub pool: Arc<CavePool>,
    /// OPA Rego policy engine (shared, RwLock for concurrent read access).
    pub rego: Arc<RwLock<rego::PolicyEngine>>,
    /// Kyverno policy engine.
    pub kyverno: Arc<RwLock<kyverno::KyvernoEngine>>,
    /// Admission webhook handler.
    pub webhook: Arc<RwLock<admission::AdmissionWebhook>>,
    /// Bundle manager.
    pub bundles: Arc<RwLock<bundle::BundleManager>>,
    /// Decision log.
    pub decision_log: Arc<decision_log::DecisionLog>,
    /// Fail-open mode for the admission webhook.
    pub fail_open: bool,
}

impl PolicyState {
    pub fn new(pool: Arc<CavePool>) -> Self {
        Self {
            pool,
            rego: Arc::new(RwLock::new(rego::PolicyEngine::new())),
            kyverno: Arc::new(RwLock::new(kyverno::KyvernoEngine::new())),
            webhook: Arc::new(RwLock::new(admission::AdmissionWebhook::default())),
            bundles: Arc::new(RwLock::new(bundle::BundleManager::new())),
            decision_log: Arc::new(decision_log::DecisionLog::new(10_000)),
            fail_open: false,
        }
    }
}

/// Legacy State alias (used by cave-runtime).
pub struct State {
    pub pool: Arc<CavePool>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    let policy_state = Arc::new(PolicyState::new(state.pool.clone()));
    routes::create_router(policy_state)
}

/// Module name for DB schema and logging.
pub const MODULE_NAME: &str = "policy";
