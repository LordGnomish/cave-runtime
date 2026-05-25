// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-zone eviction queue — `pkg/controller/nodelifecycle/scheduler/rate_limited_queue.go`.
//!
//! Two queues per zone: a primary (full QPS) and a secondary that kicks in
//! during partial disruption. The secondary runs at a lower QPS so eviction
//! storms are dampened.

use super::zone_state::ZoneState;
use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EvictionConfig {
    pub primary_qps: f64,
    pub secondary_qps: f64,
}

impl Default for EvictionConfig {
    fn default() -> Self {
        Self {
            primary_qps: 0.1,
            secondary_qps: 0.01,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EvictionQueue {
    pending: VecDeque<String>,
}

impl EvictionQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, node: impl Into<String>) {
        self.pending.push_back(node.into());
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Pop up to `take` nodes for eviction this tick. Mirrors
    /// `RateLimitedTimedQueue.Try`.
    pub fn drain_tick(&mut self, take: usize) -> Vec<String> {
        let take = take.min(self.pending.len());
        self.pending.drain(0..take).collect()
    }
}

/// Effective per-tick eviction count given zone state.
///
/// Mirrors the QPS gating in `monitorNodeHealth`: partial → secondary,
/// full → 0, normal/initial → primary.
pub fn tick_budget(state: ZoneState, cfg: &EvictionConfig, tick_seconds: u32) -> u32 {
    let qps = match state {
        ZoneState::Normal | ZoneState::Initial => cfg.primary_qps,
        ZoneState::PartialDisruption => cfg.secondary_qps,
        ZoneState::FullDisruption => 0.0,
    };
    (qps * tick_seconds as f64).round() as u32
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/nodelifecycle/scheduler/rate_limited_queue.go",
    "RateLimitedTimedQueue",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn enqueue_increments_length() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/scheduler/rate_limited_queue.go",
            "Add",
            "tenant-nl-evic-enqueue"
        );
        let mut q = EvictionQueue::new();
        q.enqueue("n1");
        q.enqueue("n2");
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn drain_tick_takes_up_to_budget() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/scheduler/rate_limited_queue.go",
            "Try",
            "tenant-nl-evic-drain"
        );
        let mut q = EvictionQueue::new();
        for i in 0..5 {
            q.enqueue(format!("n{i}"));
        }
        let taken = q.drain_tick(3);
        assert_eq!(taken.len(), 3);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn drain_tick_caps_at_pending_count() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/scheduler/rate_limited_queue.go",
            "Try",
            "tenant-nl-evic-drain-cap"
        );
        let mut q = EvictionQueue::new();
        q.enqueue("n1");
        let taken = q.drain_tick(10);
        assert_eq!(taken.len(), 1);
        assert!(q.is_empty());
    }

    #[test]
    fn full_disruption_emits_zero_budget() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "doNoExecuteTaintingPass",
            "tenant-nl-evic-full"
        );
        let cfg = EvictionConfig::default();
        assert_eq!(tick_budget(ZoneState::FullDisruption, &cfg, 60), 0);
    }

    #[test]
    fn partial_disruption_uses_secondary_qps() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "doNoExecuteTaintingPass",
            "tenant-nl-evic-partial"
        );
        let cfg = EvictionConfig::default();
        // 0.01 * 60 = 0.6 → rounds to 1.
        assert_eq!(tick_budget(ZoneState::PartialDisruption, &cfg, 60), 1);
    }

    #[test]
    fn normal_uses_primary_qps() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "doNoExecuteTaintingPass",
            "tenant-nl-evic-normal"
        );
        let cfg = EvictionConfig::default();
        // 0.1 * 60 = 6.
        assert_eq!(tick_budget(ZoneState::Normal, &cfg, 60), 6);
    }

    #[test]
    fn initial_zone_uses_primary_qps() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-nl-evic-initial"
        );
        let cfg = EvictionConfig::default();
        assert!(tick_budget(ZoneState::Initial, &cfg, 60) > 0);
    }

    #[test]
    fn defaults_match_upstream_qps() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "defaults",
            "tenant-nl-evic-defaults"
        );
        let cfg = EvictionConfig::default();
        assert!((cfg.primary_qps - 0.1).abs() < 1e-9);
        assert!((cfg.secondary_qps - 0.01).abs() < 1e-9);
    }
}
