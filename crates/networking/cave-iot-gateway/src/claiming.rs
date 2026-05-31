// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Device claiming — ThingsBoard `DeviceClaimService`.
//!
//! A device is made claimable for a bounded window by registering a claim
//! secret + expiry. A customer then presents the secret within the window to
//! bind (claim) the device to themselves. `reclaim` releases the device and
//! drops the claim info (the device must be made claimable again to re-bind).
//! Clocks are injected for determinism.

use crate::{IotError, Result};
use std::collections::HashMap;

/// Outcome of a successful claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimResult {
    Claimed { customer_id: String },
}

#[derive(Debug, Clone)]
struct ClaimInfo {
    secret: String,
    expires_at: i64,
}

/// Device-claiming service.
#[derive(Debug, Default)]
pub struct ClaimingService {
    claim_info: HashMap<String, ClaimInfo>,
    owners: HashMap<String, String>, // device → customer
}

impl ClaimingService {
    pub fn new() -> ClaimingService {
        ClaimingService::default()
    }

    /// Make a device claimable with `secret` until `expires_at` (inclusive).
    pub fn register_claim_info(&mut self, device_id: &str, secret: &str, expires_at: i64) {
        self.claim_info.insert(
            device_id.to_string(),
            ClaimInfo { secret: secret.to_string(), expires_at },
        );
    }

    /// Attempt to claim a device for a customer at `now_ms`.
    pub fn claim(
        &mut self,
        device_id: &str,
        secret: &str,
        customer_id: &str,
        now_ms: i64,
    ) -> Result<ClaimResult> {
        let info = self
            .claim_info
            .get(device_id)
            .ok_or_else(|| IotError::NotFound(format!("device {device_id} is not claimable")))?;
        if info.secret != secret {
            return Err(IotError::Invalid("claim secret mismatch".into()));
        }
        if now_ms > info.expires_at {
            return Err(IotError::Invalid("claim window expired".into()));
        }
        self.owners
            .insert(device_id.to_string(), customer_id.to_string());
        Ok(ClaimResult::Claimed { customer_id: customer_id.to_string() })
    }

    /// The customer a device is currently claimed by, if any.
    pub fn owner(&self, device_id: &str) -> Option<&str> {
        self.owners.get(device_id).map(String::as_str)
    }

    /// Release a device: drop its customer binding and claim info.
    pub fn reclaim(&mut self, device_id: &str) {
        self.owners.remove(device_id);
        self.claim_info.remove(device_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> ClaimingService {
        let mut s = ClaimingService::new();
        // device "d1" claimable with secret "S" until t=1000.
        s.register_claim_info("d1", "S", 1000);
        s
    }

    #[test]
    fn valid_claim_binds_device_to_customer() {
        let mut s = svc();
        let res = s.claim("d1", "S", "cust-1", 500);
        assert_eq!(res, Ok(ClaimResult::Claimed { customer_id: "cust-1".into() }));
        assert_eq!(s.owner("d1"), Some("cust-1"));
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let mut s = svc();
        assert!(s.claim("d1", "WRONG", "cust-1", 500).is_err());
        assert_eq!(s.owner("d1"), None);
    }

    #[test]
    fn expired_claim_is_rejected() {
        let mut s = svc();
        assert!(s.claim("d1", "S", "cust-1", 1500).is_err());
    }

    #[test]
    fn claim_window_is_inclusive_of_expiry() {
        let mut s = svc();
        assert!(s.claim("d1", "S", "cust-1", 1000).is_ok());
    }

    #[test]
    fn reclaim_clears_binding_and_claim_info() {
        let mut s = svc();
        s.claim("d1", "S", "cust-1", 500).unwrap();
        s.reclaim("d1");
        assert_eq!(s.owner("d1"), None);
        // After reclaim there is no claim info → claiming fails until re-registered.
        assert!(s.claim("d1", "S", "cust-2", 600).is_err());
    }

    #[test]
    fn claiming_unknown_device_errors() {
        let mut s = ClaimingService::new();
        assert!(s.claim("ghost", "S", "c", 0).is_err());
    }
}
