// SPDX-License-Identifier: AGPL-3.0-or-later
//! ScaledObject CRD — autoscale Deployment/StatefulSet/Custom workloads.
//! upstream: kedacore/keda v2.x — apis/keda/v1alpha1/scaledobject_types.go

use std::time::{Duration, Instant};

#[derive(Default, Debug, Clone)]
pub struct ScaledObject {
    pub tenant_id: String,
    pub min_replica_count: Option<i32>,
    pub max_replica_count: Option<i32>,
    pub polling_interval: Option<Duration>,
    pub cooldown_period: Option<Duration>,
    pub idle_replica_count: Option<i32>,
    pub current_replicas: i32,
    /// Last time at least one trigger reported active (for cooldown logic).
    pub last_active_at: Option<Instant>,
    pub paused: bool,
}

impl ScaledObject {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            min_replica_count: Some(0),
            max_replica_count: Some(100),
            polling_interval: Some(Duration::from_secs(30)),
            cooldown_period: Some(Duration::from_secs(300)),
            idle_replica_count: None,
            current_replicas: 0,
            last_active_at: None,
            paused: false,
        }
    }

    /// Force the workload to its idle replica count (KEDA's idleReplicaCount;
    /// falls back to min_replica_count if unset, then 0).
    pub fn scale_to_zero(&mut self) {
        self.current_replicas = self
            .idle_replica_count
            .or(self.min_replica_count)
            .unwrap_or(0)
            .max(0);
    }

    /// Reconcile the desired replica count given the active state of triggers
    /// and a recommended replica count from the scalers.
    /// Mirrors `pkg/scaling/executor` cooldown semantics.
    pub fn reconcile(&mut self, recommended: i32, any_active: bool, now: Instant) -> i32 {
        if self.paused {
            return self.current_replicas;
        }
        let min = self.min_replica_count.unwrap_or(0);
        let max = self.max_replica_count.unwrap_or(i32::MAX);

        if any_active {
            self.last_active_at = Some(now);
            let desired = recommended.max(min.max(1)).min(max);
            self.current_replicas = desired;
            return desired;
        }

        // No active triggers — apply cooldown before scaling to zero.
        if let Some(last) = self.last_active_at {
            let cooldown = self.cooldown_period.unwrap_or(Duration::from_secs(300));
            if now.duration_since(last) < cooldown {
                // Still within cooldown — hold replicas
                return self.current_replicas;
            }
        }
        // Past cooldown (or never active) — scale to idle/min/0
        let target = self
            .idle_replica_count
            .or(self.min_replica_count)
            .unwrap_or(0)
            .max(0);
        self.current_replicas = target;
        target
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }
}
