// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finding lifecycle state machine — DefectDojo-parity boolean flags
//! (active / verified / false_p / duplicate / risk_accepted /
//!  out_of_scope / is_mitigated / under_review).
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:2397
//!         (`Finding.active`, `.verified`, `.false_p`, `.duplicate`,
//!          `.risk_accepted`, `.out_of_scope`, `.is_mitigated`,
//!          `.under_review`) and dojo/finding/helper.py
//!         (`set_active`, `set_verified`, `risk_acceptance.add_findings_to_risk_acceptance`).

use serde::{Deserialize, Serialize};

/// All triage flags carried on a Finding. Multiple can be true
/// simultaneously (e.g. `risk_accepted=true` ⇒ `active=false`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FindingState {
    pub active: bool,
    pub verified: bool,
    pub false_p: bool,
    pub duplicate: bool,
    pub risk_accepted: bool,
    pub out_of_scope: bool,
    pub is_mitigated: bool,
    pub under_review: bool,
    /// Free-form actor that last touched state. Mirrors
    /// `Finding.last_status_update`'s implicit owner.
    pub last_actor: Option<String>,
}

impl FindingState {
    /// New finding default: active, not verified.
    /// Source: `Finding._meta.get_field('active').default = True`
    ///         (overridden to True in `Finding.__init__` for fresh imports).
    pub fn fresh() -> Self {
        Self {
            active: true,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateTransition {
    /// Manual triage: confirmed by a human.
    Verify,
    /// Reopen a previously-closed finding (e.g. SLA-expired risk acceptance).
    Reactivate,
    /// Permanent: vendor patched / cred rotated. Sets `is_mitigated=true`,
    /// `active=false`.
    Mitigate,
    /// Manual triage: not a real flaw.
    MarkFalsePositive,
    /// Cross-scanner duplicate (set by dedup pipeline).
    MarkDuplicate,
    /// Out of audit scope.
    MarkOutOfScope,
    /// Risk-accepted — moves into a Risk_Acceptance object.
    RiskAccept,
    /// Risk-acceptance expired / withdrawn — reactivates the finding.
    RiskUnaccept,
    /// Pending peer review.
    SubmitForReview,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StateError {
    #[error("cannot {0:?} a finding that is already mitigated")]
    AlreadyMitigated(StateTransition),
    #[error("cannot {0:?} a duplicate finding")]
    DuplicateLocked(StateTransition),
    #[error("cannot reactivate a finding that is already active")]
    AlreadyActive,
}

impl FindingState {
    /// Apply a transition. Returns the new state; the original is
    /// unchanged (immutable apply, mirrors event-sourcing patterns).
    pub fn apply(&self, t: StateTransition, actor: &str) -> Result<Self, StateError> {
        let mut next = self.clone();
        next.last_actor = Some(actor.into());
        match t {
            StateTransition::Verify => {
                if self.is_mitigated {
                    return Err(StateError::AlreadyMitigated(t));
                }
                next.verified = true;
            }
            StateTransition::Reactivate => {
                if self.active && !self.is_mitigated && !self.risk_accepted {
                    return Err(StateError::AlreadyActive);
                }
                next.active = true;
                next.is_mitigated = false;
                next.risk_accepted = false;
                next.false_p = false;
                next.out_of_scope = false;
            }
            StateTransition::Mitigate => {
                if self.duplicate {
                    return Err(StateError::DuplicateLocked(t));
                }
                next.is_mitigated = true;
                next.active = false;
            }
            StateTransition::MarkFalsePositive => {
                next.false_p = true;
                next.active = false;
                next.verified = false;
            }
            StateTransition::MarkDuplicate => {
                next.duplicate = true;
                next.active = false;
            }
            StateTransition::MarkOutOfScope => {
                next.out_of_scope = true;
                next.active = false;
            }
            StateTransition::RiskAccept => {
                if self.duplicate {
                    return Err(StateError::DuplicateLocked(t));
                }
                next.risk_accepted = true;
                next.active = false;
            }
            StateTransition::RiskUnaccept => {
                next.risk_accepted = false;
                next.active = true;
            }
            StateTransition::SubmitForReview => {
                next.under_review = true;
            }
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_finding_is_active_only() {
        let s = FindingState::fresh();
        assert!(s.active);
        assert!(!s.verified);
        assert!(!s.is_mitigated);
        assert!(!s.duplicate);
    }

    #[test]
    fn verify_sets_verified_flag() {
        let s = FindingState::fresh()
            .apply(StateTransition::Verify, "alice")
            .unwrap();
        assert!(s.verified);
        assert!(s.active);
        assert_eq!(s.last_actor.as_deref(), Some("alice"));
    }

    #[test]
    fn mitigate_clears_active_and_sets_mitigated() {
        let s = FindingState::fresh()
            .apply(StateTransition::Mitigate, "bob")
            .unwrap();
        assert!(s.is_mitigated);
        assert!(!s.active);
    }

    #[test]
    fn cannot_verify_mitigated_finding() {
        let s = FindingState::fresh()
            .apply(StateTransition::Mitigate, "bob")
            .unwrap();
        let err = s.apply(StateTransition::Verify, "alice").unwrap_err();
        assert_eq!(err, StateError::AlreadyMitigated(StateTransition::Verify));
    }

    #[test]
    fn cannot_mitigate_duplicate() {
        let s = FindingState::fresh()
            .apply(StateTransition::MarkDuplicate, "x")
            .unwrap();
        let err = s.apply(StateTransition::Mitigate, "x").unwrap_err();
        assert_eq!(err, StateError::DuplicateLocked(StateTransition::Mitigate));
    }

    #[test]
    fn risk_accept_clears_active() {
        let s = FindingState::fresh()
            .apply(StateTransition::RiskAccept, "ciso")
            .unwrap();
        assert!(s.risk_accepted);
        assert!(!s.active);
    }

    #[test]
    fn risk_unaccept_reactivates() {
        let s = FindingState::fresh()
            .apply(StateTransition::RiskAccept, "ciso")
            .unwrap()
            .apply(StateTransition::RiskUnaccept, "auto")
            .unwrap();
        assert!(!s.risk_accepted);
        assert!(s.active);
    }

    #[test]
    fn false_positive_sets_false_p_and_clears_active() {
        let s = FindingState::fresh()
            .apply(StateTransition::MarkFalsePositive, "u")
            .unwrap();
        assert!(s.false_p);
        assert!(!s.active);
    }

    #[test]
    fn reactivate_after_mitigation_works() {
        let s = FindingState::fresh()
            .apply(StateTransition::Mitigate, "x")
            .unwrap()
            .apply(StateTransition::Reactivate, "y")
            .unwrap();
        assert!(s.active);
        assert!(!s.is_mitigated);
    }

    #[test]
    fn cannot_reactivate_already_active_finding() {
        let err = FindingState::fresh()
            .apply(StateTransition::Reactivate, "x")
            .unwrap_err();
        assert_eq!(err, StateError::AlreadyActive);
    }
}
