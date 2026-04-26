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

#[derive(Debug, Clone)]
pub struct BreakerConfig {
    pub window: WindowKind,
    pub failure_rate_threshold: f64, // e.g. 0.5 for 50%
    pub minimum_calls: usize,
    pub reset_timeout: Duration,
    pub half_open_permitted: usize,
}

impl BreakerConfig {
    pub fn new(window: WindowKind, threshold: f64, minimum_calls: usize, reset: Duration) -> Self {
        assert!((0.0..=1.0).contains(&threshold), "threshold ∈ [0,1]");
        assert!(minimum_calls > 0, "minimum_calls must be > 0");
        Self {
            window,
            failure_rate_threshold: threshold,
            minimum_calls,
            reset_timeout: reset,
            half_open_permitted: 1,
        }
    }

    pub fn with_half_open_permitted(mut self, n: usize) -> Self {
        assert!(n > 0, "half_open_permitted must be > 0");
        self.half_open_permitted = n;
        self
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

        // Evaluate trip
        let total = inner.samples.len();
        if total < inner.cfg.minimum_calls {
            return;
        }
        let failures = inner
            .samples
            .iter()
            .filter(|s| s.outcome == Outcome::Failure)
            .count();
        let rate = failures as f64 / total as f64;
        if rate >= inner.cfg.failure_rate_threshold {
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
}
