// SPDX-License-Identifier: AGPL-3.0-or-later
//! Risk acceptance workflow — expiration, reactivation, approvers.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:3942
//!         (`class Risk_Acceptance`), dojo/risk_acceptance/helper.py
//!         (`expire_now`, `reinstate`).

use crate::finding::{Finding, StateTransition};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Source: dojo/models.py:3943-3954 (`TREATMENT_*` constants).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Treatment {
    /// "A" — Accept (acknowledged, remains)
    Accept,
    /// "V" — Avoid (do not engage with what creates the risk)
    Avoid,
    /// "M" — Mitigate (compensating controls in place)
    Mitigate,
    /// "F" — Fix (eradicated)
    Fix,
    /// "T" — Transfer (to a 3rd party)
    Transfer,
}

impl Treatment {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Accept => "A",
            Self::Avoid => "V",
            Self::Mitigate => "M",
            Self::Fix => "F",
            Self::Transfer => "T",
        }
    }
}

/// Source: dojo/models.py:3942 (`class Risk_Acceptance`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskAcceptance {
    pub id: Uuid,
    pub name: String,
    pub accepted_finding_ids: Vec<Uuid>,
    pub recommendation: Treatment,
    pub recommendation_details: Option<String>,
    pub decision: Treatment,
    pub decision_details: Option<String>,
    pub accepted_by: Option<String>,
    pub owner: String,
    pub expiration_date: Option<DateTime<Utc>>,
    pub expiration_date_warned: Option<DateTime<Utc>>,
    pub expiration_date_handled: Option<DateTime<Utc>>,
    pub reactivate_expired: bool,
    pub restart_sla_expired: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

impl RiskAcceptance {
    pub fn new(name: impl Into<String>, owner: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            accepted_finding_ids: Vec::new(),
            recommendation: Treatment::Fix,
            recommendation_details: None,
            decision: Treatment::Accept,
            decision_details: None,
            accepted_by: None,
            owner: owner.into(),
            expiration_date: None,
            expiration_date_warned: None,
            expiration_date_handled: None,
            reactivate_expired: true,
            restart_sla_expired: false,
            created: now,
            updated: now,
        }
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expiration_date.map_or(false, |d| now > d)
    }

    /// Days until expiration. Negative when expired. None when no
    /// expiration set. Source: dojo/risk_acceptance/helper.py.
    pub fn days_until_expiration(&self, now: DateTime<Utc>) -> Option<i64> {
        self.expiration_date.map(|d| (d - now).num_days())
    }
}

/// Add findings to a RiskAcceptance — flips their state to
/// `risk_accepted=true, active=false`. Source: dojo/risk_acceptance/
/// helper.py::add_findings_to_risk_acceptance.
pub fn add_findings(
    ra: &mut RiskAcceptance,
    findings: &mut [Finding],
    actor: &str,
) -> Result<(), crate::finding::StateError> {
    for f in findings.iter_mut() {
        if !ra.accepted_finding_ids.contains(&f.id) {
            ra.accepted_finding_ids.push(f.id);
        }
        f.transition(StateTransition::RiskAccept, actor)?;
    }
    ra.updated = Utc::now();
    Ok(())
}

/// Expire a RiskAcceptance — reactivates findings if
/// `ra.reactivate_expired`. Source: dojo/risk_acceptance/helper.py::
/// reinstate_risk_acceptance.
pub fn expire(
    ra: &mut RiskAcceptance,
    findings: &mut [Finding],
    actor: &str,
) -> Result<usize, crate::finding::StateError> {
    let now = Utc::now();
    ra.expiration_date_handled = Some(now);
    if !ra.reactivate_expired {
        return Ok(0);
    }
    let mut reactivated = 0;
    for f in findings.iter_mut() {
        if ra.accepted_finding_ids.contains(&f.id) && f.state.risk_accepted {
            f.transition(StateTransition::RiskUnaccept, actor)?;
            if ra.restart_sla_expired {
                f.date = now;
            }
            reactivated += 1;
        }
    }
    Ok(reactivated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::FindingSeverity;
    use chrono::Duration;

    fn finding() -> Finding {
        Finding::new("X", FindingSeverity::High)
    }

    #[test]
    fn treatment_codes_match_defectdojo() {
        assert_eq!(Treatment::Accept.code(), "A");
        assert_eq!(Treatment::Avoid.code(), "V");
        assert_eq!(Treatment::Mitigate.code(), "M");
        assert_eq!(Treatment::Fix.code(), "F");
        assert_eq!(Treatment::Transfer.code(), "T");
    }

    #[test]
    fn risk_acceptance_defaults_reactivate_true() {
        let ra = RiskAcceptance::new("Q3 review", "alice");
        assert!(ra.reactivate_expired);
        assert!(!ra.restart_sla_expired);
        assert_eq!(ra.owner, "alice");
        assert_eq!(ra.recommendation, Treatment::Fix);
        assert_eq!(ra.decision, Treatment::Accept);
    }

    #[test]
    fn expiration_check_before_after() {
        let mut ra = RiskAcceptance::new("x", "alice");
        ra.expiration_date = Some(Utc::now() + Duration::days(7));
        assert!(!ra.is_expired(Utc::now()));
        assert!(ra.is_expired(Utc::now() + Duration::days(8)));
    }

    #[test]
    fn days_until_expiration_positive_then_negative() {
        let mut ra = RiskAcceptance::new("x", "alice");
        let now = Utc::now();
        ra.expiration_date = Some(now + Duration::days(10));
        assert_eq!(ra.days_until_expiration(now), Some(10));
        assert_eq!(ra.days_until_expiration(now + Duration::days(15)), Some(-5));
    }

    #[test]
    fn add_findings_sets_risk_accepted_flag() {
        let mut ra = RiskAcceptance::new("x", "alice");
        let f1 = finding();
        let f2 = finding();
        let mut findings = vec![f1.clone(), f2.clone()];
        add_findings(&mut ra, &mut findings, "ciso").unwrap();
        assert_eq!(ra.accepted_finding_ids.len(), 2);
        assert!(findings[0].state.risk_accepted);
        assert!(findings[1].state.risk_accepted);
        assert!(!findings[0].state.active);
        let _ = (f1, f2);
    }

    #[test]
    fn expire_reactivates_findings_when_flag_set() {
        let mut ra = RiskAcceptance::new("x", "alice");
        let mut findings = vec![finding(), finding()];
        add_findings(&mut ra, &mut findings, "ciso").unwrap();
        let n = expire(&mut ra, &mut findings, "auto").unwrap();
        assert_eq!(n, 2);
        assert!(findings[0].state.active);
        assert!(findings[1].state.active);
        assert!(ra.expiration_date_handled.is_some());
    }

    #[test]
    fn expire_does_not_reactivate_when_flag_clear() {
        let mut ra = RiskAcceptance::new("x", "alice");
        ra.reactivate_expired = false;
        let mut findings = vec![finding()];
        add_findings(&mut ra, &mut findings, "ciso").unwrap();
        let n = expire(&mut ra, &mut findings, "auto").unwrap();
        assert_eq!(n, 0);
        assert!(!findings[0].state.active);
        assert!(findings[0].state.risk_accepted);
    }

    #[test]
    fn restart_sla_resets_finding_date() {
        let mut ra = RiskAcceptance::new("x", "alice");
        ra.restart_sla_expired = true;
        let original_date = Utc::now() - Duration::days(100);
        let mut f = finding();
        f.date = original_date;
        let mut findings = vec![f];
        add_findings(&mut ra, &mut findings, "x").unwrap();
        expire(&mut ra, &mut findings, "auto").unwrap();
        assert!(findings[0].date > original_date);
    }
}
