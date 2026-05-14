// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provider runtime — error classification, rate limits, controller
//! options, pre-init hooks.
//!
//! Mirrors the tooling around the cloud-provider trait surface that lives
//! in `staging/src/k8s.io/cloud-provider/options` (controller flags) and
//! `staging/src/k8s.io/client-go/util/retry` (error classification +
//! retry helpers).

use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── Error classification ────────────────────────────────────────────────────

/// How a controller should treat an error returned from a cloud call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorClass {
    /// Transient — retry with backoff.
    Retryable,
    /// API-level rate limit — retry after the cooldown window.
    Throttled,
    /// Permanent — bubble up; no retry.
    Permanent,
}

/// Classify a `CloudError`. Mirrors the dispatch in
/// `staging/src/k8s.io/client-go/util/retry/util.go::IsErrorRetryable`.
pub fn classify_error(err: &CloudError) -> ErrorClass {
    match err {
        CloudError::Upstream { reason, .. } => classify_reason(reason),
        CloudError::InvalidConfig { .. } => ErrorClass::Permanent,
        CloudError::TenantDenied { .. } => ErrorClass::Permanent,
        CloudError::Unimplemented(_) => ErrorClass::Permanent,
    }
}

fn classify_reason(reason: &str) -> ErrorClass {
    let r = reason.to_lowercase();
    if r.contains("429") || r.contains("rate limit") || r.contains("throttl") {
        return ErrorClass::Throttled;
    }
    if r.contains("503")
        || r.contains("502")
        || r.contains("504")
        || r.contains("500")
        || r.contains("connection reset")
        || r.contains("timeout")
        || r.contains("temporarily")
    {
        return ErrorClass::Retryable;
    }
    ErrorClass::Permanent
}

/// Helper: should the controller retry the next cloud call?
pub fn should_retry(err: &CloudError) -> bool {
    matches!(classify_error(err), ErrorClass::Retryable | ErrorClass::Throttled)
}

// ─── Token-bucket rate limiter ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: u32,
    tokens: u32,
    /// Tokens added per second.
    refill_per_second: u32,
}

impl TokenBucket {
    pub fn new(capacity: u32, refill_per_second: u32) -> Result<Self, CloudError> {
        if capacity == 0 || refill_per_second == 0 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "rate-limiter capacity and refill must be > 0".into(),
            });
        }
        Ok(Self { capacity, tokens: capacity, refill_per_second })
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn available(&self) -> u32 {
        self.tokens
    }

    /// Try to take `n` tokens. Returns `Ok(())` on success, otherwise the
    /// estimated wait in milliseconds before enough tokens are available.
    pub fn try_take(&mut self, n: u32) -> Result<(), u32> {
        if n <= self.tokens {
            self.tokens -= n;
            Ok(())
        } else {
            let deficit = n - self.tokens;
            let wait_ms = (deficit as u64 * 1_000 / self.refill_per_second.max(1) as u64) as u32;
            Err(wait_ms.max(1))
        }
    }

    /// Add `elapsed_ms` of refill to the bucket, capped at `capacity`.
    pub fn refill(&mut self, elapsed_ms: u32) {
        let added = (elapsed_ms as u64 * self.refill_per_second as u64 / 1_000) as u32;
        self.tokens = self.tokens.saturating_add(added).min(self.capacity);
    }
}

// ─── Controller options ──────────────────────────────────────────────────────

/// Mirrors `cloud-controller-manager/app/options/options.go`'s flag set
/// (subset relevant to test parity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerOptions {
    pub concurrent_node_syncs: u32,
    pub concurrent_service_syncs: u32,
    pub node_monitor_period_seconds: u32,
    pub route_reconciliation_period_seconds: u32,
    pub allow_untagged_cloud: bool,
}

impl ControllerOptions {
    pub fn defaults() -> Self {
        Self {
            concurrent_node_syncs: 1,
            concurrent_service_syncs: 1,
            node_monitor_period_seconds: 5,
            route_reconciliation_period_seconds: 10,
            allow_untagged_cloud: false,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if self.concurrent_node_syncs == 0 || self.concurrent_node_syncs > 50 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "concurrent_node_syncs {} outside [1, 50]",
                    self.concurrent_node_syncs
                ),
            });
        }
        if self.concurrent_service_syncs == 0 || self.concurrent_service_syncs > 50 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "concurrent_service_syncs {} outside [1, 50]",
                    self.concurrent_service_syncs
                ),
            });
        }
        if !(1..=300).contains(&self.node_monitor_period_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "node_monitor_period_seconds {} outside [1, 300]",
                    self.node_monitor_period_seconds
                ),
            });
        }
        if !(1..=600).contains(&self.route_reconciliation_period_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "route_reconciliation_period_seconds {} outside [1, 600]",
                    self.route_reconciliation_period_seconds
                ),
            });
        }
        Ok(())
    }
}

// ─── PreInitNodeHook ─────────────────────────────────────────────────────────

/// Marker trait for cloud-specific pre-init logic. Mirrors the upstream
/// `Cloud.HasClusterID` + `Cloud.Initialize` hooks called before the
/// initializer taint is removed from a node.
pub trait PreInitNodeHook {
    fn before_taint_removal(&self, node_name: &str) -> Result<(), CloudError>;
}

/// Run the hook then return the resulting outcome. Pure helper: lets
/// callers swap in a no-op hook in tests.
pub fn run_pre_init(hook: &dyn PreInitNodeHook, node_name: &str) -> Result<(), CloudError> {
    hook.before_taint_removal(node_name)
}

// ─── Health probe ────────────────────────────────────────────────────────────

/// Minimal cloud-provider-side health probe. Mirrors
/// `cloudprovider.Interface.HasClusterID`'s read-only check.
pub trait HealthProbe {
    fn alive(&self) -> bool;
    fn last_error(&self) -> Option<&str>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) {
        let (cite, _t) = test_ctx!(path, sym, tenant);
        assert_eq!(cite.repo, "kubernetes/kubernetes");
    }

    // ─── Error classification ────────────────────────────────────────────────

    #[test]
    fn rate_limit_reason_is_throttled() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "IsErrorRetryable");
        let err = CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: "HTTP 429: rate limit exceeded".into(),
        };
        assert_eq!(classify_error(&err), ErrorClass::Throttled);
        assert!(should_retry(&err));
    }

    #[test]
    fn http_5xx_reason_is_retryable() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "IsErrorRetryable");
        for code in ["500", "502", "503", "504"] {
            let err = CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("HTTP {code}: server error"),
            };
            assert_eq!(classify_error(&err), ErrorClass::Retryable, "code {code}");
        }
    }

    #[test]
    fn timeout_and_connection_reset_are_retryable() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "IsErrorRetryable");
        let err = CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: "i/o timeout".into(),
        };
        assert_eq!(classify_error(&err), ErrorClass::Retryable);
        let err = CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: "connection reset by peer".into(),
        };
        assert_eq!(classify_error(&err), ErrorClass::Retryable);
    }

    #[test]
    fn invalid_config_is_permanent() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "IsErrorRetryable");
        let err = CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: "x".into(),
        };
        assert_eq!(classify_error(&err), ErrorClass::Permanent);
        assert!(!should_retry(&err));
    }

    #[test]
    fn tenant_denied_is_permanent() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "IsErrorRetryable");
        let err = CloudError::TenantDenied {
            tenant: crate::types::TenantId::new("attacker").expect("test fixture"),
            kind: "Service",
            name: "web".into(),
        };
        assert_eq!(classify_error(&err), ErrorClass::Permanent);
    }

    #[test]
    fn unimplemented_is_permanent() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "IsErrorRetryable");
        let err = CloudError::Unimplemented("v6 allocator");
        assert_eq!(classify_error(&err), ErrorClass::Permanent);
    }

    #[test]
    fn unknown_upstream_reason_defaults_to_permanent() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "IsErrorRetryable");
        let err = CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: "object not found".into(),
        };
        assert_eq!(classify_error(&err), ErrorClass::Permanent);
        assert!(!should_retry(&err));
    }

    // ─── Token bucket ────────────────────────────────────────────────────────

    #[test]
    fn token_bucket_constructor_rejects_zero_capacity() {
        ctx("acme", "staging/src/k8s.io/client-go/util/flowcontrol/throttle.go", "tokenBucketRateLimiter");
        assert!(TokenBucket::new(0, 5).is_err());
        assert!(TokenBucket::new(10, 0).is_err());
    }

    #[test]
    fn token_bucket_starts_full_and_drains_with_take() {
        ctx("acme", "staging/src/k8s.io/client-go/util/flowcontrol/throttle.go", "tokenBucketRateLimiter");
        let mut tb = TokenBucket::new(10, 5).unwrap();
        assert_eq!(tb.available(), 10);
        assert!(tb.try_take(7).is_ok());
        assert_eq!(tb.available(), 3);
    }

    #[test]
    fn token_bucket_returns_wait_when_empty() {
        ctx("acme", "staging/src/k8s.io/client-go/util/flowcontrol/throttle.go", "tokenBucketRateLimiter");
        let mut tb = TokenBucket::new(5, 5).unwrap();
        tb.try_take(5).unwrap();
        let wait = tb.try_take(3).unwrap_err();
        assert!(wait > 0);
    }

    #[test]
    fn token_bucket_refill_caps_at_capacity() {
        ctx("acme", "staging/src/k8s.io/client-go/util/flowcontrol/throttle.go", "tokenBucketRateLimiter");
        let mut tb = TokenBucket::new(10, 5).unwrap();
        tb.try_take(5).unwrap();
        tb.refill(60_000); // 60 s × 5/s = 300 tokens, but capped at 10
        assert_eq!(tb.available(), 10);
    }

    #[test]
    fn token_bucket_refill_partial_window() {
        ctx("acme", "staging/src/k8s.io/client-go/util/flowcontrol/throttle.go", "tokenBucketRateLimiter");
        let mut tb = TokenBucket::new(20, 10).unwrap();
        tb.try_take(15).unwrap();
        tb.refill(500); // 500 ms × 10/s = 5
        assert_eq!(tb.available(), 10);
    }

    #[test]
    fn token_bucket_capacity_is_constant() {
        ctx("acme", "staging/src/k8s.io/client-go/util/flowcontrol/throttle.go", "tokenBucketRateLimiter");
        let tb = TokenBucket::new(7, 1).unwrap();
        assert_eq!(tb.capacity(), 7);
    }

    // ─── Controller options ──────────────────────────────────────────────────

    #[test]
    fn controller_options_defaults_validate() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "Validate");
        assert!(ControllerOptions::defaults().validate().is_ok());
    }

    #[test]
    fn controller_options_concurrent_node_syncs_must_be_in_range() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "Validate");
        let mut o = ControllerOptions::defaults();
        o.concurrent_node_syncs = 0;
        assert!(matches!(o.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        o.concurrent_node_syncs = 100;
        assert!(matches!(o.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn controller_options_concurrent_service_syncs_must_be_in_range() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "Validate");
        let mut o = ControllerOptions::defaults();
        o.concurrent_service_syncs = 0;
        assert!(matches!(o.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn controller_options_node_monitor_period_must_be_in_range() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "Validate");
        let mut o = ControllerOptions::defaults();
        o.node_monitor_period_seconds = 0;
        assert!(matches!(o.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        o.node_monitor_period_seconds = 1000;
        assert!(matches!(o.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn controller_options_route_reconciliation_must_be_in_range() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "Validate");
        let mut o = ControllerOptions::defaults();
        o.route_reconciliation_period_seconds = 0;
        assert!(matches!(o.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── PreInitNodeHook ─────────────────────────────────────────────────────

    struct AcceptingHook;
    impl PreInitNodeHook for AcceptingHook {
        fn before_taint_removal(&self, _node_name: &str) -> Result<(), CloudError> {
            Ok(())
        }
    }

    struct RejectingHook;
    impl PreInitNodeHook for RejectingHook {
        fn before_taint_removal(&self, node_name: &str) -> Result<(), CloudError> {
            Err(CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("preflight failed for {node_name}"),
            })
        }
    }

    #[test]
    fn run_pre_init_returns_ok_for_accepting_hook() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/cloud.go", "Initialize");
        assert!(run_pre_init(&AcceptingHook, "node-1").is_ok());
    }

    #[test]
    fn run_pre_init_propagates_hook_error() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/cloud.go", "Initialize");
        let err = run_pre_init(&RejectingHook, "node-1").unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    // ─── HealthProbe ─────────────────────────────────────────────────────────

    struct OkProbe;
    impl HealthProbe for OkProbe {
        fn alive(&self) -> bool {
            true
        }
        fn last_error(&self) -> Option<&str> {
            None
        }
    }

    struct DeadProbe(&'static str);
    impl HealthProbe for DeadProbe {
        fn alive(&self) -> bool {
            false
        }
        fn last_error(&self) -> Option<&str> {
            Some(self.0)
        }
    }

    #[test]
    fn health_probe_alive_no_error() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/cloud.go", "HasClusterID");
        let p = OkProbe;
        assert!(p.alive());
        assert!(p.last_error().is_none());
    }

    #[test]
    fn health_probe_dead_carries_last_error() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/cloud.go", "HasClusterID");
        let p = DeadProbe("503");
        assert!(!p.alive());
        assert_eq!(p.last_error(), Some("503"));
    }
}
