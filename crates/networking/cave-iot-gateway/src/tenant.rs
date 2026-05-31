// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Multi-tenant isolation + quotas.
//!
//! Ports the tenant-scoping spine of ThingsBoard: a `TenantProfile`'s entity
//! quotas (max devices) + message rate limits, strict cross-tenant ownership
//! denial (every entity carries a `tenantId`; access in another tenant's
//! context is refused), and a fixed-window per-tenant rate limiter modelling
//! the `TbRateLimits` transport guard. Clocks are injected (`now_ms`) so the
//! logic is deterministic and side-effect free.

use crate::{IotError, Result};
use std::collections::HashMap;

/// Per-tenant quota / limit configuration (subset of `DefaultTenantProfileConfiguration`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantLimits {
    /// `None` = unlimited.
    pub max_devices: Option<u64>,
    pub max_messages_per_min: Option<u64>,
}

/// Tenant registry + device-count accounting.
#[derive(Debug, Default)]
pub struct TenantManager {
    limits: HashMap<String, TenantLimits>,
    device_counts: HashMap<String, u64>,
}

impl TenantManager {
    pub fn new() -> TenantManager {
        TenantManager::default()
    }

    pub fn register(&mut self, tenant_id: &str, limits: TenantLimits) {
        self.limits.insert(tenant_id.to_string(), limits);
        self.device_counts.entry(tenant_id.to_string()).or_insert(0);
    }

    pub fn device_count(&self, tenant_id: &str) -> u64 {
        self.device_counts.get(tenant_id).copied().unwrap_or(0)
    }

    /// Check whether a tenant has room for one more device.
    pub fn can_add_device(&self, tenant_id: &str) -> Result<()> {
        let limits = self
            .limits
            .get(tenant_id)
            .ok_or_else(|| IotError::NotFound(format!("tenant {tenant_id}")))?;
        if let Some(max) = limits.max_devices {
            if self.device_count(tenant_id) >= max {
                return Err(IotError::TenantViolation(format!(
                    "tenant {tenant_id} at device quota ({max})"
                )));
            }
        }
        Ok(())
    }

    pub fn incr_device(&mut self, tenant_id: &str) {
        *self.device_counts.entry(tenant_id.to_string()).or_insert(0) += 1;
    }

    pub fn decr_device(&mut self, tenant_id: &str) {
        if let Some(c) = self.device_counts.get_mut(tenant_id) {
            *c = c.saturating_sub(1);
        }
    }

    /// Deny access when an entity's owning tenant differs from the caller's.
    pub fn check_ownership(&self, caller_tenant: &str, entity_tenant: &str) -> Result<()> {
        if caller_tenant == entity_tenant {
            Ok(())
        } else {
            Err(IotError::TenantViolation(format!(
                "tenant {caller_tenant} may not access entity owned by {entity_tenant}"
            )))
        }
    }
}

/// Fixed-window per-tenant rate limiter.
#[derive(Debug)]
pub struct RateLimiter {
    max_per_window: u64,
    window_ms: i64,
    state: HashMap<String, (i64, u64)>, // tenant → (window_start, count)
}

impl RateLimiter {
    pub fn new(max_per_window: u64, window_ms: i64) -> RateLimiter {
        RateLimiter { max_per_window, window_ms, state: HashMap::new() }
    }

    /// Record an attempt at `now_ms`; returns whether it is within budget.
    pub fn allow(&mut self, tenant_id: &str, now_ms: i64) -> bool {
        let window = if self.window_ms > 0 {
            (now_ms / self.window_ms) * self.window_ms
        } else {
            0
        };
        let entry = self.state.entry(tenant_id.to_string()).or_insert((window, 0));
        if entry.0 != window {
            *entry = (window, 0);
        }
        if entry.1 < self.max_per_window {
            entry.1 += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_quota_blocks_at_limit() {
        let mut tm = TenantManager::new();
        tm.register("t1", TenantLimits { max_devices: Some(2), max_messages_per_min: None });
        assert!(tm.can_add_device("t1").is_ok());
        tm.incr_device("t1");
        tm.incr_device("t1");
        // Now at 2 devices == limit → next add rejected.
        let err = tm.can_add_device("t1").unwrap_err();
        assert!(matches!(err, crate::IotError::TenantViolation(_)));
    }

    #[test]
    fn decr_device_frees_quota() {
        let mut tm = TenantManager::new();
        tm.register("t1", TenantLimits { max_devices: Some(1), max_messages_per_min: None });
        tm.incr_device("t1");
        assert!(tm.can_add_device("t1").is_err());
        tm.decr_device("t1");
        assert!(tm.can_add_device("t1").is_ok());
    }

    #[test]
    fn unlimited_quota_never_blocks() {
        let mut tm = TenantManager::new();
        tm.register("t1", TenantLimits { max_devices: None, max_messages_per_min: None });
        for _ in 0..1000 {
            tm.incr_device("t1");
        }
        assert!(tm.can_add_device("t1").is_ok());
    }

    #[test]
    fn cross_tenant_access_is_denied() {
        let tm = TenantManager::new();
        // An entity owned by t2 may not be accessed in t1's context.
        assert!(tm.check_ownership("t1", "t1").is_ok());
        assert!(matches!(
            tm.check_ownership("t1", "t2").unwrap_err(),
            crate::IotError::TenantViolation(_)
        ));
    }

    #[test]
    fn unknown_tenant_cannot_add_device() {
        let tm = TenantManager::new();
        assert!(matches!(
            tm.can_add_device("ghost").unwrap_err(),
            crate::IotError::NotFound(_)
        ));
    }

    #[test]
    fn rate_limiter_fixed_window() {
        let mut rl = RateLimiter::new(2, 1000);
        // 2 allowed in window [0,1000)
        assert!(rl.allow("t1", 0));
        assert!(rl.allow("t1", 500));
        assert!(!rl.allow("t1", 900));
        // window rolls over at 1000 → counter resets.
        assert!(rl.allow("t1", 1000));
    }

    #[test]
    fn rate_limiter_is_per_tenant() {
        let mut rl = RateLimiter::new(1, 1000);
        assert!(rl.allow("a", 0));
        assert!(!rl.allow("a", 1));
        // A different tenant has its own budget.
        assert!(rl.allow("b", 1));
    }
}
