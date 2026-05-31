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

/// `trafficWeightToReplicas` — `ceil(weight / maxWeight * specReplicas)`.
///
/// The number of replicas a ReplicaSet needs to *approximately* serve
/// `weight`/`maxWeight` of traffic when no traffic router is splitting requests
/// by header/weight (i.e. traffic follows pod count).
pub fn traffic_weight_to_replicas(spec_replicas: i32, weight: i32, max_weight: i32) -> i32 {
    if max_weight <= 0 || spec_replicas <= 0 || weight <= 0 {
        return 0;
    }
    ((weight as f64) / (max_weight as f64) * (spec_replicas as f64)).ceil() as i32
}

/// `CheckMinPodsPerReplicaSet` — raise a *non-zero* count up to the HA floor.
///
/// A zero count is left untouched (a ReplicaSet that should be fully scaled
/// down is not forced back up), matching upstream behaviour.
pub fn check_min_pods_per_replica_set(count: i32, min_pods: Option<i32>) -> i32 {
    match min_pods {
        Some(min) if count != 0 && count < min => min,
        _ => count,
    }
}

/// `CalculateReplicaCountsForTrafficRoutedCanary` — weighted form.
///
/// With a traffic router in play the canary ReplicaSet only needs enough pods to
/// back the weighted slice. When `dynamic_stable_scale` is set the stable
/// ReplicaSet is shrunk inversely (`maxWeight - desiredWeight`); otherwise it
/// stays at full `spec_replicas`. Returns `(canary_count, stable_count)`.
pub fn calculate_replica_counts_for_traffic_routed_canary(
    spec_replicas: i32,
    desired_weight: i32,
    max_weight: i32,
    min_pods: Option<i32>,
    dynamic_stable_scale: bool,
) -> (i32, i32) {
    let canary = check_min_pods_per_replica_set(
        traffic_weight_to_replicas(spec_replicas, desired_weight, max_weight),
        min_pods,
    );
    if !dynamic_stable_scale {
        return (canary, spec_replicas);
    }
    let stable = check_min_pods_per_replica_set(
        traffic_weight_to_replicas(spec_replicas, max_weight - desired_weight, max_weight),
        min_pods,
    );
    (canary, stable)
}

/// Basic (non-traffic-routed) canary split.
///
/// Without a traffic router, request share *is* pod share, so the canary count
/// is the ceil-weighted replica count (capped at spec) and the stable count is
/// the remainder. Returns `(canary_count, stable_count)`.
pub fn calculate_replica_counts_for_basic_canary(
    spec_replicas: i32,
    desired_weight: i32,
    max_weight: i32,
) -> (i32, i32) {
    let canary =
        traffic_weight_to_replicas(spec_replicas, desired_weight, max_weight).min(spec_replicas);
    let stable = (spec_replicas - canary).max(0);
    (canary, stable)
}

/// `maxReplicaCountAllowed` fencepost: `specReplicas + maxSurge`.
pub fn max_replica_count_allowed(spec_replicas: i32, max_surge: i32) -> i32 {
    spec_replicas + max_surge
}

/// `minAvailableReplicaCount` fencepost: `max(0, specReplicas - maxUnavailable)`.
pub fn min_available_replica_count(spec_replicas: i32, max_unavailable: i32) -> i32 {
    (spec_replicas - max_unavailable).max(0)
}

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
