//! Circuit breaker — per-upstream, per-target.
//! States: Closed → Open (on failures) → HalfOpen → Closed (on success)

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CbState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug)]
struct Breaker {
    state: CbState,
    failure_count: u32,
    success_count: u32,
    last_failure: Option<Instant>,
    open_at: Option<Instant>,
}

impl Default for Breaker {
    fn default() -> Self {
        Self {
            state: CbState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure: None,
            open_at: None,
        }
    }
}

#[derive(Clone)]
pub struct CircuitBreakerRegistry {
    breakers: Arc<DashMap<Uuid, Breaker>>,
    /// How many consecutive failures to open the circuit
    pub failure_threshold: u32,
    /// How many consecutive successes to close from HalfOpen
    pub success_threshold: u32,
    /// How long to wait in Open state before trying HalfOpen
    pub timeout: Duration,
}

impl CircuitBreakerRegistry {
    pub fn new(failure_threshold: u32, success_threshold: u32, timeout_secs: u64) -> Self {
        Self {
            breakers: Arc::new(DashMap::new()),
            failure_threshold,
            success_threshold,
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Returns true if a request is allowed (circuit not open).
    pub fn allow(&self, target_id: Uuid) -> bool {
        let mut entry = self.breakers.entry(target_id).or_insert_with(Breaker::default);
        match entry.state {
            CbState::Closed => true,
            CbState::HalfOpen => true,
            CbState::Open => {
                // Check if timeout has elapsed → transition to HalfOpen
                if let Some(open_at) = entry.open_at {
                    if open_at.elapsed() >= self.timeout {
                        entry.state = CbState::HalfOpen;
                        entry.success_count = 0;
                        return true;
                    }
                }
                false
            }
        }
    }

    pub fn on_success(&self, target_id: Uuid) {
        let mut entry = self.breakers.entry(target_id).or_insert_with(Breaker::default);
        match entry.state {
            CbState::HalfOpen => {
                entry.success_count += 1;
                if entry.success_count >= self.success_threshold {
                    entry.state = CbState::Closed;
                    entry.failure_count = 0;
                    entry.success_count = 0;
                }
            }
            CbState::Closed => {
                entry.failure_count = 0;
            }
            CbState::Open => {}
        }
    }

    pub fn on_failure(&self, target_id: Uuid) {
        let mut entry = self.breakers.entry(target_id).or_insert_with(Breaker::default);
        entry.failure_count += 1;
        entry.last_failure = Some(Instant::now());
        if entry.failure_count >= self.failure_threshold {
            if entry.state != CbState::Open {
                warn!(target=%target_id, failures=entry.failure_count, "circuit breaker opened");
            }
            entry.state = CbState::Open;
            entry.open_at = Some(Instant::now());
            entry.success_count = 0;
        }
    }

    pub fn get_state(&self, target_id: Uuid) -> CbState {
        self.breakers
            .get(&target_id)
            .map(|b| b.state.clone())
            .unwrap_or(CbState::Closed)
    }

    pub fn reset(&self, target_id: Uuid) {
        self.breakers.remove(&target_id);
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
}
