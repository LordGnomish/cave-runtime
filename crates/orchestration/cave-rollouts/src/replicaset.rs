// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ReplicaSet replica-count calculator — Argo Rollouts parity.
//!
//! Pure-function port of `utils/replicaset/canary.go` (argoproj/argo-rollouts
//! v1.9.0): translate a desired canary *traffic weight* into concrete canary /
//! stable ReplicaSet replica counts, honouring `maxSurge` / `maxUnavailable`
//! fenceposts and a `minPodsPerReplicaSet` floor. The live ReplicaSet scale
//! actions belong to cave-controller-manager; this module ships the arithmetic
//! the controller drives.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_to_replicas_ceils() {
        // ceil(weight/maxWeight * specReplicas)
        assert_eq!(traffic_weight_to_replicas(10, 20, 100), 2);
        assert_eq!(traffic_weight_to_replicas(10, 25, 100), 3); // ceil(2.5)
        assert_eq!(traffic_weight_to_replicas(4, 10, 100), 1); // ceil(0.4)
        assert_eq!(traffic_weight_to_replicas(10, 0, 100), 0);
        assert_eq!(traffic_weight_to_replicas(10, 100, 100), 10);
    }

    #[test]
    fn min_pods_floor_applies_only_when_nonzero() {
        assert_eq!(check_min_pods_per_replica_set(1, Some(2)), 2);
        assert_eq!(check_min_pods_per_replica_set(3, Some(2)), 3);
        assert_eq!(check_min_pods_per_replica_set(0, Some(2)), 0); // zero is left alone
        assert_eq!(check_min_pods_per_replica_set(1, None), 1);
    }

    #[test]
    fn traffic_routed_static_stable_keeps_full_spec() {
        let (canary, stable) =
            calculate_replica_counts_for_traffic_routed_canary(10, 30, 100, None, false);
        assert_eq!(canary, 3);
        assert_eq!(stable, 10); // !dynamicStableScale → stable stays at spec
    }

    #[test]
    fn traffic_routed_dynamic_stable_shrinks_inverse() {
        let (canary, stable) =
            calculate_replica_counts_for_traffic_routed_canary(10, 30, 100, None, true);
        assert_eq!(canary, 3); // ceil(30/100*10)
        assert_eq!(stable, 7); // ceil(70/100*10)
    }

    #[test]
    fn traffic_routed_dynamic_stable_honours_min_pods() {
        // weight 95% → canary ceil(9.5)=10, stable ceil(0.5)=1 → floor to 2
        let (canary, stable) =
            calculate_replica_counts_for_traffic_routed_canary(10, 95, 100, Some(2), true);
        assert_eq!(canary, 10);
        assert_eq!(stable, 2);
    }

    #[test]
    fn basic_canary_splits_spec() {
        assert_eq!(calculate_replica_counts_for_basic_canary(10, 30, 100), (3, 7));
        assert_eq!(calculate_replica_counts_for_basic_canary(10, 100, 100), (10, 0));
        assert_eq!(calculate_replica_counts_for_basic_canary(10, 0, 100), (0, 10));
    }

    #[test]
    fn fenceposts_bound_surge_and_availability() {
        assert_eq!(max_replica_count_allowed(10, 2), 12);
        assert_eq!(min_available_replica_count(10, 2), 8);
        assert_eq!(min_available_replica_count(3, 5), 0); // never negative
    }
}
