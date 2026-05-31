// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Multi-tenant isolation + quotas. (RED.)

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
