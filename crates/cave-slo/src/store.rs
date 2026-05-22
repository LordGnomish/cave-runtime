// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{Slo, SloStats, SloStatus};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Default)]
pub struct SloStore {
    pub slos: RwLock<HashMap<Uuid, Slo>>,
}

impl SloStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn insert(&self, slo: Slo) {
        self.slos.write().unwrap().insert(slo.id, slo);
    }

    pub fn get(&self, id: Uuid) -> Option<Slo> {
        self.slos.read().unwrap().get(&id).cloned()
    }

    pub fn list(&self) -> Vec<Slo> {
        self.slos.read().unwrap().values().cloned().collect()
    }

    pub fn update(&self, slo: Slo) -> bool {
        let mut map = self.slos.write().unwrap();
        if map.contains_key(&slo.id) {
            map.insert(slo.id, slo);
            true
        } else {
            false
        }
    }

    pub fn delete(&self, id: Uuid) -> bool {
        self.slos.write().unwrap().remove(&id).is_some()
    }

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
