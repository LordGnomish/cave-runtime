// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory SLO store — thread-safe CRUD + aggregate stats.
//! Mirrors the nobl9-go SDK's Object CRUD surface for SLO resources.

use crate::models::{SLO, SloStats, SloStatus};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// Thread-safe in-memory store for SLO objects.
#[derive(Default)]
pub struct SloStore {
    pub slos: RwLock<HashMap<Uuid, SLO>>,
}

impl SloStore {
    /// Create a new shared store instance.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Insert (or overwrite) an SLO.
    pub fn insert(&self, slo: SLO) {
        self.slos.write().unwrap().insert(slo.id, slo);
    }

    /// Retrieve an SLO by ID.
    pub fn get(&self, id: Uuid) -> Option<SLO> {
        self.slos.read().unwrap().get(&id).cloned()
    }

    /// List all SLOs (unordered).
    pub fn list(&self) -> Vec<SLO> {
        self.slos.read().unwrap().values().cloned().collect()
    }

    /// Update an existing SLO. Returns `true` if the ID existed.
    pub fn update(&self, slo: SLO) -> bool {
        let mut map = self.slos.write().unwrap();
        if map.contains_key(&slo.id) {
            map.insert(slo.id, slo);
            true
        } else {
            false
        }
    }

    /// Delete an SLO by ID. Returns `true` if the ID existed.
    pub fn delete(&self, id: Uuid) -> bool {
        self.slos.write().unwrap().remove(&id).is_some()
    }

    /// Compute aggregate statistics across all stored SLOs.
    pub fn compute_stats(&self) -> SloStats {
        let slos = self.slos.read().unwrap();
        let mut stats = SloStats::default();
        stats.total = slos.len() as u64;

        let mut compliance_sum = 0.0_f64;
        for slo in slos.values() {
            match slo.status {
                SloStatus::Ok => stats.ok += 1,
                SloStatus::AtRisk => stats.at_risk += 1,
                SloStatus::Breaching => stats.breaching += 1,
                SloStatus::Breached => stats.breached += 1,
                SloStatus::Unknown => {}
            }
            compliance_sum += slo.current_sli;
        }

        stats.avg_compliance = if stats.total > 0 {
            compliance_sum / stats.total as f64
        } else {
            0.0
        };

        stats
    }
}
