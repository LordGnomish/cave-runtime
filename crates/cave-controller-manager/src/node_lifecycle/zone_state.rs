// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Zone state classifier — `pkg/controller/nodelifecycle/node_lifecycle_controller.go::ComputeZoneState`.
//!
//! Each topology zone in the cluster has a state that drives the per-zone
//! pod-eviction rate limiter:
//!
//! * `Initial` — fewer than 3 nodes observed; controller hasn't yet
//!   classified.
//! * `Normal` — small fraction unhealthy; full eviction speed.
//! * `PartialDisruption` — between
//!   `unhealthy_zone_threshold` (default 0.55) and 1.0 of nodes are
//!   not-ready; reduce to `secondary_evictor_qps`.
//! * `FullDisruption` — every node is not-ready; eviction halted.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneState {
    Initial,
    Normal,
    PartialDisruption,
    FullDisruption,
}

pub const UNHEALTHY_ZONE_THRESHOLD_FRACTION: f64 = 0.55;
pub const LARGE_CLUSTER_NODE_THRESHOLD: u32 = 50;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ZoneConfig {
    pub unhealthy_threshold: f64,
    pub large_cluster_node_threshold: u32,
}

impl Default for ZoneConfig {
    fn default() -> Self {
        Self {
            unhealthy_threshold: UNHEALTHY_ZONE_THRESHOLD_FRACTION,
            large_cluster_node_threshold: LARGE_CLUSTER_NODE_THRESHOLD,
        }
    }
}

/// Classify a zone's state.
///
/// Mirrors `ComputeZoneState`:
///
/// * `total < 3` → `Initial`.
/// * `total - ready == 0` → `Normal`.
/// * `total - ready == total` → `FullDisruption`.
/// * Otherwise compute the unhealthy fraction; in clusters above
///   `large_cluster_node_threshold`, only flip to `PartialDisruption` when
///   the fraction exceeds the threshold.
pub fn compute_zone_state(ready_nodes: u32, total_nodes: u32, cfg: &ZoneConfig) -> ZoneState {
    if total_nodes < 3 {
        return ZoneState::Initial;
    }
    if total_nodes == ready_nodes {
        return ZoneState::Normal;
    }
    let unhealthy = total_nodes - ready_nodes;
    if unhealthy == total_nodes {
        return ZoneState::FullDisruption;
    }
    let fraction = unhealthy as f64 / total_nodes as f64;
    // Small clusters: any unhealthy node => partial.
    if total_nodes <= cfg.large_cluster_node_threshold {
        return ZoneState::PartialDisruption;
    }
    // Large clusters: gate by threshold.
    if fraction >= cfg.unhealthy_threshold {
        ZoneState::PartialDisruption
    } else {
        ZoneState::Normal
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
    "ComputeZoneState",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn cfg() -> ZoneConfig {
        ZoneConfig::default()
    }

    #[test]
    fn fewer_than_three_nodes_is_initial() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "ComputeZoneState",
            "tenant-nl-zone-initial"
        );
        assert_eq!(compute_zone_state(2, 2, &cfg()), ZoneState::Initial);
        assert_eq!(compute_zone_state(0, 0, &cfg()), ZoneState::Initial);
    }

    #[test]
    fn all_ready_is_normal() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "ComputeZoneState",
            "tenant-nl-zone-normal"
        );
        assert_eq!(compute_zone_state(10, 10, &cfg()), ZoneState::Normal);
    }

    #[test]
    fn all_unready_is_full_disruption() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "ComputeZoneState",
            "tenant-nl-zone-full"
        );
        assert_eq!(compute_zone_state(0, 10, &cfg()), ZoneState::FullDisruption);
    }

    #[test]
    fn small_cluster_any_unhealthy_is_partial() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "ComputeZoneState",
            "tenant-nl-zone-small-partial"
        );
        // Cluster of 5 nodes, 1 unhealthy → partial.
        assert_eq!(
            compute_zone_state(4, 5, &cfg()),
            ZoneState::PartialDisruption
        );
    }

    #[test]
    fn large_cluster_below_threshold_stays_normal() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "ComputeZoneState",
            "tenant-nl-zone-large-normal"
        );
        // 100 nodes, 10 unhealthy = 10% → < 55% threshold → Normal.
        assert_eq!(compute_zone_state(90, 100, &cfg()), ZoneState::Normal);
    }

    #[test]
    fn large_cluster_above_threshold_is_partial() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "ComputeZoneState",
            "tenant-nl-zone-large-partial"
        );
        // 100 nodes, 60 unhealthy = 60% → > 55% threshold → Partial.
        assert_eq!(
            compute_zone_state(40, 100, &cfg()),
            ZoneState::PartialDisruption
        );
    }

    #[test]
    fn defaults_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "defaults",
            "tenant-nl-zone-defaults"
        );
        assert_eq!(UNHEALTHY_ZONE_THRESHOLD_FRACTION, 0.55);
        assert_eq!(LARGE_CLUSTER_NODE_THRESHOLD, 50);
    }

    #[test]
    fn zone_state_serializes_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "ZoneState",
            "tenant-nl-zone-serde"
        );
        for s in [
            ZoneState::Initial,
            ZoneState::Normal,
            ZoneState::PartialDisruption,
            ZoneState::FullDisruption,
        ] {
            let bytes = serde_json::to_string(&s).unwrap();
            let back: ZoneState = serde_json::from_str(&bytes).unwrap();
            assert_eq!(s, back);
        }
    }
}
