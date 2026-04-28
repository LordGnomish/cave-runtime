//! S3 / MinIO Object Lock — governance + compliance modes + legal hold.
//!
//! Reference: <https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html>.

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectLockMode {
    /// Governance — can be removed by users with `s3:BypassGovernanceRetention`.
    Governance,
    /// Compliance — even root cannot delete until `retain_until` passes.
    Compliance,
}

impl ObjectLockMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_uppercase().as_str() {
            "GOVERNANCE" => Self::Governance,
            "COMPLIANCE" => Self::Compliance,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Governance => "GOVERNANCE",
            Self::Compliance => "COMPLIANCE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectLockRetention {
    Governance,
    Compliance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockState {
    pub mode: ObjectLockMode,
    pub retain_until: DateTime<Utc>,
    pub legal_hold: bool,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LockError {
    #[error("retention still active until {0}")]
    RetentionActive(DateTime<Utc>),
    #[error("legal hold present")]
    LegalHold,
    #[error("compliance mode forbids modification")]
    ComplianceLocked,
    #[error("invalid mode: {0}")]
    InvalidMode(String),
}

impl LockState {
    pub fn new_governance(retain_until: DateTime<Utc>) -> Self {
        Self {
            mode: ObjectLockMode::Governance,
            retain_until,
            legal_hold: false,
        }
    }

    pub fn new_compliance(retain_until: DateTime<Utc>) -> Self {
        Self {
            mode: ObjectLockMode::Compliance,
            retain_until,
            legal_hold: false,
        }
    }

    pub fn with_legal_hold(mut self, hold: bool) -> Self {
        self.legal_hold = hold;
        self
    }

    /// Authorize a delete request given the lock state.
    pub fn authorize_delete(
        &self,
        now: DateTime<Utc>,
        bypass_governance: bool,
    ) -> Result<(), LockError> {
        if self.legal_hold {
            return Err(LockError::LegalHold);
        }
        if now < self.retain_until {
            match self.mode {
                ObjectLockMode::Compliance => return Err(LockError::ComplianceLocked),
                ObjectLockMode::Governance => {
                    if !bypass_governance {
                        return Err(LockError::RetentionActive(self.retain_until));
                    }
                }
            }
        }
        Ok(())
    }

    /// Extend retention (only allowed if new date is later than current).
    pub fn extend_retention(&mut self, new_until: DateTime<Utc>) -> Result<(), LockError> {
        if new_until <= self.retain_until {
            return Err(LockError::RetentionActive(self.retain_until));
        }
        self.retain_until = new_until;
        Ok(())
    }

    pub fn release_legal_hold(&mut self) {
        self.legal_hold = false;
    }

    pub fn place_legal_hold(&mut self) {
        self.legal_hold = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn future() -> DateTime<Utc> {
        Utc::now() + Duration::days(7)
    }

    fn past() -> DateTime<Utc> {
        Utc::now() - Duration::days(1)
    }

    #[test]
    fn parse_mode_uppercase() {
        assert_eq!(ObjectLockMode::parse("GOVERNANCE"), Some(ObjectLockMode::Governance));
        assert_eq!(ObjectLockMode::parse("COMPLIANCE"), Some(ObjectLockMode::Compliance));
    }

    #[test]
    fn parse_mode_lowercase() {
        assert_eq!(ObjectLockMode::parse("governance"), Some(ObjectLockMode::Governance));
    }

    #[test]
    fn parse_mode_invalid() {
        assert_eq!(ObjectLockMode::parse("WEAK"), None);
    }

    #[test]
    fn governance_blocks_delete_without_bypass() {
        let l = LockState::new_governance(future());
        let err = l.authorize_delete(Utc::now(), false).unwrap_err();
        assert!(matches!(err, LockError::RetentionActive(_)));
    }

    #[test]
    fn governance_allows_delete_with_bypass() {
        let l = LockState::new_governance(future());
        assert!(l.authorize_delete(Utc::now(), true).is_ok());
    }

    #[test]
    fn compliance_blocks_even_with_bypass() {
        let l = LockState::new_compliance(future());
        let err = l.authorize_delete(Utc::now(), true).unwrap_err();
        assert!(matches!(err, LockError::ComplianceLocked));
    }

    #[test]
    fn legal_hold_blocks_regardless_of_mode() {
        let l = LockState::new_governance(past()).with_legal_hold(true);
        assert!(l.authorize_delete(Utc::now(), true).is_err());
    }

    #[test]
    fn delete_allowed_after_retention_expires() {
        let l = LockState::new_compliance(past());
        assert!(l.authorize_delete(Utc::now(), false).is_ok());
    }

    #[test]
    fn extend_retention_allowed_when_later() {
        let mut l = LockState::new_governance(future());
        let later = future() + Duration::days(1);
        l.extend_retention(later).unwrap();
        assert_eq!(l.retain_until, later);
    }

    #[test]
    fn extend_retention_rejected_when_earlier() {
        let mut l = LockState::new_governance(future());
        let earlier = Utc::now();
        assert!(l.extend_retention(earlier).is_err());
    }

    #[test]
    fn place_then_release_legal_hold() {
        let mut l = LockState::new_governance(future());
        l.place_legal_hold();
        assert!(l.legal_hold);
        l.release_legal_hold();
        assert!(!l.legal_hold);
    }

    #[test]
    fn legal_hold_persists_after_retention_expires() {
        let mut l = LockState::new_governance(past());
        l.place_legal_hold();
        let err = l.authorize_delete(Utc::now(), true).unwrap_err();
        assert_eq!(err, LockError::LegalHold);
    }

    #[test]
    fn mode_str_round_trip() {
        for m in [ObjectLockMode::Governance, ObjectLockMode::Compliance] {
            assert_eq!(ObjectLockMode::parse(m.as_str()), Some(m));
        }
    }
}
