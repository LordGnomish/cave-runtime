//! CAVE Compliance — SOC2/ISO27001/GDPR/HIPAA/PCI-DSS tracking.
//!
//! Provides automated evidence collection, control assessment, gap detection,
//! and compliance reporting across the CAVE module graph.
//!
//! ## Upstream Tracking: Vanta / Drata / Tugboat Logic
//! - Replaces point-in-time manual audits with continuous automated checks.
//! - Controls mapped directly to CAVE module state (cave-auth, cave-vault, …).

pub mod engine;
pub mod mapping;
pub mod models;
pub mod routes;

use axum::Router;
use models::{Assessment, AuditTrail, Evidence, PolicyDocument, Remediation, Risk};
use std::sync::{Arc, Mutex};

// ────────────────────────────────────────────────────────────────────────────
// In-memory store
// ────────────────────────────────────────────────────────────────────────────

/// All in-memory compliance data.
///
/// Seeded with the built-in control library at startup.
/// Future: back this with PostgreSQL via cave-db.
pub struct ComplianceStore {
    pub controls: Vec<models::Control>,
    pub evidences: Vec<Evidence>,
    pub assessments: Vec<Assessment>,
    pub audit_trail: Vec<AuditTrail>,
    pub risks: Vec<Risk>,
    pub policy_documents: Vec<PolicyDocument>,
    pub remediations: Vec<Remediation>,
}

impl Default for ComplianceStore {
    fn default() -> Self {
        Self {
            controls: mapping::seed_controls(),
            evidences: Vec::new(),
            assessments: Vec::new(),
            audit_trail: Vec::new(),
            risks: Vec::new(),
            policy_documents: Vec::new(),
            remediations: Vec::new(),
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Module state
// ────────────────────────────────────────────────────────────────────────────

/// State shared across all compliance request handlers.
pub struct ComplianceState {
    pub store: Mutex<ComplianceStore>,
}

impl Default for ComplianceState {
    fn default() -> Self {
        Self {
            store: Mutex::new(ComplianceStore::default()),
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Public API
// ────────────────────────────────────────────────────────────────────────────

/// Create the axum router for the compliance module.
pub fn router(state: Arc<ComplianceState>) -> Router {
    routes::create_router(state)
}

/// Module name used for logging and future DB schema namespacing.
pub const MODULE_NAME: &str = "compliance";
