// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! activator — cold-start request hold, capacity partitioning, load balancing.
//! upstream: knative/serving knative-v1.22.0 — pkg/activator/net/{throttler,lb_policy}.go

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle 3: calculate_capacity + min_one_or_value + InfiniteBreaker ────

    #[test]
    fn min_one_or_value_floors_at_one() {
        assert_eq!(min_one_or_value(0), 1);
        assert_eq!(min_one_or_value(1), 1);
        assert_eq!(min_one_or_value(5), 5);
        assert_eq!(min_one_or_value(-3), 1);
    }

    #[test]
    fn capacity_pod_direct_is_cc_times_trackers() {
        let rt = RevisionThrottler::new(10);
        // numTrackers > 0 -> containerConcurrency * numTrackers
        assert_eq!(rt.calculate_capacity(5, 3, 4), 30);
    }

    #[test]
    fn capacity_clusterip_splits_across_activators() {
        let rt = RevisionThrottler::new(10);
        // numTrackers == 0 -> cc*backend / activatorCount = 10*8/4 = 20
        assert_eq!(rt.calculate_capacity(8, 0, 4), 20);
    }

    #[test]
    fn capacity_clusterip_floors_at_one_when_split_rounds_down() {
        let rt = RevisionThrottler::new(1);
        // 1*3 / 4 = 0 -> minOneOrValue -> 1
        assert_eq!(rt.calculate_capacity(3, 0, 4), 1);
    }

    #[test]
    fn capacity_zero_when_no_backends() {
        let rt = RevisionThrottler::new(10);
        assert_eq!(rt.calculate_capacity(0, 0, 4), 0);
    }

    #[test]
    fn capacity_infinite_concurrency_clamps_to_revision_max() {
        let rt = RevisionThrottler::new(0); // containerConcurrency == 0 => infinite
        // backendCount > 0 && cc == 0 -> revisionMaxConcurrency
        assert_eq!(rt.calculate_capacity(2, 0, 4), REVISION_MAX_CONCURRENCY);
        // but with zero backends it stays 0
        assert_eq!(rt.calculate_capacity(0, 0, 4), 0);
    }

    #[test]
    fn capacity_overflow_clamps_to_revision_max() {
        let rt = RevisionThrottler::new(i32::MAX as i64);
        // cc * trackers overflows revisionMaxConcurrency -> clamp
        assert_eq!(rt.calculate_capacity(1, 2, 1), REVISION_MAX_CONCURRENCY);
    }

    #[test]
    fn infinite_breaker_gates_on_capacity() {
        let b = InfiniteBreaker::new();
        assert!(!b.has_capacity(), "starts closed");
        b.update_concurrency(5);
        assert!(b.has_capacity(), "any positive backend count opens it");
        assert_eq!(b.capacity(), 1, "infinite breaker reports 0 or 1");
        b.update_concurrency(0);
        assert!(!b.has_capacity(), "scaled back to zero closes it");
        assert_eq!(b.capacity(), 0);
    }
}
