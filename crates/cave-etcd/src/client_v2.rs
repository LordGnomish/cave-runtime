// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! v3 client refinements — auth-token interceptor, retry classifier with
//! gRPC codes, lease keep-alive heartbeat scheduler, and watch reconnect
//! state machine.
//!
//! Mirrors etcd v3.6.10
//!   `client/v3/auth.go` (UnaryInterceptor injects token),
//!   `client/v3/retry_interceptor.go` (gRPC code → retry decision),
//!   `client/v3/lease.go` (LeaseKeepAlive heartbeat loop),
//!   `client/v3/watch.go` (Watch reconnect with progress notify).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

// ── Auth-token interceptor ────────────────────────────────────────────────

/// gRPC unary interceptor for auth.  Maintains the active token and the
/// per-call header injection; on Unauthenticated reauthenticates lazily.
pub struct AuthInterceptor {
    token: RwLock<Option<String>>,
    header_name: String,
    /// Counts successful header-injection passes.
    inject_count: AtomicU64,
    /// Counts re-auth attempts.
    reauth_count: AtomicU64,
}

impl AuthInterceptor {
    pub fn new() -> Self {
        Self {
            token: RwLock::new(None),
            header_name: "token".to_string(),
            inject_count: AtomicU64::new(0),
            reauth_count: AtomicU64::new(0),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>) -> Self {
        self.header_name = name.into();
        self
    }

    pub fn header_name(&self) -> &str {
        &self.header_name
    }
    pub fn token(&self) -> Option<String> {
        self.token.read().unwrap().clone()
    }
    pub fn set_token(&self, t: impl Into<String>) {
        *self.token.write().unwrap() = Some(t.into());
    }
    pub fn clear_token(&self) {
        *self.token.write().unwrap() = None;
    }

    pub fn inject_count(&self) -> u64 {
        self.inject_count.load(Ordering::SeqCst)
    }
    pub fn reauth_count(&self) -> u64 {
        self.reauth_count.load(Ordering::SeqCst)
    }

    /// Inject the active token into the supplied metadata bucket.
    /// Returns false if no token is available (caller should authenticate).
    pub fn inject(&self, metadata: &mut BTreeMap<String, String>) -> bool {
        match self.token() {
            Some(t) => {
                metadata.insert(self.header_name.clone(), t);
                self.inject_count.fetch_add(1, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Mark the active token as stale and prepare for re-authentication.
    /// Returns the previous token (or None).
    pub fn invalidate(&self) -> Option<String> {
        self.reauth_count.fetch_add(1, Ordering::SeqCst);
        self.token.write().unwrap().take()
    }
}

impl Default for AuthInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Retry classifier ─────────────────────────────────────────────────────

/// Subset of gRPC status codes the v3 client cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcCode {
    Ok,
    Cancelled,
    Unavailable,
    DeadlineExceeded,
    ResourceExhausted,
    Unauthenticated,
    PermissionDenied,
    InvalidArgument,
    NotFound,
    AlreadyExists,
    Internal,
    /// Etcd-specific "no leader" — translates from gRPC custom code.
    NoLeader,
    LeaderChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Retry on the same endpoint after `backoff`.
    RetryHere,
    /// Retry on a different endpoint.
    RetryElsewhere,
    /// Re-authenticate (`AuthInterceptor::invalidate`) and retry.
    Reauth,
    /// Permanent failure.
    Fail,
}

/// Classify a gRPC code into a retry decision.  Mirrors
/// `client/v3/retry_interceptor.go#isSafeRetryImmutableRPC`.
pub fn classify_retry(code: RpcCode, attempt: u32, max_retries: u32) -> RetryDecision {
    if attempt >= max_retries {
        return RetryDecision::Fail;
    }
    match code {
        RpcCode::Ok => RetryDecision::Fail, // never called when Ok
        RpcCode::Unauthenticated => RetryDecision::Reauth,
        RpcCode::Unavailable | RpcCode::DeadlineExceeded => RetryDecision::RetryElsewhere,
        RpcCode::NoLeader | RpcCode::LeaderChanged => RetryDecision::RetryElsewhere,
        RpcCode::ResourceExhausted => RetryDecision::RetryHere,
        RpcCode::Internal => RetryDecision::RetryElsewhere,
        RpcCode::Cancelled
        | RpcCode::PermissionDenied
        | RpcCode::InvalidArgument
        | RpcCode::NotFound
        | RpcCode::AlreadyExists => RetryDecision::Fail,
    }
}

// ── Lease keep-alive scheduler ───────────────────────────────────────────

/// One lease registered with the keep-alive scheduler.
#[derive(Debug, Clone)]
pub struct LeaseHandle {
    pub lease_id: i64,
    pub ttl_secs: i64,
    /// When the next heartbeat must fire (deadline).
    pub next_heartbeat_at: Instant,
    /// Heartbeats observed since registration.
    pub heartbeats: u64,
}

/// Scheduler that picks the next lease whose heartbeat is due.  Test-
/// driven (no async runtime needed); production wraps it in a `tokio::spawn`.
pub struct KeepAliveScheduler {
    /// `lease_id → handle`.  Kept ordered by lease_id; the deadline scan
    /// is O(n).  Etcd's actual scheduler uses a min-heap; for the test
    /// surface this is plenty.
    inner: Mutex<BTreeMap<i64, LeaseHandle>>,
    /// Default fraction of TTL that the heartbeat fires at.  etcd's
    /// `client/v3/lease.go` uses TTL/3.
    pulse_fraction: u32,
    pulses_fired: AtomicU64,
}

impl KeepAliveScheduler {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BTreeMap::new()),
            pulse_fraction: 3,
            pulses_fired: AtomicU64::new(0),
        }
    }

    pub fn with_pulse_fraction(mut self, n: u32) -> Self {
        self.pulse_fraction = n.max(1);
        self
    }

    pub fn pulse_fraction(&self) -> u32 {
        self.pulse_fraction
    }
    pub fn pulses_fired(&self) -> u64 {
        self.pulses_fired.load(Ordering::SeqCst)
    }

    pub fn register(&self, lease_id: i64, ttl_secs: i64) {
        let mut g = self.inner.lock().unwrap();
        let next = Instant::now()
            + Duration::from_secs((ttl_secs as u64).max(1) / self.pulse_fraction as u64);
        g.insert(
            lease_id,
            LeaseHandle {
                lease_id,
                ttl_secs,
                next_heartbeat_at: next,
                heartbeats: 0,
            },
        );
    }

    pub fn deregister(&self, lease_id: i64) -> bool {
        self.inner.lock().unwrap().remove(&lease_id).is_some()
    }

    pub fn known_leases(&self) -> Vec<i64> {
        self.inner.lock().unwrap().keys().copied().collect()
    }

    pub fn handle(&self, lease_id: i64) -> Option<LeaseHandle> {
        self.inner.lock().unwrap().get(&lease_id).cloned()
    }

    /// Find the next lease whose deadline is <= now.  Returns None if
    /// nothing is due.
    pub fn next_due(&self, now: Instant) -> Option<i64> {
        let g = self.inner.lock().unwrap();
        g.values()
            .filter(|h| h.next_heartbeat_at <= now)
            .min_by_key(|h| h.next_heartbeat_at)
            .map(|h| h.lease_id)
    }

    /// Record that the lease's keep-alive ping fired; resets its deadline.
    pub fn fired(&self, lease_id: i64) -> bool {
        let mut g = self.inner.lock().unwrap();
        match g.get_mut(&lease_id) {
            Some(h) => {
                h.heartbeats += 1;
                h.next_heartbeat_at = Instant::now()
                    + Duration::from_secs((h.ttl_secs as u64).max(1) / self.pulse_fraction as u64);
                self.pulses_fired.fetch_add(1, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for KeepAliveScheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ── Watch reconnect state machine ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchReconnectStage {
    /// Initial — no stream open yet.
    Initial,
    /// Stream open and receiving events.
    Connected,
    /// Server reported `compacted` — caller must restart from a higher rev.
    Compacted,
    /// Disconnect detected, reconnect attempt in progress.
    Reconnecting,
    /// Reconnected; checking for missed events via progress_notify.
    ProgressCheck,
}

pub struct WatchReconnect {
    state: RwLock<WatchInner>,
    progress_notify_interval: Duration,
}

#[derive(Default)]
struct WatchInner {
    stage: Option<WatchReconnectStage>,
    last_revision: u64,
    /// Last revision the server confirmed via progress_notify.
    last_progress: u64,
    attempts: u32,
}

impl WatchReconnect {
    pub fn new() -> Self {
        let mut inner = WatchInner::default();
        inner.stage = Some(WatchReconnectStage::Initial);
        Self {
            state: RwLock::new(inner),
            progress_notify_interval: Duration::from_secs(10),
        }
    }

    pub fn stage(&self) -> WatchReconnectStage {
        self.state.read().unwrap().stage.unwrap()
    }
    pub fn last_revision(&self) -> u64 {
        self.state.read().unwrap().last_revision
    }
    pub fn attempts(&self) -> u32 {
        self.state.read().unwrap().attempts
    }
    pub fn progress_notify_interval(&self) -> Duration {
        self.progress_notify_interval
    }

    pub fn set_progress_interval(&mut self, d: Duration) {
        self.progress_notify_interval = d;
    }

    /// Stream went up — Initial → Connected.
    pub fn connected(&self, rev: u64) {
        let mut s = self.state.write().unwrap();
        s.stage = Some(WatchReconnectStage::Connected);
        s.last_revision = rev;
        s.last_progress = rev;
        s.attempts = 0;
    }

    /// Stream emitted progress_notify — bookkeeping.
    pub fn progress_notify(&self, rev: u64) {
        let mut s = self.state.write().unwrap();
        s.last_progress = rev;
        s.last_revision = rev;
    }

    /// Network error — Connected/ProgressCheck → Reconnecting.
    pub fn disconnect(&self) {
        let mut s = self.state.write().unwrap();
        s.stage = Some(WatchReconnectStage::Reconnecting);
        s.attempts += 1;
    }

    /// Server returned compacted — caller must rewind.
    pub fn compacted(&self) {
        let mut s = self.state.write().unwrap();
        s.stage = Some(WatchReconnectStage::Compacted);
    }

    /// After reconnect, check for missed events (Reconnecting → ProgressCheck).
    pub fn enter_progress_check(&self) {
        let mut s = self.state.write().unwrap();
        s.stage = Some(WatchReconnectStage::ProgressCheck);
    }

    /// Successful reconnect; ProgressCheck → Connected with the new rev.
    pub fn resumed(&self, rev: u64) {
        let mut s = self.state.write().unwrap();
        s.stage = Some(WatchReconnectStage::Connected);
        s.last_revision = rev;
    }

    /// Revision the caller must rewind to after a `Compacted`.  Returns
    /// the next consumable rev (caller decides "skip" vs "fail").
    pub fn rewind_target(&self, server_compact_rev: u64) -> u64 {
        let s = self.state.read().unwrap();
        s.last_revision.max(server_compact_rev + 1)
    }
}

impl Default for WatchReconnect {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M16
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── AuthInterceptor ───────────────────────────────────────────────

    #[test]
    fn test_auth_interceptor_default_header() {
        // cite: client/v3/auth.go (default 'token' header)
        let a = AuthInterceptor::new();
        assert_eq!(a.header_name(), "token");
    }

    #[test]
    fn test_auth_interceptor_inject_when_token_set() {
        // cite: client/v3/auth.go (UnaryClientInterceptor)
        let a = AuthInterceptor::new();
        a.set_token("abc");
        let mut md = BTreeMap::new();
        assert!(a.inject(&mut md));
        assert_eq!(md.get("token"), Some(&"abc".to_string()));
        assert_eq!(a.inject_count(), 1);
    }

    #[test]
    fn test_auth_interceptor_inject_without_token() {
        let a = AuthInterceptor::new();
        let mut md = BTreeMap::new();
        assert!(!a.inject(&mut md));
        assert!(md.is_empty());
    }

    #[test]
    fn test_auth_interceptor_invalidate_clears_token() {
        // cite: client/v3/auth.go (Unauthenticated ⇒ re-auth)
        let a = AuthInterceptor::new();
        a.set_token("abc");
        assert_eq!(a.invalidate(), Some("abc".into()));
        assert!(a.token().is_none());
        assert_eq!(a.reauth_count(), 1);
    }

    #[test]
    fn test_auth_interceptor_custom_header() {
        // cite: client/v3/auth.go (custom auth header)
        let a = AuthInterceptor::new().with_header("X-Etcd-Token");
        a.set_token("t");
        let mut md = BTreeMap::new();
        assert!(a.inject(&mut md));
        assert!(md.contains_key("X-Etcd-Token"));
    }

    // ── classify_retry ────────────────────────────────────────────────

    #[test]
    fn test_classify_retry_unauthenticated_reauth() {
        // cite: retry_interceptor.go (Unauthenticated ⇒ reauth)
        assert_eq!(
            classify_retry(RpcCode::Unauthenticated, 0, 3),
            RetryDecision::Reauth
        );
    }

    #[test]
    fn test_classify_retry_unavailable_elsewhere() {
        // cite: retry_interceptor.go (Unavailable ⇒ next endpoint)
        assert_eq!(
            classify_retry(RpcCode::Unavailable, 0, 3),
            RetryDecision::RetryElsewhere
        );
    }

    #[test]
    fn test_classify_retry_resource_exhausted_here() {
        // cite: retry_interceptor.go (back-off but stay)
        assert_eq!(
            classify_retry(RpcCode::ResourceExhausted, 0, 3),
            RetryDecision::RetryHere
        );
    }

    #[test]
    fn test_classify_retry_leader_changed_elsewhere() {
        // cite: retry_interceptor.go (NotLeader ⇒ next endpoint)
        assert_eq!(
            classify_retry(RpcCode::LeaderChanged, 0, 3),
            RetryDecision::RetryElsewhere
        );
    }

    #[test]
    fn test_classify_retry_no_leader_elsewhere() {
        assert_eq!(
            classify_retry(RpcCode::NoLeader, 0, 3),
            RetryDecision::RetryElsewhere
        );
    }

    #[test]
    fn test_classify_retry_permanent_codes_fail() {
        // cite: retry_interceptor.go (PermissionDenied / InvalidArg ⇒ no retry)
        assert_eq!(
            classify_retry(RpcCode::PermissionDenied, 0, 3),
            RetryDecision::Fail
        );
        assert_eq!(
            classify_retry(RpcCode::InvalidArgument, 0, 3),
            RetryDecision::Fail
        );
        assert_eq!(classify_retry(RpcCode::NotFound, 0, 3), RetryDecision::Fail);
    }

    #[test]
    fn test_classify_retry_budget_exceeded() {
        // cite: retry_interceptor.go MaxAttempts
        assert_eq!(
            classify_retry(RpcCode::Unavailable, 5, 3),
            RetryDecision::Fail
        );
    }

    #[test]
    fn test_classify_retry_deadline_exceeded_elsewhere() {
        assert_eq!(
            classify_retry(RpcCode::DeadlineExceeded, 0, 3),
            RetryDecision::RetryElsewhere
        );
    }

    // ── KeepAliveScheduler ────────────────────────────────────────────

    #[test]
    fn test_keepalive_register_and_known() {
        // cite: client/v3/lease.go (KeepAliveOnce registration)
        let s = KeepAliveScheduler::new();
        s.register(7, 30);
        assert_eq!(s.known_leases(), vec![7]);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn test_keepalive_deregister_returns_true() {
        let s = KeepAliveScheduler::new();
        s.register(7, 30);
        assert!(s.deregister(7));
        assert!(s.is_empty());
    }

    #[test]
    fn test_keepalive_deregister_missing_returns_false() {
        let s = KeepAliveScheduler::new();
        assert!(!s.deregister(99));
    }

    #[test]
    fn test_keepalive_default_pulse_fraction_three() {
        // cite: client/v3/lease.go (TTL/3 default)
        let s = KeepAliveScheduler::new();
        assert_eq!(s.pulse_fraction(), 3);
    }

    #[test]
    fn test_keepalive_pulse_fraction_clamped_to_one() {
        // cite: defensive — divisor must not be zero
        let s = KeepAliveScheduler::new().with_pulse_fraction(0);
        assert_eq!(s.pulse_fraction(), 1);
    }

    #[test]
    fn test_keepalive_fired_increments_heartbeats_and_counter() {
        // cite: client/v3/lease.go (KeepAliveResponse handler)
        let s = KeepAliveScheduler::new();
        s.register(7, 30);
        assert!(s.fired(7));
        assert_eq!(s.handle(7).unwrap().heartbeats, 1);
        assert_eq!(s.pulses_fired(), 1);
    }

    #[test]
    fn test_keepalive_fired_unknown_returns_false() {
        let s = KeepAliveScheduler::new();
        assert!(!s.fired(99));
    }

    #[test]
    fn test_keepalive_next_due_picks_overdue() {
        // cite: lease.go (heartbeat scheduler scan)
        let s = KeepAliveScheduler::new();
        s.register(7, 30);
        // Force the deadline into the past.
        {
            let mut g = s.inner.lock().unwrap();
            let h = g.get_mut(&7).unwrap();
            h.next_heartbeat_at = Instant::now() - Duration::from_secs(1);
        }
        assert_eq!(s.next_due(Instant::now()), Some(7));
    }

    #[test]
    fn test_keepalive_next_due_none_when_clear() {
        let s = KeepAliveScheduler::new();
        s.register(7, 30);
        assert!(s.next_due(Instant::now()).is_none());
    }

    // ── WatchReconnect ────────────────────────────────────────────────

    #[test]
    fn test_watch_starts_in_initial() {
        // cite: client/v3/watch.go (initial state)
        let w = WatchReconnect::new();
        assert_eq!(w.stage(), WatchReconnectStage::Initial);
    }

    #[test]
    fn test_watch_connected_sets_revision() {
        // cite: watch.go (CreateResponse carries header.revision)
        let w = WatchReconnect::new();
        w.connected(42);
        assert_eq!(w.stage(), WatchReconnectStage::Connected);
        assert_eq!(w.last_revision(), 42);
    }

    #[test]
    fn test_watch_disconnect_increments_attempts() {
        // cite: watch.go (Reconnect attempts counted)
        let w = WatchReconnect::new();
        w.connected(0);
        w.disconnect();
        w.disconnect();
        assert_eq!(w.attempts(), 2);
        assert_eq!(w.stage(), WatchReconnectStage::Reconnecting);
    }

    #[test]
    fn test_watch_progress_notify_advances_revision() {
        // cite: watch.go (progress_notify advances last_revision)
        let w = WatchReconnect::new();
        w.connected(10);
        w.progress_notify(20);
        assert_eq!(w.last_revision(), 20);
    }

    #[test]
    fn test_watch_resumed_returns_to_connected() {
        // cite: watch.go (post-reconnect ⇒ Connected)
        let w = WatchReconnect::new();
        w.connected(0);
        w.disconnect();
        w.enter_progress_check();
        w.resumed(50);
        assert_eq!(w.stage(), WatchReconnectStage::Connected);
        assert_eq!(w.last_revision(), 50);
    }

    #[test]
    fn test_watch_compacted_stage() {
        // cite: watch.go (compacted ⇒ caller rewinds)
        let w = WatchReconnect::new();
        w.connected(0);
        w.compacted();
        assert_eq!(w.stage(), WatchReconnectStage::Compacted);
    }

    #[test]
    fn test_watch_rewind_target_max_of_last_and_compact() {
        // cite: watch.go (resume rev = max(last, compact+1))
        let w = WatchReconnect::new();
        w.connected(50);
        // server compacted at 100 ⇒ resume at 101
        assert_eq!(w.rewind_target(100), 101);
    }

    #[test]
    fn test_watch_rewind_target_when_last_ahead() {
        let w = WatchReconnect::new();
        w.connected(200);
        assert_eq!(w.rewind_target(100), 200);
    }

    #[test]
    fn test_watch_attempts_reset_on_connected() {
        // cite: watch.go (successful (re)connect resets backoff)
        let w = WatchReconnect::new();
        w.disconnect();
        w.disconnect();
        w.connected(0);
        assert_eq!(w.attempts(), 0);
    }

    #[test]
    fn test_watch_progress_notify_interval_default() {
        // cite: watch.go progressNotifyInterval default
        let w = WatchReconnect::new();
        assert_eq!(w.progress_notify_interval(), Duration::from_secs(10));
    }
}
