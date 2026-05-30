// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! activator — cold-start request hold, capacity partitioning, load balancing.
//! upstream: knative/serving knative-v1.22.0 — pkg/activator/net/{throttler,lb_policy}.go

use std::sync::atomic::{AtomicU32, Ordering};

/// Per-revision concurrency ceiling — upstream `revisionMaxConcurrency`,
/// equal to `queue.MaxBreakerCapacity` (`math.MaxInt32`).
pub const REVISION_MAX_CONCURRENCY: i64 = i32::MAX as i64;

/// `minOneOrValue` — division-safety floor preventing a zero quotient.
pub fn min_one_or_value(num: i64) -> i64 {
    if num > 1 {
        num
    } else {
        1
    }
}

/// Per-revision throttler — owns the breaker capacity the activator hands
/// out for a single revision, partitioning total backend concurrency across
/// the fleet of activator instances.
#[derive(Debug, Clone)]
pub struct RevisionThrottler {
    /// `containerConcurrency` — 0 means unbounded (infinite breaker).
    pub container_concurrency: i64,
}

impl RevisionThrottler {
    /// Build a throttler for a revision with the given container concurrency.
    pub fn new(container_concurrency: i64) -> Self {
        Self {
            container_concurrency,
        }
    }

    /// Compute this activator's slice of the revision's total capacity.
    ///
    /// * pod-direct (`num_trackers > 0`): `cc * num_trackers`.
    /// * ClusterIP (`num_trackers == 0`): `cc * backend / activators`, floored at 1.
    /// * unbounded (`cc == 0`) or overflow past [`REVISION_MAX_CONCURRENCY`]
    ///   with live backends: clamps to [`REVISION_MAX_CONCURRENCY`].
    pub fn calculate_capacity(
        &self,
        backend_count: i64,
        num_trackers: i64,
        activator_count: i64,
    ) -> i64 {
        let mut target = if num_trackers > 0 {
            self.container_concurrency * num_trackers
        } else {
            let t = self.container_concurrency * backend_count;
            if t > 0 {
                min_one_or_value(t / min_one_or_value(activator_count))
            } else {
                t
            }
        };
        if backend_count > 0
            && (self.container_concurrency == 0 || target > REVISION_MAX_CONCURRENCY)
        {
            target = REVISION_MAX_CONCURRENCY;
        }
        target
    }
}

/// Breaker variant used when `containerConcurrency == 0` (unbounded).
///
/// Upstream's `infiniteBreaker` reports a capacity of 0 or 1 and uses a
/// broadcast to unblock held requests once any backend is ready; we model
/// the gate as an open/closed flag driven by the live backend count.
#[derive(Debug)]
pub struct InfiniteBreaker {
    /// 0 (closed, scaled-to-zero) or 1 (open, backends available).
    capacity: AtomicU32,
}

impl Default for InfiniteBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl InfiniteBreaker {
    /// A closed (capacity-0) breaker.
    pub fn new() -> Self {
        Self {
            capacity: AtomicU32::new(0),
        }
    }

    /// Open the gate iff `backend_count > 0`, else close it.
    pub fn update_concurrency(&self, backend_count: i64) {
        let v = if backend_count > 0 { 1 } else { 0 };
        self.capacity.store(v, Ordering::Release);
    }

    /// Reported capacity — 0 or 1.
    pub fn capacity(&self) -> u32 {
        self.capacity.load(Ordering::Acquire)
    }

    /// Whether a held request may proceed immediately.
    pub fn has_capacity(&self) -> bool {
        self.capacity() > 0
    }
}

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

    // ── Cycle 4: pick_indices + assign_slice + pick_p2c + retry ─────────────

    #[test]
    fn pick_indices_more_activators_than_trackers_is_round_robin() {
        // numActivators(5) > numTrackers(3): selfIndex % numTrackers, width 1
        assert_eq!(pick_indices(3, 0, 5), (0, 1, 0));
        assert_eq!(pick_indices(3, 4, 5), (1, 2, 0)); // 4 % 3 == 1
    }

    #[test]
    fn pick_indices_even_split_no_remnants() {
        // 6 trackers / 3 activators = slice 2, no remnant
        assert_eq!(pick_indices(6, 0, 3), (0, 2, 0));
        assert_eq!(pick_indices(6, 2, 3), (4, 6, 0));
    }

    #[test]
    fn pick_indices_uneven_split_has_remnants() {
        // 7 trackers / 3 activators = slice 2, remnant 1
        assert_eq!(pick_indices(7, 0, 3), (0, 2, 1));
        assert_eq!(pick_indices(7, 2, 3), (4, 6, 1));
    }

    #[test]
    fn assign_slice_single_activator_takes_all() {
        let trackers = vec![10u32, 11, 12, 13];
        assert_eq!(assign_slice(&trackers, 0, 1), trackers);
    }

    #[test]
    fn assign_slice_one_or_zero_trackers_returns_all() {
        let trackers = vec![10u32];
        assert_eq!(assign_slice(&trackers, 0, 3), trackers);
        assert_eq!(assign_slice(&[], 0, 3), Vec::<u32>::new());
    }

    #[test]
    fn assign_slice_even_split_partitions() {
        let trackers = vec![10u32, 11, 12, 13, 14, 15];
        assert_eq!(assign_slice(&trackers, 0, 3), vec![10, 11]);
        assert_eq!(assign_slice(&trackers, 1, 3), vec![12, 13]);
        assert_eq!(assign_slice(&trackers, 2, 3), vec![14, 15]);
    }

    #[test]
    fn assign_slice_remnant_goes_to_low_index_activators() {
        // 7 trackers, 3 activators: slice 2, remnant 1 (the last tracker, id 16).
        let trackers = vec![10u32, 11, 12, 13, 14, 15, 16];
        // tail = [16]; only selfIndex 0 (< len(tail)=1) gets the remnant.
        assert_eq!(assign_slice(&trackers, 0, 3), vec![10, 11, 16]);
        assert_eq!(assign_slice(&trackers, 1, 3), vec![12, 13]);
        assert_eq!(assign_slice(&trackers, 2, 3), vec![14, 15]);
    }

    #[test]
    fn p2c_empty_is_none_single_is_zero() {
        assert_eq!(pick_p2c(&[], 0, 0, false), None);
        assert_eq!(pick_p2c(&[7], 0, 0, false), Some(0));
    }

    #[test]
    fn p2c_picks_lower_weight_of_two() {
        // weights: idx0=5, idx1=2, idx2=9. r1=0, r2=1 -> compare 5 vs 2 -> pick idx1
        assert_eq!(pick_p2c(&[5, 2, 9], 0, 1, false), Some(1));
    }

    #[test]
    fn p2c_shifts_second_index_to_avoid_collision() {
        // r1=1, r2=1 -> r2 >= r1 so r2 becomes 2; compare idx1=2 vs idx2=9 -> idx1
        assert_eq!(pick_p2c(&[5, 2, 9], 1, 1, false), Some(1));
    }

    #[test]
    fn p2c_tie_uses_coin() {
        // equal weights at idx0 and idx1; coin=false keeps pick(r1), coin=true takes alt
        assert_eq!(pick_p2c(&[4, 4], 0, 0, false), Some(0));
        assert_eq!(pick_p2c(&[4, 4], 0, 0, true), Some(1));
    }

    #[test]
    fn retry_policy_retries_503_with_exponential_backoff() {
        let p = RetryPolicy {
            max_retries: 3,
            base_backoff_ms: 100,
        };
        assert_eq!(p.should_retry(503, 0), Some(100)); // 100 * 2^0
        assert_eq!(p.should_retry(503, 1), Some(200)); // 100 * 2^1
        assert_eq!(p.should_retry(503, 2), Some(400)); // 100 * 2^2
        assert_eq!(p.should_retry(503, 3), None); // exhausted
    }

    #[test]
    fn retry_policy_does_not_retry_success_or_4xx() {
        let p = RetryPolicy {
            max_retries: 5,
            base_backoff_ms: 50,
        };
        assert_eq!(p.should_retry(200, 0), None);
        assert_eq!(p.should_retry(404, 0), None);
        assert_eq!(p.should_retry(500, 0), None, "only 503 (cold-start) retries");
    }
}
