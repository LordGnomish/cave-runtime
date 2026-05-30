// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! queue-proxy sidecar — concurrency enforcement + request reporting.
//! upstream: knative/serving knative-v1.22.0 — pkg/queue/breaker.go

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// `math.MaxInt32` — upstream caps `MaxConcurrency` here.
const MAX_BREAKER_CAPACITY: i64 = i32::MAX as i64;

/// Parameters for a [`Breaker`] — mirrors upstream `BreakerParams`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerParams {
    /// Pending-request queue capacity (must be > 0).
    pub queue_depth: i64,
    /// Maximum concurrent executions (>= 0, capped at `math.MaxInt32`).
    pub max_concurrency: i64,
    /// Starting active slots (0..=max_concurrency).
    pub initial_capacity: i64,
}

impl BreakerParams {
    /// Validate the params the way `NewBreaker` panics-or-accepts upstream.
    pub fn validate(&self) -> Result<(), String> {
        if self.queue_depth <= 0 {
            return Err(format!("queue_depth must be > 0, got {}", self.queue_depth));
        }
        if self.max_concurrency < 0 {
            return Err(format!(
                "max_concurrency must be >= 0, got {}",
                self.max_concurrency
            ));
        }
        if self.max_concurrency > MAX_BREAKER_CAPACITY {
            return Err(format!(
                "max_concurrency must be <= {}, got {}",
                MAX_BREAKER_CAPACITY, self.max_concurrency
            ));
        }
        if self.initial_capacity < 0 || self.initial_capacity > self.max_concurrency {
            return Err(format!(
                "initial_capacity must be in [0, {}], got {}",
                self.max_concurrency, self.initial_capacity
            ));
        }
        Ok(())
    }
}

/// Lock-free counting semaphore — packs `capacity` (high 32 bits) and
/// `in_flight` (low 32 bits) into a single atomic `u64`, exactly as
/// upstream's `semaphore.state` does.
#[derive(Debug)]
pub struct Semaphore {
    state: AtomicU64,
    max_capacity: u32,
}

impl Semaphore {
    /// Create a semaphore with a hard `max_capacity` and a starting
    /// `initial_capacity` (clamped into `0..=max_capacity`).
    pub fn new(max_capacity: u32, initial_capacity: u32) -> Self {
        let cap = initial_capacity.min(max_capacity);
        Self {
            state: AtomicU64::new(pack(cap, 0)),
            max_capacity,
        }
    }

    fn load(&self) -> (u32, u32) {
        unpack(self.state.load(Ordering::Acquire))
    }

    /// Currently allowed concurrent executions.
    pub fn capacity(&self) -> u32 {
        self.load().0
    }

    /// Currently active (acquired) slots.
    pub fn in_flight(&self) -> u32 {
        self.load().1
    }

    /// Non-blocking acquire: take a slot iff `in_flight < capacity`.
    pub fn try_acquire(&self) -> bool {
        loop {
            let cur = self.state.load(Ordering::Acquire);
            let (cap, inflight) = unpack(cur);
            if inflight >= cap {
                return false;
            }
            let next = pack(cap, inflight + 1);
            if self
                .state
                .compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Release one active slot.
    pub fn release(&self) {
        loop {
            let cur = self.state.load(Ordering::Acquire);
            let (cap, inflight) = unpack(cur);
            debug_assert!(inflight > 0, "release without matching acquire");
            if inflight == 0 {
                return;
            }
            let next = pack(cap, inflight - 1);
            if self
                .state
                .compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }
        }
    }

    /// Atomically set a new capacity (clamped to `max_capacity`).
    pub fn update_capacity(&self, size: u32) {
        let size = size.min(self.max_capacity);
        loop {
            let cur = self.state.load(Ordering::Acquire);
            let (_cap, inflight) = unpack(cur);
            let next = pack(size, inflight);
            if self
                .state
                .compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }
        }
    }
}

#[inline]
fn pack(capacity: u32, in_flight: u32) -> u64 {
    ((capacity as u64) << 32) | (in_flight as u64)
}

#[inline]
fn unpack(state: u64) -> (u32, u32) {
    ((state >> 32) as u32, (state & 0xFFFF_FFFF) as u32)
}

/// Why a [`Breaker`] turned a request away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerError {
    /// Pending queue + active slots are all full (`ErrRequestQueueFull`).
    QueueFull,
}

/// Concurrency limiter with a bounded pending queue.
///
/// `total_slots = queue_depth + max_concurrency`. A request first claims a
/// pending slot (rejected with [`BreakerError::QueueFull`] once `in_flight`
/// reaches `total_slots`), then tries for an active execution slot via the
/// inner [`Semaphore`].
#[derive(Debug)]
pub struct Breaker {
    in_flight: AtomicI64,
    total_slots: i64,
    sem: Semaphore,
}

impl Breaker {
    /// Build a breaker from validated params.
    pub fn new(params: BreakerParams) -> Result<Self, String> {
        params.validate()?;
        Ok(Self {
            in_flight: AtomicI64::new(0),
            total_slots: params.queue_depth + params.max_concurrency,
            sem: Semaphore::new(params.max_concurrency as u32, params.initial_capacity as u32),
        })
    }

    /// `queue_depth + max_concurrency`.
    pub fn total_slots(&self) -> i64 {
        self.total_slots
    }

    /// Active concurrency capacity right now.
    pub fn capacity(&self) -> u32 {
        self.sem.capacity()
    }

    /// Pending + active requests currently held.
    pub fn in_flight(&self) -> i64 {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Try to admit a request.
    ///
    /// * `Err(QueueFull)` — no pending slot left.
    /// * `Ok(true)`  — admitted with an active execution slot.
    /// * `Ok(false)` — queued (pending slot held), waiting for capacity.
    pub fn try_acquire(&self) -> Result<bool, BreakerError> {
        // Phase 1: reserve a pending slot.
        loop {
            let cur = self.in_flight.load(Ordering::Acquire);
            if cur >= self.total_slots {
                return Err(BreakerError::QueueFull);
            }
            if self
                .in_flight
                .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        // Phase 2: try for an active execution slot.
        Ok(self.sem.try_acquire())
    }

    /// Release a request. `had_active` must reflect the [`try_acquire`]
    /// return value so the active slot is only freed if it was taken.
    ///
    /// [`try_acquire`]: Breaker::try_acquire
    pub fn release(&self, had_active: bool) {
        if had_active {
            self.sem.release();
        }
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }

    /// Dynamically resize the active-concurrency window.
    pub fn update_concurrency(&self, size: u32) {
        self.sem.update_capacity(size);
    }
}

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
