//! Renewal controller — fires when the renewal window opens.
//!
//! Cite: cert-manager v1.20.2
//! `pkg/controller/certificates/trigger/trigger_controller.go::shouldReissue`
//! — the trigger is `now >= notAfter - renewBefore`.

use crate::crds::{renewal_due_at, CertificateSpec, CertificateStatus};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewalDecision {
    /// Cert is fresh — wait until `next_eval_at`.
    NotYet { next_eval_at: DateTime<Utc> },
    /// `now >= notAfter - renewBefore` — kick off a new ACME order.
    Due,
    /// Cert is past `notAfter`. Cite: cert-manager
    /// `pkg/controller/certificates/trigger` ExpiringSoon-vs-Expired
    /// distinction; cave reports `Expired` separately so audit can
    /// page on past-due workloads.
    Expired,
    /// Cert never had a notAfter populated — first issuance is needed.
    NeedsInitialIssuance,
}

#[derive(Debug, Default)]
pub struct RenewalController {
    pub tenant_id: String,
}

impl RenewalController {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into() }
    }

    /// Cite: cert-manager
    /// `pkg/controller/certificates/trigger::shouldReissue`.
    pub fn evaluate(
        &self,
        spec: &CertificateSpec,
        status: &CertificateStatus,
        now: DateTime<Utc>,
    ) -> RenewalDecision {
        if spec.tenant_id != self.tenant_id {
            // Cross-tenant evaluations are no-ops in this scaffold.
            return RenewalDecision::NotYet { next_eval_at: now };
        }
        let Some(not_after) = status.not_after else {
            return RenewalDecision::NeedsInitialIssuance;
        };
        if now >= not_after {
            return RenewalDecision::Expired;
        }
        let Some(due_at) = renewal_due_at(spec, status) else {
            return RenewalDecision::NeedsInitialIssuance;
        };
        if now >= due_at {
            RenewalDecision::Due
        } else {
            RenewalDecision::NotYet { next_eval_at: due_at }
        }
    }
}
