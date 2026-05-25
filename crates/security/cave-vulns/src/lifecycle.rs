// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finding lifecycle workflows — compound state transitions and audit trail.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/finding/helper.py
//!         (`accept_finding`, `mark_false_positive`, …) and
//!         dojo/risk_acceptance/helper.py — the upstream helpers run as
//!         imperative scripts that mutate Finding + emit audit records.
//!
//! cave-vulns ports the workflow shape (not the Django ORM): each helper
//! drives a `Finding` through one or more `StateTransition`s and returns
//! an `AuditEntry` that the caller persists in cave-db.

use crate::finding::{Finding, StateError, StateTransition};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub finding_id: uuid::Uuid,
    pub actor: String,
    pub action: AuditAction,
    pub note: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    Accepted,
    FalsePositive,
    Mitigated,
    Reactivated,
    Verified,
    OutOfScope,
    SubmittedForReview,
}

/// Accept the risk on a finding. Mirrors `dojo/risk_acceptance/helper.py
/// ::add_findings_to_risk_acceptance` — flips `risk_accepted=true` and
/// `active=false`.
pub fn accept_risk(
    f: &mut Finding,
    actor: &str,
    note: Option<&str>,
) -> Result<AuditEntry, StateError> {
    f.transition(StateTransition::RiskAccept, actor)?;
    Ok(AuditEntry {
        finding_id: f.id,
        actor: actor.to_string(),
        action: AuditAction::Accepted,
        note: note.map(String::from),
        at: Utc::now(),
    })
}

/// Mark a finding as a false positive. Mirrors
/// `dojo/finding/helper.py::set_false_p` — flips `false_p=true`,
/// `active=false`.
pub fn mark_false_positive(
    f: &mut Finding,
    actor: &str,
    note: Option<&str>,
) -> Result<AuditEntry, StateError> {
    f.transition(StateTransition::MarkFalsePositive, actor)?;
    Ok(AuditEntry {
        finding_id: f.id,
        actor: actor.to_string(),
        action: AuditAction::FalsePositive,
        note: note.map(String::from),
        at: Utc::now(),
    })
}

/// Mark the finding mitigated (patched / cred rotated). Sets
/// `is_mitigated=true`, `mitigated=now()`.
pub fn mitigate(
    f: &mut Finding,
    actor: &str,
    note: Option<&str>,
) -> Result<AuditEntry, StateError> {
    f.transition(StateTransition::Mitigate, actor)?;
    Ok(AuditEntry {
        finding_id: f.id,
        actor: actor.to_string(),
        action: AuditAction::Mitigated,
        note: note.map(String::from),
        at: Utc::now(),
    })
}

/// Reopen a mitigated/risk-accepted finding.
pub fn reactivate(
    f: &mut Finding,
    actor: &str,
    note: Option<&str>,
) -> Result<AuditEntry, StateError> {
    f.transition(StateTransition::Reactivate, actor)?;
    Ok(AuditEntry {
        finding_id: f.id,
        actor: actor.to_string(),
        action: AuditAction::Reactivated,
        note: note.map(String::from),
        at: Utc::now(),
    })
}

/// Mark the finding "out of scope" — won't count against SLA but stays in
/// the record.
pub fn mark_out_of_scope(
    f: &mut Finding,
    actor: &str,
    note: Option<&str>,
) -> Result<AuditEntry, StateError> {
    f.transition(StateTransition::MarkOutOfScope, actor)?;
    Ok(AuditEntry {
        finding_id: f.id,
        actor: actor.to_string(),
        action: AuditAction::OutOfScope,
        note: note.map(String::from),
        at: Utc::now(),
    })
}

/// Confirm a finding's authenticity.
pub fn verify(f: &mut Finding, actor: &str) -> Result<AuditEntry, StateError> {
    f.transition(StateTransition::Verify, actor)?;
    Ok(AuditEntry {
        finding_id: f.id,
        actor: actor.to_string(),
        action: AuditAction::Verified,
        note: None,
        at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Finding, FindingSeverity};

    fn fixture() -> Finding {
        Finding::new("XSS in login", FindingSeverity::High)
    }

    #[test]
    fn accept_risk_marks_inactive() {
        let mut f = fixture();
        let audit = accept_risk(&mut f, "alice", Some("compensating control")).unwrap();
        assert!(f.state.risk_accepted);
        assert!(!f.state.active);
        assert_eq!(audit.action, AuditAction::Accepted);
        assert_eq!(audit.actor, "alice");
    }

    #[test]
    fn mark_false_positive_flow() {
        let mut f = fixture();
        let audit = mark_false_positive(&mut f, "bob", Some("noise")).unwrap();
        assert!(f.state.false_p);
        assert!(!f.state.active);
        assert_eq!(audit.action, AuditAction::FalsePositive);
    }

    #[test]
    fn mitigate_sets_timestamp() {
        let mut f = fixture();
        let audit = mitigate(&mut f, "carol", None).unwrap();
        assert!(f.state.is_mitigated);
        assert!(f.mitigated.is_some());
        assert_eq!(audit.action, AuditAction::Mitigated);
    }

    #[test]
    fn mitigate_then_reactivate() {
        let mut f = fixture();
        mitigate(&mut f, "carol", None).unwrap();
        let r = reactivate(&mut f, "dave", Some("not actually fixed"));
        assert!(r.is_ok(), "expected reactivate to succeed: {:?}", r.err());
        assert!(f.state.active);
        assert!(!f.state.is_mitigated);
    }

    #[test]
    fn verify_after_mitigate_fails() {
        // DefectDojo's state machine forbids verifying a mitigated finding;
        // this exercise pins that the lifecycle helpers preserve the
        // underlying state-machine guard rather than papering over it.
        let mut f = fixture();
        mitigate(&mut f, "x", None).unwrap();
        let err = verify(&mut f, "y").unwrap_err();
        assert!(matches!(err, StateError::AlreadyMitigated(_)));
    }

    #[test]
    fn mark_out_of_scope_workflow() {
        let mut f = fixture();
        let audit = mark_out_of_scope(&mut f, "alice", None).unwrap();
        assert!(f.state.out_of_scope);
        assert_eq!(audit.action, AuditAction::OutOfScope);
    }

    #[test]
    fn verify_workflow() {
        let mut f = fixture();
        let audit = verify(&mut f, "alice").unwrap();
        assert!(f.state.verified);
        assert_eq!(audit.action, AuditAction::Verified);
    }

    #[test]
    fn audit_entry_serialization_roundtrip() {
        let mut f = fixture();
        let audit = accept_risk(&mut f, "alice", Some("compensating")).unwrap();
        let j = serde_json::to_string(&audit).unwrap();
        let back: AuditEntry = serde_json::from_str(&j).unwrap();
        assert_eq!(back, audit);
    }

    #[test]
    fn audit_action_serde_snake_case() {
        let j = serde_json::to_string(&AuditAction::FalsePositive).unwrap();
        assert_eq!(j, "\"false_positive\"");
    }
}
