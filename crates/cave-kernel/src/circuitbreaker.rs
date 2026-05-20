// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Circuit breaker — Closed → Open → HalfOpen state machine, per-key.
//!
//! Upstream cite: `resilience4j` (Java) reference for state model + sliding
//! window semantics. Reimpl in pure Rust, no JVM. Two window flavors:
//!   - `Count` — last N calls
//!   - `Time(d)` — calls within last `d` duration
//!
//! Transitions:
//!   - Closed: every failure is recorded; if failure rate ≥ threshold within
//!     window AND minimum calls met → Open.
//!   - Open: rejects everything; after `reset_timeout`, single trial allowed
//!     (first `can_proceed` call moves state to HalfOpen and returns true).
//!   - HalfOpen: allows up to `half_open_permitted` trial calls. After that
//!     many successes → Closed; any failure → Open (and resets timer).

use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowKind {
    /// Last N calls.
    Count(usize),
    /// Calls within the trailing duration.
    Time(Duration),
}

/// What flips the breaker from Closed → Open.
///
/// Two flavours, both observed in production breakers:
///
/// * `WindowedRate` — resilience4j model. Failure **rate** over the last
///   `minimum_calls` (or window-bounded) calls hits `threshold`. Good for
///   high-traffic upstreams where a few bad responses shouldn't trip.
/// * `Consecutive` — Envoy/Istio outlier-detection model used by
///   `cave-gateway`. Strictly N consecutive failures (`count`) → Open.
///   Half-open recovery requires `success_count` consecutive successes.
///   Simpler to reason about for low-traffic upstreams where rate-based
///   detection is too noisy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TripCondition {
    WindowedRate {
        failure_rate_threshold: f64,
        minimum_calls: usize,
    },
    Consecutive {
        failure_count: u32,
        success_count: u32,
    },
}

#[derive(Debug, Clone)]
pub struct BreakerConfig {
    pub window: WindowKind,
    pub trip: TripCondition,
    pub reset_timeout: Duration,
    pub half_open_permitted: usize,
}

// Field-style accessors that pre-date `TripCondition`. Kept as `pub fn`
// so the older `cfg.failure_rate_threshold` / `cfg.minimum_calls` call
// sites still compile after the enum split — they panic-free degrade to
// 0/0 when the breaker is configured for the Consecutive trip mode.
impl BreakerConfig {
    pub fn new(window: WindowKind, threshold: f64, minimum_calls: usize, reset: Duration) -> Self {
        assert!((0.0..=1.0).contains(&threshold), "threshold ∈ [0,1]");
        assert!(minimum_calls > 0, "minimum_calls must be > 0");
        Self {
            window,
            trip: TripCondition::WindowedRate {
                failure_rate_threshold: threshold,
                minimum_calls,
            },
            reset_timeout: reset,
            half_open_permitted: 1,
        }
    }

    /// Construct a breaker tuned for **Envoy/Istio outlier-detection**
    /// semantics: trip after `failure_count` consecutive failures; close
    /// from HalfOpen after `success_count` consecutive successes. The
    /// `window` is forced to `Count(failure_count)` because consecutive
    /// counting only needs `failure_count` samples to make a decision.
    pub fn consecutive(failure_count: u32, success_count: u32, reset: Duration) -> Self {
        assert!(failure_count > 0, "failure_count must be > 0");
        assert!(success_count > 0, "success_count must be > 0");
        Self {
            window: WindowKind::Count(failure_count as usize),
            trip: TripCondition::Consecutive {
                failure_count,
                success_count,
            },
            reset_timeout: reset,
            half_open_permitted: success_count as usize,
        }
    }

    pub fn with_half_open_permitted(mut self, n: usize) -> Self {
        assert!(n > 0, "half_open_permitted must be > 0");
        self.half_open_permitted = n;
        self
    }

    /// Legacy field accessor — returns the rate threshold when the breaker
    /// is in `WindowedRate` mode, or `1.0` in `Consecutive` mode (any
    /// failure rate at full saturation trivially trips on the consecutive
    /// path; callers should consult `self.trip` for the real semantics).
    pub fn failure_rate_threshold(&self) -> f64 {
        match self.trip {
            TripCondition::WindowedRate {
                failure_rate_threshold,
                ..
            } => failure_rate_threshold,
            TripCondition::Consecutive { .. } => 1.0,
        }
    }

    /// Legacy field accessor — returns the minimum-calls floor when the
    /// breaker is in `WindowedRate` mode, or the consecutive failure count
    /// in `Consecutive` mode (so the call-counting logic still has a sane
    /// "have we seen enough calls to decide" answer).
    pub fn minimum_calls(&self) -> usize {
        match self.trip {
            TripCondition::WindowedRate { minimum_calls, .. } => minimum_calls,
            TripCondition::Consecutive { failure_count, .. } => failure_count as usize,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Success,
    Failure,
}

#[derive(Debug)]
struct Sample {
    at: Instant,
    outcome: Outcome,
}

#[derive(Debug)]
struct BreakerInner {
    cfg: BreakerConfig,
    state: BreakerState,
    samples: VecDeque<Sample>,
    opened_at: Option<Instant>,
    half_open_attempts: usize,
    half_open_successes: usize,
}

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    inner: Arc<Mutex<BreakerInner>>,
}

impl CircuitBreaker {
    pub fn new(cfg: BreakerConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BreakerInner {
                cfg,
                state: BreakerState::Closed,
                samples: VecDeque::new(),
                opened_at: None,
                half_open_attempts: 0,
                half_open_successes: 0,
            })),
        }
    }

    pub fn state(&self) -> BreakerState {
        self.state_at(Instant::now())
    }

    pub fn state_at(&self, now: Instant) -> BreakerState {
        let inner = self.inner.lock();
        match inner.state {
            BreakerState::Open => {
                let opened_at = inner.opened_at.expect("Open implies opened_at");
                if now.saturating_duration_since(opened_at) >= inner.cfg.reset_timeout {
                    // Lazy peek — actual transition happens on can_proceed_at.
                    BreakerState::Open
                } else {
                    BreakerState::Open
                }
            }
            other => other,
        }
    }

    /// Check whether a call may proceed; transitions Open → HalfOpen on permit grant.
    pub fn can_proceed(&self) -> bool {
        self.can_proceed_at(Instant::now())
    }

    pub fn can_proceed_at(&self, now: Instant) -> bool {
        let mut inner = self.inner.lock();
        match inner.state {
            BreakerState::Closed => true,
            BreakerState::Open => {
                let opened_at = inner.opened_at.expect("Open implies opened_at");
                if now.saturating_duration_since(opened_at) >= inner.cfg.reset_timeout {
                    inner.state = BreakerState::HalfOpen;
                    inner.half_open_attempts = 1;
                    inner.half_open_successes = 0;
                    true
                } else {
                    false
                }
            }
            BreakerState::HalfOpen => {
                if inner.half_open_attempts < inner.cfg.half_open_permitted {
                    inner.half_open_attempts += 1;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn record_success(&self) {
        self.record_at(Instant::now(), Outcome::Success);
    }

    pub fn record_failure(&self) {
        self.record_at(Instant::now(), Outcome::Failure);
    }

    pub fn record_success_at(&self, now: Instant) {
        self.record_at(now, Outcome::Success);
    }

    pub fn record_failure_at(&self, now: Instant) {
        self.record_at(now, Outcome::Failure);
    }

    fn record_at(&self, now: Instant, outcome: Outcome) {
        let mut inner = self.inner.lock();
        match inner.state {
            BreakerState::HalfOpen => {
                if outcome == Outcome::Failure {
                    inner.state = BreakerState::Open;
                    inner.opened_at = Some(now);
                    inner.half_open_attempts = 0;
                    inner.half_open_successes = 0;
                    return;
                }
                inner.half_open_successes += 1;
                if inner.half_open_successes >= inner.cfg.half_open_permitted {
                    inner.state = BreakerState::Closed;
                    inner.samples.clear();
                    inner.opened_at = None;
                    inner.half_open_attempts = 0;
                    inner.half_open_successes = 0;
                }
                return;
            }
            BreakerState::Open => {
                // Should not happen — caller should have been rejected by can_proceed.
                return;
            }
            BreakerState::Closed => {}
        }

        inner.samples.push_back(Sample { at: now, outcome });

        // Trim window
        let cfg_window = inner.cfg.window;
        let cap = if let WindowKind::Count(n) = cfg_window {
            Some(n)
        } else {
            None
        };
        if let Some(cap) = cap {
            while inner.samples.len() > cap {
                inner.samples.pop_front();
            }
        }
        if let WindowKind::Time(d) = cfg_window {
            while let Some(front) = inner.samples.front() {
                if now.saturating_duration_since(front.at) > d {
                    inner.samples.pop_front();
                } else {
                    break;
                }
            }
        }

        // Evaluate trip — branch on the configured trip condition.
        let trip = inner.cfg.trip;
        let total = inner.samples.len();
        let should_open = match trip {
            TripCondition::WindowedRate {
                failure_rate_threshold,
                minimum_calls,
            } => {
                if total < minimum_calls {
                    false
                } else {
                    let failures = inner
                        .samples
                        .iter()
                        .filter(|s| s.outcome == Outcome::Failure)
                        .count();
                    let rate = failures as f64 / total as f64;
                    rate >= failure_rate_threshold
                }
            }
            TripCondition::Consecutive { failure_count, .. } => {
                // Walk samples from newest to oldest; count the trailing
                // failures. Envoy/Istio outlier-detection semantics: any
                // success resets the streak, so a single success in the
                // tail keeps the breaker Closed.
                let consecutive = inner
                    .samples
                    .iter()
                    .rev()
                    .take_while(|s| s.outcome == Outcome::Failure)
                    .count() as u32;
                consecutive >= failure_count
            }
        };
        if should_open {
            inner.state = BreakerState::Open;
            inner.opened_at = Some(now);
        }
    }

    pub fn current_failure_rate(&self) -> f64 {
        let inner = self.inner.lock();
        let total = inner.samples.len();
        if total == 0 {
            return 0.0;
        }
        let failures = inner
            .samples
            .iter()
            .filter(|s| s.outcome == Outcome::Failure)
            .count();
        failures as f64 / total as f64
    }
}

// ── Per-key registry ──────────────────────────────────────────────────────────

pub struct PerKeyBreakers {
    cfg: BreakerConfig,
    map: Mutex<HashMap<String, CircuitBreaker>>,
}

impl PerKeyBreakers {
    pub fn new(cfg: BreakerConfig) -> Self {
        Self {
            cfg,
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn for_key(&self, key: &str) -> CircuitBreaker {
        let mut m = self.map.lock();
        m.entry(key.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.cfg.clone()))
            .clone()
    }

    pub fn known_keys(&self) -> Vec<String> {
        let m = self.map.lock();
        let mut v: Vec<String> = m.keys().cloned().collect();
        v.sort();
        v
    }

    /// Drop the breaker for `key`. The next `for_key(key)` call yields a
    /// fresh Closed breaker. Used by Sweep-005 adopters that need an
    /// explicit operator-reset path (e.g. `gateway::CircuitBreakerRegistry::reset`).
    pub fn remove(&self, key: &str) -> bool {
        let mut m = self.map.lock();
        m.remove(key).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_cfg(threshold: f64, min: usize, reset_ms: u64) -> BreakerConfig {
        BreakerConfig::new(
            WindowKind::Count(10),
            threshold,
            min,
            Duration::from_millis(reset_ms),
        )
    }

    /// cite: resilience4j — fresh breaker is Closed
    #[test]
    fn breaker_acme_starts_closed() {
        let tenant_id = "acme";
        let b = CircuitBreaker::new(count_cfg(0.5, 4, 100));
        assert_eq!(b.state(), BreakerState::Closed);
        assert!(b.can_proceed());
        let _ = tenant_id;
    }

    /// cite: resilience4j — does not trip below minimum_calls
    #[test]
    fn breaker_acme_below_minimum_calls_stays_closed() {
        let tenant_id = "acme";
        let b = CircuitBreaker::new(count_cfg(0.5, 5, 100));
        for _ in 0..3 {
            b.record_failure();
        }
        assert_eq!(b.state(), BreakerState::Closed);
        let _ = tenant_id;
    }

    /// cite: resilience4j — trip when failure rate ≥ threshold and minimum met
    #[test]
    fn breaker_globex_trips_open_after_threshold() {
        let tenant_id = "globex";
        let b = CircuitBreaker::new(count_cfg(0.5, 4, 100));
        for _ in 0..4 {
            b.record_failure();
        }
        assert_eq!(b.state(), BreakerState::Open);
        assert!(!b.can_proceed());
        let _ = tenant_id;
    }

    /// cite: resilience4j — successes interleaved keep it Closed below threshold
    #[test]
    fn breaker_initech_below_threshold_stays_closed() {
        let tenant_id = "initech";
        let b = CircuitBreaker::new(count_cfg(0.6, 5, 100));
        // 5 calls, 2 failures = 0.4 rate, below 0.6
        b.record_failure();
        b.record_success();
        b.record_failure();
        b.record_success();
        b.record_success();
        assert_eq!(b.state(), BreakerState::Closed);
        let _ = tenant_id;
    }

    /// cite: resilience4j — Open → HalfOpen after reset_timeout
    #[test]
    fn breaker_acme_open_to_halfopen_after_reset_timeout() {
        let tenant_id = "acme";
        let cfg = count_cfg(0.5, 2, 50);
        let b = CircuitBreaker::new(cfg);
        b.record_failure();
        b.record_failure();
        assert_eq!(b.state(), BreakerState::Open);
        let later = Instant::now() + Duration::from_millis(60);
        assert!(b.can_proceed_at(later));
        assert_eq!(b.state(), BreakerState::HalfOpen);
        let _ = tenant_id;
    }

    /// cite: resilience4j — HalfOpen success closes the breaker
    #[test]
    fn breaker_acme_halfopen_success_closes() {
        let tenant_id = "acme";
        let cfg = count_cfg(0.5, 2, 10);
        let b = CircuitBreaker::new(cfg);
        b.record_failure();
        b.record_failure();
        let later = Instant::now() + Duration::from_millis(20);
        assert!(b.can_proceed_at(later));
        b.record_success_at(later);
        assert_eq!(b.state(), BreakerState::Closed);
        let _ = tenant_id;
    }

    /// cite: resilience4j — HalfOpen failure re-opens the breaker
    #[test]
    fn breaker_globex_halfopen_failure_reopens() {
        let tenant_id = "globex";
        let cfg = count_cfg(0.5, 2, 10);
        let b = CircuitBreaker::new(cfg);
        b.record_failure();
        b.record_failure();
        let later = Instant::now() + Duration::from_millis(20);
        assert!(b.can_proceed_at(later));
        b.record_failure_at(later);
        assert_eq!(b.state_at(later), BreakerState::Open);
        let _ = tenant_id;
    }

    /// cite: resilience4j — HalfOpen permits configurable trial count
    #[test]
    fn breaker_acme_halfopen_permits_n_trials() {
        let tenant_id = "acme";
        let cfg = count_cfg(0.5, 2, 10).with_half_open_permitted(3);
        let b = CircuitBreaker::new(cfg);
        b.record_failure();
        b.record_failure();
        let later = Instant::now() + Duration::from_millis(20);
        assert!(b.can_proceed_at(later));
        assert!(b.can_proceed_at(later));
        assert!(b.can_proceed_at(later));
        assert!(!b.can_proceed_at(later), "tenant {tenant_id} 4th rejected");
    }

    /// cite: resilience4j — Closed call to record_failure does not crash on Open call
    #[test]
    fn breaker_initech_open_record_after_trip_is_safe_noop() {
        let tenant_id = "initech";
        let cfg = count_cfg(0.5, 2, 10_000);
        let b = CircuitBreaker::new(cfg);
        b.record_failure();
        b.record_failure();
        assert_eq!(b.state(), BreakerState::Open);
        b.record_failure();
        b.record_success();
        assert_eq!(b.state(), BreakerState::Open);
        let _ = tenant_id;
    }

    /// cite: resilience4j — count window evicts oldest sample beyond capacity
    #[test]
    fn breaker_acme_count_window_evicts_oldest() {
        let tenant_id = "acme";
        let cfg = BreakerConfig::new(WindowKind::Count(4), 0.75, 4, Duration::from_secs(1));
        let b = CircuitBreaker::new(cfg);
        // 4 successes — fills window
        for _ in 0..4 {
            b.record_success();
        }
        assert_eq!(b.state(), BreakerState::Closed);
        // 3 failures push successes out (window: 1 success + 3 failures = 0.75 rate)
        for _ in 0..3 {
            b.record_failure();
        }
        assert_eq!(b.state(), BreakerState::Open, "tenant {tenant_id}");
    }

    /// cite: resilience4j — time window drops samples older than window
    #[test]
    fn breaker_globex_time_window_drops_old_samples() {
        let tenant_id = "globex";
        // High threshold + min_calls so 2 early failures don't trip the breaker.
        let cfg = BreakerConfig::new(
            WindowKind::Time(Duration::from_millis(100)),
            0.99,
            10,
            Duration::from_secs(1),
        );
        let b = CircuitBreaker::new(cfg);
        let t0 = Instant::now();
        b.record_failure_at(t0);
        b.record_failure_at(t0);
        // 200ms later, prior failures rolled off; now record successes
        let t1 = t0 + Duration::from_millis(200);
        b.record_success_at(t1);
        b.record_success_at(t1);
        assert_eq!(b.current_failure_rate(), 0.0);
        let _ = tenant_id;
    }

    /// cite: resilience4j — current_failure_rate reflects sample window
    #[test]
    fn breaker_acme_current_failure_rate_computed() {
        let tenant_id = "acme";
        let cfg = count_cfg(0.99, 4, 100);
        let b = CircuitBreaker::new(cfg);
        b.record_failure();
        b.record_failure();
        b.record_success();
        b.record_success();
        let r = b.current_failure_rate();
        assert!((r - 0.5).abs() < 1e-9);
        let _ = tenant_id;
    }

    /// cite: per-key — different keys have independent state
    #[test]
    fn breaker_perkey_acme_globex_independent() {
        let pk = PerKeyBreakers::new(count_cfg(0.5, 2, 100));
        let acme = pk.for_key("upstream:acme-db");
        let globex = pk.for_key("upstream:globex-cache");
        acme.record_failure();
        acme.record_failure();
        assert_eq!(acme.state(), BreakerState::Open);
        assert_eq!(globex.state(), BreakerState::Closed);
    }

    /// cite: per-key — same key returns shared instance
    #[test]
    fn breaker_perkey_acme_db_returns_shared_instance() {
        let pk = PerKeyBreakers::new(count_cfg(0.5, 2, 100));
        let a = pk.for_key("upstream:acme-db");
        a.record_failure();
        a.record_failure();
        let b = pk.for_key("upstream:acme-db");
        assert_eq!(b.state(), BreakerState::Open);
    }

    /// cite: per-key — known_keys reports observed keys sorted
    #[test]
    fn breaker_perkey_known_keys_sorted() {
        let pk = PerKeyBreakers::new(count_cfg(0.5, 2, 100));
        pk.for_key("z-key");
        pk.for_key("a-key");
        pk.for_key("m-key");
        assert_eq!(pk.known_keys(), vec!["a-key", "m-key", "z-key"]);
    }

    // ── Consecutive trip mode (Sweep-005 unblock) ────────────────────────────

    /// cite: Envoy outlier-detection — N consecutive failures opens the breaker
    #[test]
    fn breaker_consecutive_opens_after_n_failures() {
        let b = CircuitBreaker::new(BreakerConfig::consecutive(3, 2, Duration::from_millis(100)));
        assert_eq!(b.state(), BreakerState::Closed);
        b.record_failure();
        b.record_failure();
        assert_eq!(
            b.state(),
            BreakerState::Closed,
            "2 failures < 3 stays closed"
        );
        b.record_failure();
        assert_eq!(b.state(), BreakerState::Open, "3 consecutive failures trip");
    }

    /// cite: Envoy outlier-detection — a single success resets the streak
    #[test]
    fn breaker_consecutive_success_resets_streak() {
        let b = CircuitBreaker::new(BreakerConfig::consecutive(3, 2, Duration::from_millis(100)));
        b.record_failure();
        b.record_failure();
        b.record_success(); // resets streak
        b.record_failure();
        b.record_failure();
        assert_eq!(
            b.state(),
            BreakerState::Closed,
            "streak reset by success; only 2 trailing failures"
        );
        b.record_failure();
        assert_eq!(b.state(), BreakerState::Open, "now 3 trailing");
    }

    /// cite: Envoy outlier-detection — Open → HalfOpen → Closed via
    /// `success_count` trial successes
    #[test]
    fn breaker_consecutive_halfopen_closes_after_n_successes() {
        let b = CircuitBreaker::new(BreakerConfig::consecutive(2, 2, Duration::from_millis(10)));
        b.record_failure();
        b.record_failure();
        assert_eq!(b.state(), BreakerState::Open);
        let later = Instant::now() + Duration::from_millis(20);
        assert!(b.can_proceed_at(later), "reset_timeout permits first trial");
        b.record_success_at(later);
        // success_count=2 means 2 consecutive HalfOpen successes close
        assert!(b.can_proceed_at(later), "second trial slot");
        b.record_success_at(later);
        assert_eq!(b.state_at(later), BreakerState::Closed);
    }

    /// cite: legacy field accessors keep the older callers compiling
    #[test]
    fn breaker_legacy_accessors_match_trip_mode() {
        let windowed = count_cfg(0.5, 4, 100);
        assert_eq!(windowed.failure_rate_threshold(), 0.5);
        assert_eq!(windowed.minimum_calls(), 4);
        let consec = BreakerConfig::consecutive(5, 2, Duration::from_millis(100));
        assert_eq!(consec.failure_rate_threshold(), 1.0);
        assert_eq!(consec.minimum_calls(), 5);
    }
}
