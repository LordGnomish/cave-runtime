// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Graceful node-shutdown handler — KEP-2000.
//!
//! Mirrors `pkg/kubelet/nodeshutdown/` from upstream. The
//! production path integrates with systemd-inhibitor; this
//! module implements the *ordering* state machine so the
//! kubelet knows in what order to terminate pods when a
//! shutdown signal arrives.
//!
//! Order of termination (upstream contract):
//!
//! 1. Non-critical pods first.
//! 2. Critical (`system-cluster-critical`,
//!    `system-node-critical`) pods last.
//! 3. Within each tier, lowest-priority first, equal priority
//!    in stable name order.
//!
//! Each pod is allowed `shutdownGracePeriod` seconds; if the
//! total shutdown budget runs out mid-batch, every remaining
//! pod is force-killed.

use std::time::Duration;

/// Where a pod sits in the shutdown precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShutdownTier {
    /// Non-critical workload — terminated first.
    Regular,
    /// `system-cluster-critical` priority class.
    ClusterCritical,
    /// `system-node-critical` priority class — terminated last.
    NodeCritical,
}

/// One pod observed by the shutdown ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownCandidate {
    pub pod_uid: String,
    pub name: String,
    pub priority: i32,
    pub tier: ShutdownTier,
}

/// Configuration for the shutdown handler. Matches upstream's
/// kubelet config `shutdownGracePeriod` /
/// `shutdownGracePeriodCriticalPods`.
#[derive(Debug, Clone, Copy)]
pub struct ShutdownConfig {
    pub total_grace: Duration,
    pub critical_grace: Duration,
}

impl ShutdownConfig {
    pub fn new(total_grace: Duration, critical_grace: Duration) -> Self {
        Self {
            total_grace,
            critical_grace,
        }
    }

    /// Budget for non-critical pods = total − critical reserve.
    /// Saturates to zero if mis-configured.
    pub fn regular_grace(&self) -> Duration {
        self.total_grace.saturating_sub(self.critical_grace)
    }
}

/// What the kubelet should do for each pod in shutdown order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownStep {
    pub pod_uid: String,
    pub name: String,
    pub tier: ShutdownTier,
    /// How long this individual pod has to terminate before
    /// SIGKILL.
    pub grace: Duration,
}

/// Build the ordered shutdown plan. Deterministic in `pods`'
/// iteration plus stable name tie-break.
pub fn build_plan(pods: &[ShutdownCandidate], cfg: ShutdownConfig) -> Vec<ShutdownStep> {
    let mut sorted: Vec<&ShutdownCandidate> = pods.iter().collect();
    sorted.sort_by(|a, b| {
        // Tier ascending — Regular before ClusterCritical
        // before NodeCritical.
        a.tier
            .cmp(&b.tier)
            // Within a tier, lower priority first.
            .then(a.priority.cmp(&b.priority))
            // Finally a stable tie-break by name.
            .then(a.name.cmp(&b.name))
    });

    let reg = cfg.regular_grace();
    let crit = cfg.critical_grace;
    sorted
        .into_iter()
        .map(|p| {
            let grace = match p.tier {
                ShutdownTier::Regular => reg,
                ShutdownTier::ClusterCritical | ShutdownTier::NodeCritical => crit,
            };
            ShutdownStep {
                pod_uid: p.pod_uid.clone(),
                name: p.name.clone(),
                tier: p.tier,
                grace,
            }
        })
        .collect()
}

/// Tracks how much of the total shutdown budget has already
/// elapsed. `step_consumed(d)` records that the previous step
/// took `d`; `remaining()` reports what's left for the next
/// step. The kubelet uses this to detect "budget exhausted —
/// SIGKILL everything remaining" outcomes.
#[derive(Debug, Clone)]
pub struct ShutdownClock {
    cfg: ShutdownConfig,
    spent: Duration,
}

impl ShutdownClock {
    pub fn new(cfg: ShutdownConfig) -> Self {
        Self {
            cfg,
            spent: Duration::ZERO,
        }
    }

    pub fn step_consumed(&mut self, d: Duration) {
        self.spent = self.spent.saturating_add(d);
    }

    pub fn remaining(&self) -> Duration {
        self.cfg.total_grace.saturating_sub(self.spent)
    }

    pub fn exhausted(&self) -> bool {
        self.remaining() == Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(total: u64, crit: u64) -> ShutdownConfig {
        ShutdownConfig::new(Duration::from_secs(total), Duration::from_secs(crit))
    }

    fn candidate(uid: &str, name: &str, priority: i32, tier: ShutdownTier) -> ShutdownCandidate {
        ShutdownCandidate {
            pod_uid: uid.into(),
            name: name.into(),
            priority,
            tier,
        }
    }

    #[test]
    fn regular_grace_subtracts_critical() {
        let c = cfg(60, 10);
        assert_eq!(c.regular_grace(), Duration::from_secs(50));
    }

    #[test]
    fn regular_grace_saturates_to_zero() {
        let c = cfg(10, 30);
        assert_eq!(c.regular_grace(), Duration::ZERO);
    }

    #[test]
    fn plan_orders_regular_before_critical() {
        let pods = vec![
            candidate("a", "a", 100, ShutdownTier::NodeCritical),
            candidate("b", "b", 50, ShutdownTier::Regular),
            candidate("c", "c", 75, ShutdownTier::ClusterCritical),
        ];
        let plan = build_plan(&pods, cfg(60, 10));
        let order: Vec<&str> = plan.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, vec!["b", "c", "a"]);
    }

    #[test]
    fn plan_within_tier_orders_lower_priority_first() {
        let pods = vec![
            candidate("a", "a", 100, ShutdownTier::Regular),
            candidate("b", "b", 50, ShutdownTier::Regular),
            candidate("c", "c", 75, ShutdownTier::Regular),
        ];
        let plan = build_plan(&pods, cfg(60, 10));
        let order: Vec<&str> = plan.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, vec!["b", "c", "a"]);
    }

    #[test]
    fn plan_breaks_ties_by_name() {
        let pods = vec![
            candidate("a", "z", 100, ShutdownTier::Regular),
            candidate("b", "a", 100, ShutdownTier::Regular),
        ];
        let plan = build_plan(&pods, cfg(60, 10));
        let order: Vec<&str> = plan.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, vec!["a", "z"]);
    }

    #[test]
    fn plan_assigns_regular_grace_to_regular_pods() {
        let pods = vec![candidate("a", "a", 0, ShutdownTier::Regular)];
        let plan = build_plan(&pods, cfg(60, 10));
        assert_eq!(plan[0].grace, Duration::from_secs(50));
    }

    #[test]
    fn plan_assigns_critical_grace_to_critical_pods() {
        let pods = vec![candidate("a", "a", 0, ShutdownTier::NodeCritical)];
        let plan = build_plan(&pods, cfg(60, 10));
        assert_eq!(plan[0].grace, Duration::from_secs(10));
    }

    #[test]
    fn plan_empty_input_returns_empty() {
        let plan = build_plan(&[], cfg(60, 10));
        assert!(plan.is_empty());
    }

    #[test]
    fn clock_remaining_decreases_per_step() {
        let mut c = ShutdownClock::new(cfg(60, 10));
        assert_eq!(c.remaining(), Duration::from_secs(60));
        c.step_consumed(Duration::from_secs(15));
        assert_eq!(c.remaining(), Duration::from_secs(45));
    }

    #[test]
    fn clock_exhausted_after_full_budget() {
        let mut c = ShutdownClock::new(cfg(60, 10));
        c.step_consumed(Duration::from_secs(70));
        assert!(c.exhausted());
        assert_eq!(c.remaining(), Duration::ZERO);
    }

    #[test]
    fn cluster_critical_orders_after_regular_before_node_critical() {
        let pods = vec![
            candidate("a", "a", 0, ShutdownTier::ClusterCritical),
            candidate("b", "b", 0, ShutdownTier::NodeCritical),
            candidate("c", "c", 0, ShutdownTier::Regular),
        ];
        let plan = build_plan(&pods, cfg(60, 10));
        let order: Vec<&str> = plan.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, vec!["c", "a", "b"]);
    }
}
