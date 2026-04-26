//! Token bucket + leaky bucket rate limiters with per-tenant scoping.
//!
//! Upstream cite: `governor` (Rust crate, MIT) for bucket pattern reference;
//! `@aws-sdk/util-retry` for client-side throttling semantics. Reimpl in pure
//! Rust, no `governor` dependency, integrated with cave metrics emission.
//!
//! - `TokenBucket`: fills at `refill_per_sec`, capped at `capacity`. Allows bursts.
//! - `LeakyBucket`: queue capped at `queue_size`, drains at `leak_per_sec`. Smooths.
//! - `PerTenant<B>`: lazy-creates a bucket per `tenant_id` from a factory.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ── Token bucket ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TokenBucketConfig {
    pub capacity: f64,
    pub refill_per_sec: f64,
}

impl TokenBucketConfig {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        assert!(capacity > 0.0, "capacity must be > 0");
        assert!(refill_per_sec > 0.0, "refill_per_sec must be > 0");
        Self {
            capacity,
            refill_per_sec,
        }
    }
}

#[derive(Debug)]
struct TokenBucketState {
    tokens: f64,
    last_refill: Instant,
}

#[derive(Debug, Clone)]
pub struct TokenBucket {
    cfg: TokenBucketConfig,
    state: Arc<Mutex<TokenBucketState>>,
}

impl TokenBucket {
    pub fn new(cfg: TokenBucketConfig) -> Self {
        let state = TokenBucketState {
            tokens: cfg.capacity,
            last_refill: Instant::now(),
        };
        Self {
            cfg,
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn refill_locked(&self, st: &mut TokenBucketState, now: Instant) {
        let elapsed = now.saturating_duration_since(st.last_refill);
        let added = elapsed.as_secs_f64() * self.cfg.refill_per_sec;
        st.tokens = (st.tokens + added).min(self.cfg.capacity);
        st.last_refill = now;
    }

    /// Synchronous take: returns `true` iff `n` tokens were available.
    pub fn try_consume(&self, n: f64) -> bool {
        self.try_consume_at(n, Instant::now())
    }

    pub fn try_consume_at(&self, n: f64, now: Instant) -> bool {
        if n <= 0.0 {
            return true;
        }
        let mut st = self.state.lock();
        self.refill_locked(&mut st, now);
        if st.tokens >= n {
            st.tokens -= n;
            true
        } else {
            false
        }
    }

    /// Available tokens at the given instant (after virtual refill).
    pub fn available_at(&self, now: Instant) -> f64 {
        let mut st = self.state.lock();
        self.refill_locked(&mut st, now);
        st.tokens
    }

    pub fn available(&self) -> f64 {
        self.available_at(Instant::now())
    }

    /// Async wait until `n` tokens are available, then consume them.
    /// Uses tokio sleep with adaptive sleep durations.
    pub async fn acquire(&self, n: f64) {
        loop {
            let needed = {
                let mut st = self.state.lock();
                self.refill_locked(&mut st, Instant::now());
                if st.tokens >= n {
                    st.tokens -= n;
                    return;
                }
                (n - st.tokens) / self.cfg.refill_per_sec
            };
            let dur = Duration::from_secs_f64(needed.max(0.001).min(1.0));
            tokio::time::sleep(dur).await;
        }
    }
}

// ── Leaky bucket ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LeakyBucketConfig {
    pub queue_size: usize,
    pub leak_per_sec: f64,
}

impl LeakyBucketConfig {
    pub fn new(queue_size: usize, leak_per_sec: f64) -> Self {
        assert!(queue_size > 0, "queue_size must be > 0");
        assert!(leak_per_sec > 0.0, "leak_per_sec must be > 0");
        Self {
            queue_size,
            leak_per_sec,
        }
    }
}

#[derive(Debug)]
struct LeakyBucketState {
    level: f64,
    last_leak: Instant,
}

#[derive(Debug, Clone)]
pub struct LeakyBucket {
    cfg: LeakyBucketConfig,
    state: Arc<Mutex<LeakyBucketState>>,
}

impl LeakyBucket {
    pub fn new(cfg: LeakyBucketConfig) -> Self {
        let state = LeakyBucketState {
            level: 0.0,
            last_leak: Instant::now(),
        };
        Self {
            cfg,
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn drain_locked(&self, st: &mut LeakyBucketState, now: Instant) {
        let elapsed = now.saturating_duration_since(st.last_leak).as_secs_f64();
        st.level = (st.level - elapsed * self.cfg.leak_per_sec).max(0.0);
        st.last_leak = now;
    }

    pub fn try_offer(&self) -> bool {
        self.try_offer_at(Instant::now())
    }

    pub fn try_offer_at(&self, now: Instant) -> bool {
        let mut st = self.state.lock();
        self.drain_locked(&mut st, now);
        if st.level + 1.0 <= self.cfg.queue_size as f64 + f64::EPSILON {
            st.level += 1.0;
            true
        } else {
            false
        }
    }

    pub fn level_at(&self, now: Instant) -> f64 {
        let mut st = self.state.lock();
        self.drain_locked(&mut st, now);
        st.level
    }

    pub fn level(&self) -> f64 {
        self.level_at(Instant::now())
    }
}

// ── Per-tenant scoping ────────────────────────────────────────────────────────

pub trait BucketFactory<B>: Send + Sync {
    fn make(&self, tenant_id: &str) -> B;
}

pub struct StaticTokenFactory(pub TokenBucketConfig);

impl BucketFactory<TokenBucket> for StaticTokenFactory {
    fn make(&self, _tenant_id: &str) -> TokenBucket {
        TokenBucket::new(self.0.clone())
    }
}

pub struct StaticLeakyFactory(pub LeakyBucketConfig);

impl BucketFactory<LeakyBucket> for StaticLeakyFactory {
    fn make(&self, _tenant_id: &str) -> LeakyBucket {
        LeakyBucket::new(self.0.clone())
    }
}

pub struct PerTenant<B> {
    factory: Arc<dyn BucketFactory<B>>,
    map: Mutex<HashMap<String, B>>,
}

impl<B: Clone> PerTenant<B> {
    pub fn new<F: BucketFactory<B> + 'static>(factory: F) -> Self {
        Self {
            factory: Arc::new(factory),
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn for_tenant(&self, tenant_id: &str) -> B {
        let mut m = self.map.lock();
        m.entry(tenant_id.to_string())
            .or_insert_with(|| self.factory.make(tenant_id))
            .clone()
    }

    pub fn known_tenants(&self) -> Vec<String> {
        let m = self.map.lock();
        let mut v: Vec<String> = m.keys().cloned().collect();
        v.sort();
        v
    }
}

// ── Metrics snapshot ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct RateLimiterMetrics {
    pub tenant_id: String,
    pub kind: &'static str,
    pub available_or_level: f64,
    pub capacity_or_queue: f64,
}

impl RateLimiterMetrics {
    pub fn from_token(tenant_id: &str, b: &TokenBucket) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            kind: "token_bucket",
            available_or_level: b.available(),
            capacity_or_queue: b.cfg.capacity,
        }
    }

    pub fn from_leaky(tenant_id: &str, b: &LeakyBucket) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            kind: "leaky_bucket",
            available_or_level: b.level(),
            capacity_or_queue: b.cfg.queue_size as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cite: governor pattern — fresh token bucket starts full at capacity
    #[test]
    fn ratelimiter_acme_token_bucket_starts_full() {
        let tenant_id = "acme";
        let b = TokenBucket::new(TokenBucketConfig::new(10.0, 1.0));
        let avail = b.available();
        assert!(avail >= 9.999 && avail <= 10.001);
        let _ = tenant_id;
    }

    /// cite: governor pattern — try_consume succeeds while tokens remain
    #[test]
    fn ratelimiter_acme_token_consume_within_capacity() {
        let tenant_id = "acme";
        let b = TokenBucket::new(TokenBucketConfig::new(5.0, 1.0));
        for _ in 0..5 {
            assert!(b.try_consume(1.0), "tenant {tenant_id} should have tokens");
        }
        assert!(!b.try_consume(1.0), "tenant {tenant_id} now empty");
    }

    /// cite: governor pattern — refill restores tokens after virtual time
    #[test]
    fn ratelimiter_globex_token_refill_after_elapsed() {
        let tenant_id = "globex";
        let b = TokenBucket::new(TokenBucketConfig::new(10.0, 5.0));
        // drain
        for _ in 0..10 {
            b.try_consume(1.0);
        }
        let _ = tenant_id;
        // jump 1 second forward
        let later = Instant::now() + Duration::from_secs(1);
        let avail = b.available_at(later);
        assert!(avail >= 4.99 && avail <= 5.01, "got {avail}");
    }

    /// cite: governor pattern — refill caps at capacity (no overfill)
    #[test]
    fn ratelimiter_initech_token_refill_caps_at_capacity() {
        let tenant_id = "initech";
        let b = TokenBucket::new(TokenBucketConfig::new(3.0, 100.0));
        let way_later = Instant::now() + Duration::from_secs(60);
        assert!(b.available_at(way_later) <= 3.001);
        let _ = tenant_id;
    }

    /// cite: governor pattern — try_consume(0) is a no-op success
    #[test]
    fn ratelimiter_dunder_token_consume_zero_is_noop() {
        let tenant_id = "dunder";
        let b = TokenBucket::new(TokenBucketConfig::new(1.0, 1.0));
        assert!(b.try_consume(0.0));
        assert!(b.available() > 0.99);
        let _ = tenant_id;
    }

    /// cite: governor pattern — burst (consume = capacity) succeeds at start
    #[test]
    fn ratelimiter_acme_token_burst_succeeds() {
        let tenant_id = "acme";
        let b = TokenBucket::new(TokenBucketConfig::new(50.0, 1.0));
        assert!(b.try_consume(50.0), "tenant {tenant_id} burst");
        assert!(!b.try_consume(0.001));
    }

    /// cite: governor pattern — async acquire returns once tokens available
    #[tokio::test]
    async fn ratelimiter_acme_token_acquire_yields_after_refill() {
        let tenant_id = "acme";
        // refill 1000/sec → 1 token in ≈1ms
        let b = TokenBucket::new(TokenBucketConfig::new(1.0, 1000.0));
        b.try_consume(1.0); // drain
        let _ = tenant_id;
        let start = std::time::Instant::now();
        b.acquire(1.0).await;
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 200, "should refill quickly: {elapsed:?}");
    }

    /// cite: leaky bucket pattern — fresh queue is empty, accepts up to queue_size
    #[test]
    fn ratelimiter_acme_leaky_starts_empty_accepts_queue() {
        let tenant_id = "acme";
        let b = LeakyBucket::new(LeakyBucketConfig::new(3, 1.0));
        assert!(b.try_offer());
        assert!(b.try_offer());
        assert!(b.try_offer());
        assert!(!b.try_offer(), "tenant {tenant_id} queue full");
    }

    /// cite: leaky bucket pattern — drains at leak_per_sec
    #[test]
    fn ratelimiter_globex_leaky_drains_after_elapsed() {
        let tenant_id = "globex";
        let b = LeakyBucket::new(LeakyBucketConfig::new(5, 2.0));
        for _ in 0..5 {
            b.try_offer();
        }
        let later = Instant::now() + Duration::from_secs(1);
        let lvl = b.level_at(later);
        assert!(lvl >= 2.99 && lvl <= 3.01, "got {lvl}");
        let _ = tenant_id;
    }

    /// cite: leaky bucket pattern — drain floor is 0 (no negative level)
    #[test]
    fn ratelimiter_initech_leaky_drain_floors_at_zero() {
        let tenant_id = "initech";
        let b = LeakyBucket::new(LeakyBucketConfig::new(5, 100.0));
        b.try_offer();
        let way_later = Instant::now() + Duration::from_secs(60);
        assert_eq!(b.level_at(way_later), 0.0);
        let _ = tenant_id;
    }

    /// cite: per-tenant scoping — independent buckets per tenant_id
    #[test]
    fn ratelimiter_per_tenant_acme_globex_independent() {
        let pt = PerTenant::new(StaticTokenFactory(TokenBucketConfig::new(2.0, 1.0)));
        let acme = pt.for_tenant("acme");
        let globex = pt.for_tenant("globex");
        for _ in 0..2 {
            acme.try_consume(1.0);
        }
        // acme drained, globex untouched
        assert!(!acme.try_consume(1.0));
        assert!(globex.try_consume(1.0));
    }

    /// cite: per-tenant scoping — same tenant returns same bucket (sticky)
    #[test]
    fn ratelimiter_per_tenant_acme_returns_same_bucket() {
        let tenant_id = "acme";
        let pt = PerTenant::new(StaticTokenFactory(TokenBucketConfig::new(5.0, 1.0)));
        let a = pt.for_tenant(tenant_id);
        a.try_consume(3.0);
        let b = pt.for_tenant(tenant_id);
        let avail = b.available();
        assert!(avail >= 1.99 && avail <= 2.01, "got {avail}");
    }

    /// cite: per-tenant scoping — known_tenants reports observed ids
    #[test]
    fn ratelimiter_per_tenant_known_tenants_lists_ids() {
        let pt = PerTenant::new(StaticTokenFactory(TokenBucketConfig::new(1.0, 1.0)));
        pt.for_tenant("acme");
        pt.for_tenant("globex");
        pt.for_tenant("initech");
        assert_eq!(
            pt.known_tenants(),
            vec!["acme".to_string(), "globex".to_string(), "initech".to_string()]
        );
    }

    /// cite: metrics — token bucket snapshot includes capacity + tenant_id
    #[test]
    fn ratelimiter_metrics_acme_token_snapshot() {
        let tenant_id = "acme";
        let b = TokenBucket::new(TokenBucketConfig::new(7.0, 1.0));
        let m = RateLimiterMetrics::from_token(tenant_id, &b);
        assert_eq!(m.tenant_id, tenant_id);
        assert_eq!(m.kind, "token_bucket");
        assert_eq!(m.capacity_or_queue, 7.0);
    }

    /// cite: metrics — leaky bucket snapshot reports current queue level
    #[test]
    fn ratelimiter_metrics_globex_leaky_snapshot() {
        let tenant_id = "globex";
        let b = LeakyBucket::new(LeakyBucketConfig::new(4, 1.0));
        b.try_offer();
        b.try_offer();
        let m = RateLimiterMetrics::from_leaky(tenant_id, &b);
        assert_eq!(m.tenant_id, tenant_id);
        assert_eq!(m.kind, "leaky_bucket");
        assert_eq!(m.capacity_or_queue, 4.0);
        assert!(m.available_or_level >= 1.99 && m.available_or_level <= 2.01);
    }
}
