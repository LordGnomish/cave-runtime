// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Compliance — Compliance & audit engine.
//! Compatible with: Manual CIS Benchmark checks, SOC2/PCI-DSS/HIPAA audit tooling.
//! Features: Automated control checks, evidence collection, audit trail, policy mapping.

pub mod models;
pub mod frameworks;
pub mod checks;
pub mod evidence;
pub mod audit;
pub mod policy;
pub mod reports;
pub mod routes;

use axum::Router;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use models::*;

pub struct ComplianceStore {
    pub frameworks: HashMap<uuid::Uuid, ComplianceFramework>,
    pub controls: HashMap<uuid::Uuid, Control>,
    pub findings: HashMap<uuid::Uuid, Finding>,
    pub evidence: HashMap<uuid::Uuid, Evidence>,
    pub audit_events: Vec<AuditEvent>,
    pub exceptions: HashMap<uuid::Uuid, ControlException>,
    pub reports: HashMap<uuid::Uuid, ComplianceReport>,
    pub policy_mappings: HashMap<uuid::Uuid, PolicyMapping>,
}

impl Default for ComplianceStore {
    fn default() -> Self {
        let mut store = ComplianceStore {
            frameworks: HashMap::new(),
            controls: HashMap::new(),
            findings: HashMap::new(),
            evidence: HashMap::new(),
            audit_events: Vec::new(),
            exceptions: HashMap::new(),
            reports: HashMap::new(),
            policy_mappings: HashMap::new(),
        };
        // Seed built-in frameworks
        for fw in frameworks::builtin_frameworks() {
            for ctrl in fw.controls.iter() {
                store.controls.insert(ctrl.id, ctrl.clone());
            }
            store.frameworks.insert(fw.id, fw);
        }
        store
    }
}

pub struct ComplianceState {
    pub store: Arc<RwLock<ComplianceStore>>,
}

impl Default for ComplianceState {
    fn default() -> Self {
        Self { store: Arc::new(RwLock::new(ComplianceStore::default())) }
    }
}

pub fn router(state: Arc<ComplianceState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "compliance";
