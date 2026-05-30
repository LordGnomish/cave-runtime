// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! queue-proxy sidecar — concurrency enforcement + request reporting.
//! upstream: knative/serving knative-v1.22.0 — pkg/queue/breaker.go

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle 1: BreakerParams + Semaphore + Breaker ────────────────────────

    #[test]
    fn breaker_params_reject_zero_queue_depth() {
        let p = BreakerParams {
            queue_depth: 0,
            max_concurrency: 10,
            initial_capacity: 0,
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn breaker_params_reject_initial_above_max() {
        let p = BreakerParams {
            queue_depth: 10,
            max_concurrency: 5,
            initial_capacity: 6,
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn breaker_params_accept_valid() {
        let p = BreakerParams {
            queue_depth: 10,
            max_concurrency: 5,
            initial_capacity: 5,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn semaphore_try_acquire_respects_capacity() {
        let s = Semaphore::new(8, 2);
        assert_eq!(s.capacity(), 2);
        assert!(s.try_acquire()); // in_flight 0 -> 1
        assert!(s.try_acquire()); // in_flight 1 -> 2 == capacity
        assert!(!s.try_acquire()); // at capacity -> reject
        assert_eq!(s.in_flight(), 2);
    }

    #[test]
    fn semaphore_release_frees_a_slot() {
        let s = Semaphore::new(8, 1);
        assert!(s.try_acquire());
        assert!(!s.try_acquire());
        s.release();
        assert_eq!(s.in_flight(), 0);
        assert!(s.try_acquire());
    }

    #[test]
    fn semaphore_update_capacity_raises_and_lowers() {
        let s = Semaphore::new(8, 1);
        s.update_capacity(4);
        assert_eq!(s.capacity(), 4);
        s.update_capacity(2);
        assert_eq!(s.capacity(), 2);
    }

    #[test]
    fn semaphore_update_capacity_clamps_to_max() {
        let s = Semaphore::new(3, 1);
        s.update_capacity(99);
        assert_eq!(s.capacity(), 3, "capacity cannot exceed max_capacity");
    }

    #[test]
    fn breaker_total_slots_is_queue_plus_concurrency() {
        let b = Breaker::new(BreakerParams {
            queue_depth: 7,
            max_concurrency: 3,
            initial_capacity: 3,
        })
        .unwrap();
        assert_eq!(b.total_slots(), 10);
        assert_eq!(b.capacity(), 3);
    }

    #[test]
    fn breaker_acquires_active_slot_until_capacity() {
        let b = Breaker::new(BreakerParams {
            queue_depth: 5,
            max_concurrency: 2,
            initial_capacity: 2,
        })
        .unwrap();
        // first two get active slots
        assert_eq!(b.try_acquire().unwrap(), true);
        assert_eq!(b.try_acquire().unwrap(), true);
        // third has no active slot -> queued (Ok(false)), in_flight grows
        assert_eq!(b.try_acquire().unwrap(), false);
        assert_eq!(b.in_flight(), 3);
    }

    #[test]
    fn breaker_rejects_when_queue_full() {
        let b = Breaker::new(BreakerParams {
            queue_depth: 1,
            max_concurrency: 1,
            initial_capacity: 1,
        })
        .unwrap();
        // total_slots = 2
        assert!(b.try_acquire().is_ok()); // active
        assert!(b.try_acquire().is_ok()); // queued (in_flight == 2)
        // third must be rejected with QueueFull, in_flight unchanged
        assert_eq!(b.try_acquire().unwrap_err(), BreakerError::QueueFull);
        assert_eq!(b.in_flight(), 2);
    }

    #[test]
    fn breaker_release_active_returns_slot() {
        let b = Breaker::new(BreakerParams {
            queue_depth: 2,
            max_concurrency: 1,
            initial_capacity: 1,
        })
        .unwrap();
        assert_eq!(b.try_acquire().unwrap(), true); // active
        b.release(true); // release active slot + pending
        assert_eq!(b.in_flight(), 0);
        assert_eq!(b.try_acquire().unwrap(), true); // active slot available again
    }

    #[test]
    fn breaker_update_concurrency_changes_capacity() {
        let b = Breaker::new(BreakerParams {
            queue_depth: 5,
            max_concurrency: 4,
            initial_capacity: 1,
        })
        .unwrap();
        assert_eq!(b.capacity(), 1);
        b.update_concurrency(3);
        assert_eq!(b.capacity(), 3);
    }
}
