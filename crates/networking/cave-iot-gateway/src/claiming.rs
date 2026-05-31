// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Device claiming. (RED.)

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
