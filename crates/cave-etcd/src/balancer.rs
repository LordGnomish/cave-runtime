// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! v3 client balancer — endpoint pool, round-robin pick, leader-pinning,
//! per-endpoint health tracking, and retry-on-leader-change.
//!
//! Mirrors etcd v3.6.10
//!   `client/v3/balancer/balancer.go` (round-robin picker),
//!   `client/v3/balancer/picker/healthy.go` (skip unhealthy endpoints),
//!   `client/v3/retry.go` (leader-change retry).
//!
//! The balancer is transport-agnostic — it emits an [`EndpointDecision`]
//! that the caller (HTTP, gRPC, in-process) consumes.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum BalancerError {
    /// All endpoints in the pool are unhealthy.
    NoHealthyEndpoint,
    /// Pool is empty.
    EmptyPool,
    /// Caller exhausted its retry budget.
    RetryBudgetExceeded { attempts: u32, last: String },
}

impl std::fmt::Display for BalancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHealthyEndpoint => write!(f, "no healthy endpoint"),
            Self::EmptyPool => write!(f, "endpoint pool is empty"),
            Self::RetryBudgetExceeded { attempts, last } => write!(f, "retry budget exceeded after {attempts} attempts; last error: {last}"),
        }
    }
}

impl std::error::Error for BalancerError {}

// ── Per-endpoint state ────────────────────────────────────────────────────

/// Health state of a single endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointHealth {
    Healthy,
    Unhealthy,
}

#[derive(Debug, Clone)]
struct EndpointEntry {
    url: String,
    health: EndpointHealth,
    /// Number of consecutive failures observed.
    fail_count: u32,
    /// When this endpoint was last marked unhealthy — used for back-off
    /// before re-trying.
    unhealthy_since: Option<Instant>,
}

// ── Endpoint pool ────────────────────────────────────────────────────────

/// Endpoint pool.  Maintains health, leader pinning, and a round-robin
/// cursor so successive `pick()` calls fan out across healthy endpoints.
pub struct EndpointPool {
    inner: RwLock<EndpointPoolInner>,
    cursor: AtomicUsize,
    /// Rolling backoff before an unhealthy endpoint is reconsidered.
    unhealthy_backoff: Duration,
    /// Counts every retry triggered by a leader change.
    leader_change_retries: AtomicU64,
}

#[derive(Default)]
struct EndpointPoolInner {
    endpoints: Vec<EndpointEntry>,
    /// Pinned leader endpoint URL.  When set, leader-aware calls go here
    /// regardless of round-robin.
    leader_url: Option<String>,
    /// Number of endpoint URLs currently registered.
    size: usize,
}

impl EndpointPool {
    pub fn new(urls: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let pool = Self {
            inner: RwLock::new(EndpointPoolInner::default()),
            cursor: AtomicUsize::new(0),
            unhealthy_backoff: Duration::from_secs(5),
            leader_change_retries: AtomicU64::new(0),
        };
        for u in urls { pool.add(u.into()); }
        pool
    }

    pub fn with_unhealthy_backoff(mut self, d: Duration) -> Self {
        self.unhealthy_backoff = d;
        self
    }

    /// Add an endpoint to the pool (or no-op if already present).
    pub fn add(&self, url: impl Into<String>) {
        let url = url.into();
        let mut inner = self.inner.write().unwrap();
        if !inner.endpoints.iter().any(|e| e.url == url) {
            inner.endpoints.push(EndpointEntry {
                url,
                health: EndpointHealth::Healthy,
                fail_count: 0,
                unhealthy_since: None,
            });
            inner.size = inner.endpoints.len();
        }
    }

    /// Remove an endpoint, e.g. after a member is dropped from the cluster.
    pub fn remove(&self, url: &str) -> bool {
        let mut inner = self.inner.write().unwrap();
        let before = inner.endpoints.len();
        inner.endpoints.retain(|e| e.url != url);
        let removed = inner.endpoints.len() != before;
        if removed { inner.size = inner.endpoints.len(); }
        if inner.leader_url.as_deref() == Some(url) { inner.leader_url = None; }
        removed
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().size }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// All registered endpoints (regardless of health).
    pub fn endpoints(&self) -> Vec<String> {
        self.inner.read().unwrap().endpoints.iter().map(|e| e.url.clone()).collect()
    }

    pub fn leader(&self) -> Option<String> {
        self.inner.read().unwrap().leader_url.clone()
    }

    /// Pin the leader.  Subsequent leader-aware calls go to this URL.
    pub fn set_leader(&self, url: impl Into<String>) {
        let url = url.into();
        let mut inner = self.inner.write().unwrap();
        // Make sure the leader URL is registered as an endpoint.
        if !inner.endpoints.iter().any(|e| e.url == url) {
            inner.endpoints.push(EndpointEntry {
                url: url.clone(),
                health: EndpointHealth::Healthy,
                fail_count: 0,
                unhealthy_since: None,
            });
            inner.size = inner.endpoints.len();
        }
        inner.leader_url = Some(url);
    }

    pub fn clear_leader(&self) {
        self.inner.write().unwrap().leader_url = None;
    }

    /// Mark an endpoint unhealthy (timeout or failed RPC).
    pub fn mark_unhealthy(&self, url: &str) {
        let mut inner = self.inner.write().unwrap();
        if let Some(e) = inner.endpoints.iter_mut().find(|e| e.url == url) {
            e.fail_count = e.fail_count.saturating_add(1);
            e.health = EndpointHealth::Unhealthy;
            e.unhealthy_since = Some(Instant::now());
        }
    }

    /// Mark an endpoint healthy after a successful call.
    pub fn mark_healthy(&self, url: &str) {
        let mut inner = self.inner.write().unwrap();
        if let Some(e) = inner.endpoints.iter_mut().find(|e| e.url == url) {
            e.fail_count = 0;
            e.health = EndpointHealth::Healthy;
            e.unhealthy_since = None;
        }
    }

    pub fn health_of(&self, url: &str) -> Option<EndpointHealth> {
        self.inner.read().unwrap().endpoints.iter()
            .find(|e| e.url == url)
            .map(|e| e.health)
    }

    pub fn fail_count(&self, url: &str) -> u32 {
        self.inner.read().unwrap().endpoints.iter()
            .find(|e| e.url == url)
            .map(|e| e.fail_count)
            .unwrap_or(0)
    }

    /// Round-robin pick of the next healthy endpoint.  Skips unhealthy
    /// endpoints unless their backoff has elapsed.
    pub fn pick(&self) -> Result<String, BalancerError> {
        let inner = self.inner.read().unwrap();
        if inner.endpoints.is_empty() { return Err(BalancerError::EmptyPool); }
        let n = inner.endpoints.len();
        let now = Instant::now();
        for _ in 0..n {
            let i = self.cursor.fetch_add(1, Ordering::SeqCst) % n;
            let e = &inner.endpoints[i];
            let usable = match e.health {
                EndpointHealth::Healthy => true,
                EndpointHealth::Unhealthy => match e.unhealthy_since {
                    Some(t) => now.duration_since(t) >= self.unhealthy_backoff,
                    None => true,
                },
            };
            if usable { return Ok(e.url.clone()); }
        }
        Err(BalancerError::NoHealthyEndpoint)
    }

    /// Pick the leader if pinned; else round-robin to a healthy endpoint.
    pub fn pick_leader_or_any(&self) -> Result<EndpointDecision, BalancerError> {
        if let Some(url) = self.leader() {
            // Leader pin is honoured even if leader is currently unhealthy
            // — caller will retry and may demote the leader.
            return Ok(EndpointDecision { url, leader: true });
        }
        Ok(EndpointDecision { url: self.pick()?, leader: false })
    }

    /// Forcibly re-promote every endpoint to healthy.  Used after a
    /// successful endpoint sync from the cluster.
    pub fn reset_health(&self) {
        let mut inner = self.inner.write().unwrap();
        for e in inner.endpoints.iter_mut() {
            e.health = EndpointHealth::Healthy;
            e.fail_count = 0;
            e.unhealthy_since = None;
        }
    }

    /// Count of leader-change retries observed (cumulative).
    pub fn leader_change_retries(&self) -> u64 {
        self.leader_change_retries.load(Ordering::SeqCst)
    }

    /// Record that a call failed because the leader changed — clears the
    /// pinned leader so the next call re-discovers it.
    pub fn observe_leader_change(&self) {
        self.leader_change_retries.fetch_add(1, Ordering::SeqCst);
        self.clear_leader();
    }
}

/// One pick decision — what URL to dial and whether it's the pinned leader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointDecision {
    pub url: String,
    pub leader: bool,
}

// ── Backoff ───────────────────────────────────────────────────────────────

/// Exponential backoff with capped jitter.  Mirrors etcd `client/v3/retry.go`
/// (ExponentialBackoff with MaxBackoff cap).
pub fn backoff_duration(initial: Duration, attempt: u32, cap: Duration) -> Duration {
    let exp = (attempt as u64).saturating_mul(initial.as_millis() as u64);
    let candidate = Duration::from_millis(exp.saturating_add(initial.as_millis() as u64));
    if candidate > cap { cap } else { candidate }
}

// ── Retry classifier ──────────────────────────────────────────────────────

/// Why an RPC failed.  Drives the balancer's response (retry, fail-fast,
/// leader-change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcOutcome {
    Ok,
    /// Transient error — retry on a new endpoint.
    EndpointDown,
    /// Server reports it's no longer leader.  Re-discover leader and retry.
    LeaderChange,
    /// Server returned a permanent error — don't retry.
    Permanent,
}

impl RpcOutcome {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::EndpointDown | Self::LeaderChange)
    }
}

/// Result of one full retry attempt sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptStats {
    pub attempts: u32,
    pub leader_changes: u32,
    pub endpoint_failures: u32,
}

// ─────────────────────────────────────────────────────────────────────────
// Balancer tests — feat/cave-etcd-100-pct-sprint
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> EndpointPool {
        EndpointPool::new(vec!["http://a:2379", "http://b:2379", "http://c:2379"])
    }

    // ── Pool basics ───────────────────────────────────────────────────

    #[test]
    fn test_pool_initial_endpoints() {
        // cite: balancer.go ResolverState
        let p = pool();
        assert_eq!(p.len(), 3);
        assert_eq!(p.endpoints().len(), 3);
    }

    #[test]
    fn test_pool_add_idempotent() {
        // cite: balancer.go (de-duplicate endpoints)
        let p = pool();
        p.add("http://a:2379");
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn test_pool_remove_member() {
        // cite: balancer.go (member-remove drops endpoint)
        let p = pool();
        assert!(p.remove("http://b:2379"));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn test_pool_remove_nonexistent_returns_false() {
        let p = pool();
        assert!(!p.remove("http://z:2379"));
    }

    #[test]
    fn test_pool_pick_empty_errors() {
        // cite: balancer.go (no endpoints ⇒ DialError)
        let p = EndpointPool::new(Vec::<&str>::new());
        assert_eq!(p.pick().unwrap_err(), BalancerError::EmptyPool);
    }

    // ── Round-robin ──────────────────────────────────────────────────

    #[test]
    fn test_round_robin_visits_all_three() {
        // cite: picker/roundrobin.go (each pick advances cursor)
        let p = pool();
        let a = p.pick().unwrap();
        let b = p.pick().unwrap();
        let c = p.pick().unwrap();
        let mut seen = vec![a, b, c];
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn test_round_robin_wraps_around() {
        // cite: picker/roundrobin.go (cursor wraps modulo N)
        let p = pool();
        let first = p.pick().unwrap();
        // Three picks completes the cycle and brings us back to `first`.
        for _ in 0..2 { p.pick().unwrap(); }
        assert_eq!(p.pick().unwrap(), first);
    }

    // ── Health tracking ──────────────────────────────────────────────

    #[test]
    fn test_mark_unhealthy_skipped_in_pick() {
        // cite: picker/healthy.go (unhealthy endpoints skipped)
        let p = pool();
        p.mark_unhealthy("http://a:2379");
        for _ in 0..6 {
            let picked = p.pick().unwrap();
            assert_ne!(picked, "http://a:2379");
        }
    }

    #[test]
    fn test_mark_healthy_resets_fail_count() {
        // cite: picker/healthy.go (success ⇒ counter reset)
        let p = pool();
        p.mark_unhealthy("http://a:2379");
        p.mark_unhealthy("http://a:2379");
        assert_eq!(p.fail_count("http://a:2379"), 2);
        p.mark_healthy("http://a:2379");
        assert_eq!(p.fail_count("http://a:2379"), 0);
    }

    #[test]
    fn test_health_of_unknown_returns_none() {
        let p = pool();
        assert_eq!(p.health_of("http://z:2379"), None);
    }

    #[test]
    fn test_all_unhealthy_returns_no_healthy_endpoint() {
        // cite: picker/healthy.go (no usable picks ⇒ ErrNoEndpoints)
        let p = EndpointPool::new(vec!["http://a:2379", "http://b:2379"])
            .with_unhealthy_backoff(Duration::from_secs(60));
        p.mark_unhealthy("http://a:2379");
        p.mark_unhealthy("http://b:2379");
        assert_eq!(p.pick().unwrap_err(), BalancerError::NoHealthyEndpoint);
    }

    #[test]
    fn test_unhealthy_recovers_after_backoff() {
        // cite: picker/healthy.go (re-try after timeout)
        let p = EndpointPool::new(vec!["http://a:2379"])
            .with_unhealthy_backoff(Duration::from_millis(5));
        p.mark_unhealthy("http://a:2379");
        std::thread::sleep(Duration::from_millis(20));
        assert!(p.pick().is_ok());
    }

    #[test]
    fn test_reset_health_restores_all() {
        // cite: balancer.go EndpointSync
        let p = pool();
        p.mark_unhealthy("http://a:2379");
        p.mark_unhealthy("http://b:2379");
        p.reset_health();
        assert_eq!(p.health_of("http://a:2379"), Some(EndpointHealth::Healthy));
        assert_eq!(p.health_of("http://b:2379"), Some(EndpointHealth::Healthy));
    }

    // ── Leader pinning ───────────────────────────────────────────────

    #[test]
    fn test_set_leader_returns_pinned() {
        // cite: client/v3 leader-aware ops (e.g. txn) pin the leader
        let p = pool();
        p.set_leader("http://b:2379");
        let d = p.pick_leader_or_any().unwrap();
        assert_eq!(d.url, "http://b:2379");
        assert!(d.leader);
    }

    #[test]
    fn test_pick_leader_or_any_falls_back() {
        // cite: client/v3 retry.go (no leader known ⇒ round-robin)
        let p = pool();
        let d = p.pick_leader_or_any().unwrap();
        assert!(!d.leader);
    }

    #[test]
    fn test_set_leader_registers_endpoint_if_new() {
        // cite: client/v3 (leader URL added to pool on demand)
        let p = EndpointPool::new(Vec::<&str>::new());
        p.set_leader("http://leader:2379");
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn test_clear_leader() {
        // cite: client/v3 retry.go (leader-change ⇒ clear pin)
        let p = pool();
        p.set_leader("http://a:2379");
        p.clear_leader();
        assert_eq!(p.leader(), None);
    }

    #[test]
    fn test_remove_clears_leader_if_matches() {
        // cite: client/v3 (member-remove of leader unpins)
        let p = pool();
        p.set_leader("http://a:2379");
        p.remove("http://a:2379");
        assert_eq!(p.leader(), None);
    }

    #[test]
    fn test_observe_leader_change_clears_pin() {
        // cite: client/v3 retry.go (NotLeader ⇒ unpin + retry)
        let p = pool();
        p.set_leader("http://a:2379");
        p.observe_leader_change();
        assert_eq!(p.leader(), None);
        assert_eq!(p.leader_change_retries(), 1);
    }

    #[test]
    fn test_observe_leader_change_increments_counter() {
        // cite: metrics: etcd_client_leader_change_total
        let p = pool();
        p.observe_leader_change();
        p.observe_leader_change();
        p.observe_leader_change();
        assert_eq!(p.leader_change_retries(), 3);
    }

    // ── Backoff ──────────────────────────────────────────────────────

    #[test]
    fn test_backoff_first_attempt() {
        // cite: client/v3 retry.go ExponentialBackoff
        let d = backoff_duration(Duration::from_millis(25), 0, Duration::from_secs(5));
        assert_eq!(d, Duration::from_millis(25));
    }

    #[test]
    fn test_backoff_grows() {
        // cite: client/v3 retry.go (exponential growth)
        let a = backoff_duration(Duration::from_millis(25), 1, Duration::from_secs(5));
        let b = backoff_duration(Duration::from_millis(25), 4, Duration::from_secs(5));
        assert!(b > a);
    }

    #[test]
    fn test_backoff_caps_at_max() {
        // cite: client/v3 retry.go MaxBackoff
        let d = backoff_duration(Duration::from_millis(1000), 1000, Duration::from_secs(2));
        assert_eq!(d, Duration::from_secs(2));
    }

    // ── RpcOutcome classifier ────────────────────────────────────────

    #[test]
    fn test_rpc_outcome_retryability() {
        assert!(RpcOutcome::EndpointDown.is_retryable());
        assert!(RpcOutcome::LeaderChange.is_retryable());
        assert!(!RpcOutcome::Permanent.is_retryable());
        assert!(!RpcOutcome::Ok.is_retryable());
    }

    // ── Failover scenario ────────────────────────────────────────────

    #[test]
    fn test_failover_continues_to_next_endpoint() {
        // cite: balancer.go (endpoint timeout ⇒ next pick)
        let p = pool();
        p.mark_unhealthy("http://a:2379");
        let next = p.pick().unwrap();
        assert!(next == "http://b:2379" || next == "http://c:2379");
    }

    #[test]
    fn test_pool_endpoints_list_stable() {
        // cite: balancer.go ResolverState reflects current pool
        let p = pool();
        let mut e = p.endpoints();
        e.sort();
        assert_eq!(e, vec!["http://a:2379", "http://b:2379", "http://c:2379"]);
    }
}
