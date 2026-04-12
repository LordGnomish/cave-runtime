//! cave-policy — OPA replacement for the CAVE runtime.
//!
//! Implements a Rego-compatible policy engine with:
//! - Rego-subset language (rules, packages, imports, default values, comprehensions)
//! - Policy evaluation engine (input + data → decisions)
//! - Built-in functions (string, regex, JWT, HTTP, time, crypto, aggregates)
//! - Bundle system (versioned policy packages)
//! - Decision logging
//! - Kubernetes ValidatingAdmissionWebhook support
//! - OPA-compatible admin API (/v1/policies, /v1/data, /v1/compile, /v1/config)
//!
//! Replaces: OPA Gatekeeper + OPAL

<<<<<<< HEAD
pub mod engine;
pub mod models;
=======
pub mod admission;
pub mod bundle;
pub mod decision_log;
pub mod engine;
>>>>>>> claude/wizardly-goldstine
pub mod routes;

use axum::Router;
use bundle::BundleStore;
use decision_log::DecisionLog;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared state for the policy module.
pub struct State {
    /// Compiled Rego policies, keyed by policy ID.
    pub policies: RwLock<HashMap<String, String>>,
    /// Bundle store.
    pub bundles: RwLock<BundleStore>,
    /// Decision log.
    pub decision_log: DecisionLog,
}

impl State {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            policies: RwLock::new(HashMap::new()),
            bundles: RwLock::new(BundleStore::new()),
            decision_log: DecisionLog::default(),
        })
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            policies: RwLock::new(HashMap::new()),
            bundles: RwLock::new(BundleStore::new()),
            decision_log: DecisionLog::default(),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "policy";
