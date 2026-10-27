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

use crate::error::{CertManagerError, CertManagerResult};
use crate::issuer::IssueOutcome;
use crate::models::IssuerRef;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// An ordered list of issuers plus the cooldown applied to a failed
/// issuer before it is retried.
#[derive(Debug, Clone)]
pub struct FailoverPolicy {
    /// Issuers in descending preference — index 0 is the primary.
    pub issuers: Vec<IssuerRef>,
    /// Seconds a failed issuer is parked before it becomes eligible
    /// again. `0` disables the cooldown (every issuer is always
    /// eligible).
    pub cooldown_seconds: i64,
}

impl FailoverPolicy {
    pub fn new(issuers: Vec<IssuerRef>, cooldown_seconds: i64) -> Self {
        Self {
            issuers,
            cooldown_seconds,
        }
    }

    /// Issuers not currently in cooldown, in preference order.
    pub fn eligible<'a>(&'a self, state: &FailoverState, now: DateTime<Utc>) -> Vec<&'a IssuerRef> {
        self.issuers
            .iter()
            .filter(|r| !state.in_cooldown(&r.name, self.cooldown_seconds, now))
            .collect()
    }

    /// The most-preferred eligible issuer, or `None` if every issuer is
    /// in cooldown (or the policy is empty).
    pub fn next_issuer<'a>(
        &'a self,
        state: &FailoverState,
        now: DateTime<Utc>,
    ) -> Option<&'a IssuerRef> {
        self.eligible(state, now).into_iter().next()
    }

    /// Try each eligible issuer in preference order until one succeeds.
    ///
    /// `f` performs the actual issuance for a given issuer. On `Err` the
    /// issuer is parked in cooldown and the next eligible issuer is
    /// tried; on `Ok` the issuer's prior failure is cleared and the
    /// `(issuer, outcome)` pair is returned. If no issuer is eligible
    /// returns [`CertManagerError::NoEligibleIssuer`]; if every eligible
    /// issuer fails returns [`CertManagerError::AllIssuersFailed`].
    pub fn attempt<F>(
        &self,
        state: &mut FailoverState,
        now: DateTime<Utc>,
        mut f: F,
    ) -> CertManagerResult<(IssuerRef, IssueOutcome)>
    where
        F: FnMut(&IssuerRef) -> CertManagerResult<IssueOutcome>,
    {
        let eligible: Vec<IssuerRef> =
            self.eligible(state, now).into_iter().cloned().collect();
        if eligible.is_empty() {
            return Err(CertManagerError::NoEligibleIssuer);
        }
        let mut tried = 0usize;
        let mut last_error = String::new();
        for iref in &eligible {
            tried += 1;
            match f(iref) {
                Ok(outcome) => {
                    state.record_success(&iref.name);
                    return Ok((iref.clone(), outcome));
                }
                Err(e) => {
                    last_error = e.to_string();
                    state.record_failure(&iref.name, now);
                }
            }
        }
        Err(CertManagerError::AllIssuersFailed { tried, last_error })
    }
}

/// Mutable failover bookkeeping — the last failure time per issuer name.
#[derive(Debug, Clone, Default)]
pub struct FailoverState {
    failures: HashMap<String, DateTime<Utc>>,
}

impl FailoverState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Park `issuer_name` in cooldown starting at `now`.
    pub fn record_failure(&mut self, issuer_name: &str, now: DateTime<Utc>) {
        self.failures.insert(issuer_name.to_string(), now);
    }

    /// Clear any recorded failure for `issuer_name` (it recovered).
    pub fn record_success(&mut self, issuer_name: &str) {
        self.failures.remove(issuer_name);
    }

    /// Whether `issuer_name` is still inside its cooldown window. A
    /// `cooldown_seconds` of `0` means an issuer is never parked.
    pub fn in_cooldown(&self, issuer_name: &str, cooldown_seconds: i64, now: DateTime<Utc>) -> bool {
        if cooldown_seconds <= 0 {
            return false;
        }
        match self.failures.get(issuer_name) {
            Some(failed_at) => now < *failed_at + Duration::seconds(cooldown_seconds),
            None => false,
        }
    }
}

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
