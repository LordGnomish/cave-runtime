// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Multi-issuer failover — ordered issuer preference with per-issuer
//! cooldown after a failed issuance.
//!
//! cert-manager has no first-party multi-issuer failover (a Certificate
//! references exactly one `issuerRef`). This is a cave control-plane
//! invariant: an operator declares an ordered list of issuers and, when
//! the preferred issuer fails, cave-cert-manager falls through to the
//! next eligible issuer on the next reconcile attempt. A failed issuer
//! is parked in a cooldown window so a flapping CA does not get retried
//! on every tick.
//!
//! The policy is pure control-plane logic — it composes with the
//! [`crate::issuer::IssuerRegistry`] through a caller-supplied closure so
//! the failover machinery never needs to know how any individual issuer
//! signs.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issuer::IssueOutcome;
    use crate::models::{IssuerRef, IssuerRefKind};
    use chrono::{Duration, Utc};

    fn iref(name: &str) -> IssuerRef {
        IssuerRef {
            name: name.into(),
            kind: IssuerRefKind::ClusterIssuer,
            group: "cert-manager.io".into(),
        }
    }

    fn outcome(serial: &str) -> IssueOutcome {
        let now = Utc::now();
        IssueOutcome {
            certificate_chain_pem: "chain".into(),
            ca_pem: "ca".into(),
            not_before: now,
            not_after: now + Duration::seconds(3600),
            serial: serial.into(),
        }
    }

    fn policy() -> FailoverPolicy {
        FailoverPolicy::new(
            vec![iref("primary"), iref("secondary"), iref("tertiary")],
            300,
        )
    }

    #[test]
    fn next_issuer_prefers_first_when_no_failures() {
        let p = policy();
        let st = FailoverState::new();
        assert_eq!(p.next_issuer(&st, Utc::now()).unwrap().name, "primary");
    }

    #[test]
    fn next_issuer_skips_cooled_down_issuer() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        st.record_failure("primary", now);
        assert_eq!(p.next_issuer(&st, now).unwrap().name, "secondary");
    }

    #[test]
    fn cooldown_expires_after_window() {
        let p = policy();
        let mut st = FailoverState::new();
        let t0 = Utc::now();
        st.record_failure("primary", t0);
        // still cooling down inside the window
        assert_eq!(
            p.next_issuer(&st, t0 + Duration::seconds(299)).unwrap().name,
            "secondary"
        );
        // window elapsed → primary eligible again
        assert_eq!(
            p.next_issuer(&st, t0 + Duration::seconds(301)).unwrap().name,
            "primary"
        );
    }

    #[test]
    fn record_success_clears_failure() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        st.record_failure("primary", now);
        st.record_success("primary");
        assert_eq!(p.next_issuer(&st, now).unwrap().name, "primary");
    }

    #[test]
    fn next_issuer_none_when_all_cooled_down() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        for name in ["primary", "secondary", "tertiary"] {
            st.record_failure(name, now);
        }
        assert!(p.next_issuer(&st, now).is_none());
    }

    #[test]
    fn empty_policy_next_is_none() {
        let p = FailoverPolicy::new(vec![], 300);
        let st = FailoverState::new();
        assert!(p.next_issuer(&st, Utc::now()).is_none());
    }

    #[test]
    fn attempt_returns_first_success_and_records_prior_failure() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        let (used, out) = p
            .attempt(&mut st, now, |iref| {
                if iref.name == "primary" {
                    Err(CertManagerError::AcmeOrder("primary down".into()))
                } else {
                    Ok(outcome(&iref.name))
                }
            })
            .unwrap();
        assert_eq!(used.name, "secondary");
        assert_eq!(out.serial, "secondary");
        // primary is now parked in cooldown
        assert!(st.in_cooldown("primary", p.cooldown_seconds, now));
        // secondary succeeded → not in cooldown
        assert!(!st.in_cooldown("secondary", p.cooldown_seconds, now));
    }

    #[test]
    fn attempt_all_fail_returns_all_issuers_failed() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        let err = p
            .attempt(&mut st, now, |_iref| {
                Err(CertManagerError::AcmeOrder("down".into()))
            })
            .unwrap_err();
        match err {
            CertManagerError::AllIssuersFailed { tried, .. } => assert_eq!(tried, 3),
            other => panic!("expected AllIssuersFailed, got {other:?}"),
        }
    }

    #[test]
    fn attempt_skips_already_cooled_issuer() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        st.record_failure("primary", now);
        let (used, _) = p
            .attempt(&mut st, now, |iref| Ok(outcome(&iref.name)))
            .unwrap();
        assert_eq!(used.name, "secondary");
    }

    #[test]
    fn attempt_no_eligible_issuer_errors() {
        let p = FailoverPolicy::new(vec![], 300);
        let mut st = FailoverState::new();
        let err = p
            .attempt(&mut st, Utc::now(), |iref| Ok(outcome(&iref.name)))
            .unwrap_err();
        assert!(matches!(err, CertManagerError::NoEligibleIssuer));
    }

    #[test]
    fn eligible_order_is_stable() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        // cool down the middle issuer only
        st.record_failure("secondary", now);
        let eligible: Vec<String> =
            p.eligible(&st, now).into_iter().map(|r| r.name.clone()).collect();
        assert_eq!(eligible, vec!["primary".to_string(), "tertiary".to_string()]);
    }

    #[test]
    fn success_after_failover_reenables_primary_on_recovery() {
        let p = policy();
        let mut st = FailoverState::new();
        let now = Utc::now();
        // primary fails, secondary used
        let _ = p
            .attempt(&mut st, now, |iref| {
                if iref.name == "primary" {
                    Err(CertManagerError::AcmeOrder("flap".into()))
                } else {
                    Ok(outcome(&iref.name))
                }
            })
            .unwrap();
        // once the cooldown elapses primary is preferred again
        let later = now + Duration::seconds(301);
        assert_eq!(p.next_issuer(&st, later).unwrap().name, "primary");
    }
}
