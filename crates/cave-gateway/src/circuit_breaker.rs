// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-upstream-target circuit breaker.
//!
//! **Sweep-005 adoption (2026-05-12)** — this module is now a thin wrapper
//! around `cave_kernel::circuitbreaker` using the new
//! `TripCondition::Consecutive` mode. The kernel's `PerKeyBreakers` owns
//! the state map; this file preserves the gateway-facing public API
//! (`CircuitBreakerRegistry`, `CbState`, `allow`/`on_success`/`on_failure`/
//! `get_state`/`reset`) so callers in `proxy.rs` and tests don't need to
//! change.
//!
//! Semantics preserved (Envoy/Istio outlier-detection model):
//!   - Closed → Open after `failure_threshold` *consecutive* failures.
//!   - Open → HalfOpen once `timeout` elapses, on the next `allow()`.
//!   - HalfOpen → Closed after `success_threshold` consecutive successes.
//!   - Any failure in HalfOpen → Open (resets the timer).

use cave_kernel::circuitbreaker::{BreakerConfig, BreakerState, CircuitBreaker, PerKeyBreakers};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Gateway-facing state name. Mirrors `BreakerState` but kept as its own
/// enum so the gateway can evolve its UI labels without churning the
/// kernel module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CbState {
    Closed,
    Open,
    HalfOpen,
}

impl From<BreakerState> for CbState {
    fn from(s: BreakerState) -> Self {
        match s {
            BreakerState::Closed => CbState::Closed,
            BreakerState::Open => CbState::Open,
            BreakerState::HalfOpen => CbState::HalfOpen,
        }
    }
}

/// Per-target circuit breaker registry. Delegates to
/// `cave_kernel::circuitbreaker::PerKeyBreakers`.
#[derive(Clone)]
pub struct CircuitBreakerRegistry {
    breakers: Arc<PerKeyBreakers>,
    /// How many consecutive failures to open the circuit.
    pub failure_threshold: u32,
    /// How many consecutive successes to close from HalfOpen.
    pub success_threshold: u32,
    /// How long to wait in Open state before trying HalfOpen.
    pub timeout: Duration,
}

impl CircuitBreakerRegistry {
    pub fn new(failure_threshold: u32, success_threshold: u32, timeout_secs: u64) -> Self {
        let timeout = Duration::from_secs(timeout_secs);
        let cfg = BreakerConfig::consecutive(failure_threshold, success_threshold, timeout);
        Self {
            breakers: Arc::new(PerKeyBreakers::new(cfg)),
            failure_threshold,
            success_threshold,
            timeout,
        }
    }

    fn breaker(&self, target_id: Uuid) -> CircuitBreaker {
        self.breakers.for_key(&target_id.to_string())
    }

    /// Returns true if a request is allowed (circuit not Open, or the
    /// reset timeout has elapsed and the breaker is moving to HalfOpen).
    pub fn allow(&self, target_id: Uuid) -> bool {
        self.breaker(target_id).can_proceed()
    }

    pub fn on_success(&self, target_id: Uuid) {
        self.breaker(target_id).record_success();
    }

    pub fn on_failure(&self, target_id: Uuid) {
        self.breaker(target_id).record_failure();
    }

    pub fn get_state(&self, target_id: Uuid) -> CbState {
        self.breaker(target_id).state().into()
    }

    /// Forget the breaker state for this target. The next `allow()` call
    /// will create a fresh Closed breaker (kernel's
    /// `PerKeyBreakers::remove` drops the per-key entry).
    pub fn reset(&self, target_id: Uuid) {
        self.breakers.remove(&target_id.to_string());
    }
}

impl Default for CircuitBreakerRegistry {
    fn default() -> Self {
        Self::new(5, 2, 30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_after_threshold() {
        let cb = CircuitBreakerRegistry::new(3, 2, 60);
        let id = Uuid::new_v4();
        assert!(cb.allow(id));
        for _ in 0..3 {
            cb.on_failure(id);
        }
        assert!(!cb.allow(id));
        assert_eq!(cb.get_state(id), CbState::Open);
    }

    #[test]
    fn closes_after_success() {
        let cb = CircuitBreakerRegistry::new(3, 2, 0); // 0s timeout for testing
        let id = Uuid::new_v4();
        for _ in 0..3 {
            cb.on_failure(id);
        }
        // immediate timeout → half-open
        assert!(cb.allow(id));
        cb.on_success(id);
        cb.on_success(id);
        assert_eq!(cb.get_state(id), CbState::Closed);
    }

    #[test]
    fn reset_clears_state() {
        let cb = CircuitBreakerRegistry::new(2, 1, 60);
        let id = Uuid::new_v4();
        cb.on_failure(id);
        cb.on_failure(id);
        cb.reset(id);
        assert!(cb.allow(id));
        assert_eq!(cb.get_state(id), CbState::Closed);
    }

    /// Sweep-005 regression — a single success inside the failure streak
    /// must reset the trip counter (Envoy outlier semantics, preserved
    /// from the pre-adoption local impl).
    #[test]
    fn success_resets_streak() {
        let cb = CircuitBreakerRegistry::new(3, 1, 60);
        let id = Uuid::new_v4();
        cb.on_failure(id);
        cb.on_failure(id);
        cb.on_success(id); // resets the streak
        cb.on_failure(id);
        cb.on_failure(id);
        assert!(cb.allow(id), "2 trailing failures < 3 threshold");
        cb.on_failure(id);
        assert!(!cb.allow(id));
        assert_eq!(cb.get_state(id), CbState::Open);
    }
}
