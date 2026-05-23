// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Renewal scheduler — computes "next renewal time" per Certificate and
//! returns a sorted plan the controller drains in order.
//!
//! Cite: `pkg/controller/certificates/trigger/trigger.go::shouldReissue` —
//! cert-manager triggers a reissue when `now >= notAfter - renewBefore`.

use crate::models::{Certificate, CertificateConditionType, ConditionStatus};
use chrono::{DateTime, Duration, Utc};

/// One scheduled renewal — `(certificate_id, renew_at)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenewalPlan {
    pub certificate_id: uuid::Uuid,
    pub name: String,
    pub renew_at: DateTime<Utc>,
    pub reason: RenewalReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenewalReason {
    /// First issuance — no status yet.
    InitialIssuance,
    /// `notBefore` window not yet established but cert has no Ready
    /// status — treat as immediate retry.
    NotReady,
    /// We crossed the `notAfter - renewBefore` threshold.
    RenewBeforeReached,
    /// Cert has already expired.
    Expired,
}

#[derive(Debug, Default)]
pub struct RenewalScheduler;

impl RenewalScheduler {
    /// Return all certificates that need an issuance / reissue, sorted
    /// by `renew_at` ascending. A clock argument keeps the scheduler
    /// deterministic under tests.
    pub fn plan(&self, certs: &[Certificate], now: DateTime<Utc>) -> Vec<RenewalPlan> {
        let mut plans = Vec::new();
        for cert in certs {
            let plan = match Self::evaluate(cert, now) {
                Some(p) => p,
                None => continue,
            };
            plans.push(plan);
        }
        plans.sort_by_key(|p| p.renew_at);
        plans
    }

    /// Single-cert evaluation — returns `None` if the cert is healthy
    /// and not yet due. Exposed for unit tests.
    pub fn evaluate(cert: &Certificate, now: DateTime<Utc>) -> Option<RenewalPlan> {
        let make = |reason: RenewalReason, renew_at: DateTime<Utc>| RenewalPlan {
            certificate_id: cert.id,
            name: cert.name.clone(),
            renew_at,
            reason,
        };
        let Some(status) = cert.status.as_ref() else {
            return Some(make(RenewalReason::InitialIssuance, now));
        };

        let is_ready = status
            .conditions
            .iter()
            .any(|c| c.kind == CertificateConditionType::Ready && c.status == ConditionStatus::True);
        if !is_ready {
            return Some(make(RenewalReason::NotReady, now));
        }

        let Some(not_after) = status.not_after else {
            return Some(make(RenewalReason::NotReady, now));
        };

        if now >= not_after {
            return Some(make(RenewalReason::Expired, now));
        }
        let renew_before = Duration::seconds(cert.spec.renew_before_seconds);
        let renew_at = not_after - renew_before;
        if now >= renew_at {
            return Some(make(RenewalReason::RenewBeforeReached, renew_at));
        }
        None
    }

    /// Convenience: when does cert-manager's `shouldReissue` next return
    /// true? Returns `None` for certs without a status.
    pub fn next_renewal_at(cert: &Certificate) -> Option<DateTime<Utc>> {
        let status = cert.status.as_ref()?;
        let not_after = status.not_after?;
        Some(not_after - Duration::seconds(cert.spec.renew_before_seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateCondition, CertificateConditionType, CertificateSpec,
        CertificateStatus, ConditionStatus, IssuerRef, IssuerRefKind, PrivateKeyPolicy,
        SecretRef,
    };
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn spec() -> CertificateSpec {
        CertificateSpec {
            secret_name: "tls".into(),
            issuer_ref: IssuerRef {
                name: "ca".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            dns_names: vec!["example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 90 * 24 * 3600,
            renew_before_seconds: 30 * 24 * 3600,
            usages: vec![],
            private_key: PrivateKeyPolicy::default(),
            is_ca: false,
            subject: None,
            secret_template_labels: BTreeMap::new(),
            secret_template_annotations: BTreeMap::new(),
        }
    }

    fn make_cert(
        not_after_offset_secs: Option<i64>,
        ready: bool,
        now: DateTime<Utc>,
    ) -> Certificate {
        let status = not_after_offset_secs.map(|offset| {
            let not_after = now + Duration::seconds(offset);
            let cond_status = if ready {
                ConditionStatus::True
            } else {
                ConditionStatus::False
            };
            CertificateStatus {
                conditions: vec![CertificateCondition {
                    kind: CertificateConditionType::Ready,
                    status: cond_status,
                    reason: None,
                    message: None,
                    last_transition_time: now,
                }],
                serial: Some("abc".into()),
                not_before: Some(now - Duration::seconds(60)),
                not_after: Some(not_after),
                renewal_time: Some(not_after - Duration::seconds(spec().renew_before_seconds)),
                revision: 1,
                last_failure_message: None,
                secret_ref: Some(SecretRef {
                    name: "tls".into(),
                    namespace: "default".into(),
                }),
            }
        });
        Certificate {
            id: Uuid::new_v4(),
            name: "demo".into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            spec: spec(),
            status,
            created_at: now,
            updated_at: now,
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
        }
    }

    #[test]
    fn cert_with_no_status_is_initial_issuance() {
        let now = Utc::now();
        let c = make_cert(None, false, now);
        let plan = RenewalScheduler::evaluate(&c, now).unwrap();
        assert_eq!(plan.reason, RenewalReason::InitialIssuance);
        assert_eq!(plan.renew_at, now);
    }

    #[test]
    fn ready_far_from_expiry_yields_no_plan() {
        let now = Utc::now();
        // 60 days remaining, renewBefore = 30 → not due yet.
        let c = make_cert(Some(60 * 24 * 3600), true, now);
        assert!(RenewalScheduler::evaluate(&c, now).is_none());
    }

    #[test]
    fn renew_before_reached_within_window() {
        let now = Utc::now();
        // 10 days remaining, renewBefore = 30 → past the threshold.
        let c = make_cert(Some(10 * 24 * 3600), true, now);
        let plan = RenewalScheduler::evaluate(&c, now).unwrap();
        assert_eq!(plan.reason, RenewalReason::RenewBeforeReached);
    }

    #[test]
    fn expired_cert_is_flagged_expired() {
        let now = Utc::now();
        // -10 days remaining → expired.
        let c = make_cert(Some(-10 * 24 * 3600), true, now);
        let plan = RenewalScheduler::evaluate(&c, now).unwrap();
        assert_eq!(plan.reason, RenewalReason::Expired);
    }

    #[test]
    fn not_ready_cert_retries_immediately() {
        let now = Utc::now();
        // ready=False status flips us into NotReady.
        let c = make_cert(Some(60 * 24 * 3600), false, now);
        let plan = RenewalScheduler::evaluate(&c, now).unwrap();
        assert_eq!(plan.reason, RenewalReason::NotReady);
    }

    #[test]
    fn plan_is_sorted_ascending_by_renew_at() {
        let now = Utc::now();
        let mut a = make_cert(Some(10 * 24 * 3600), true, now);
        a.name = "a".into();
        let mut b = make_cert(Some(40 * 24 * 3600), true, now);
        b.name = "b".into();
        let mut c = make_cert(Some(20 * 24 * 3600), true, now);
        c.name = "c".into();
        // b is far enough out that it should be skipped (60d > 30d renewBefore).
        let plans = RenewalScheduler.plan(&[a.clone(), b, c.clone()], now);
        // a and c should both be due — sorted by renew_at
        assert_eq!(plans.len(), 2);
        assert!(plans[0].renew_at <= plans[1].renew_at);
    }

    #[test]
    fn next_renewal_at_returns_not_after_minus_renew_before() {
        let now = Utc::now();
        let c = make_cert(Some(45 * 24 * 3600), true, now);
        let next = RenewalScheduler::next_renewal_at(&c).unwrap();
        // 45d - 30d = ~15d ahead
        let delta = (next - now).num_days();
        assert!((delta - 15).abs() <= 1, "delta={}", delta);
    }
}
