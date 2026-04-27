//! PKI issuer chain + CRL state.
//!
//! Tracks a forest of issuer certs (root → intermediate → leaf), serial-number
//! issuance, and the CRL (revoked serial set with `revoked_at`).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PkiError {
    #[error("issuer not found: {0}")]
    IssuerNotFound(String),
    #[error("certificate not found: {0}")]
    CertNotFound(String),
    #[error("certificate already revoked: {0}")]
    AlreadyRevoked(String),
    #[error("issuer chain cycle detected")]
    Cycle,
    #[error("expired before now: {0}")]
    Expired(DateTime<Utc>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cert {
    pub serial: String,
    pub subject: String,
    pub issuer: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub is_ca: bool,
}

#[derive(Debug, Clone)]
pub struct RevocationEntry {
    pub serial: String,
    pub revoked_at: DateTime<Utc>,
    pub reason: RevocationReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationReason {
    Unspecified = 0,
    KeyCompromise = 1,
    CACompromise = 2,
    AffiliationChanged = 3,
    Superseded = 4,
    CessationOfOperation = 5,
}

#[derive(Debug, Default)]
pub struct PkiChain {
    pub certs: BTreeMap<String, Cert>,
    pub revocations: BTreeMap<String, RevocationEntry>,
    pub serial_counter: u64,
}

impl PkiChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_serial(&mut self) -> String {
        self.serial_counter += 1;
        format!("{:032x}", self.serial_counter)
    }

    pub fn issue(
        &mut self,
        subject: &str,
        issuer: &str,
        not_before: DateTime<Utc>,
        not_after: DateTime<Utc>,
        is_ca: bool,
    ) -> Result<Cert, PkiError> {
        // Issuer must exist if it's not the root self-signed cert.
        if subject != issuer && !self.certs.values().any(|c| c.subject == issuer) {
            return Err(PkiError::IssuerNotFound(issuer.into()));
        }
        if not_after <= not_before {
            return Err(PkiError::Expired(not_after));
        }
        let serial = self.next_serial();
        let cert = Cert {
            serial: serial.clone(),
            subject: subject.into(),
            issuer: issuer.into(),
            not_before,
            not_after,
            is_ca,
        };
        self.certs.insert(serial, cert.clone());
        Ok(cert)
    }

    pub fn revoke(
        &mut self,
        serial: &str,
        reason: RevocationReason,
        now: DateTime<Utc>,
    ) -> Result<(), PkiError> {
        if !self.certs.contains_key(serial) {
            return Err(PkiError::CertNotFound(serial.into()));
        }
        if self.revocations.contains_key(serial) {
            return Err(PkiError::AlreadyRevoked(serial.into()));
        }
        self.revocations.insert(
            serial.into(),
            RevocationEntry {
                serial: serial.into(),
                revoked_at: now,
                reason,
            },
        );
        Ok(())
    }

    pub fn is_revoked(&self, serial: &str) -> bool {
        self.revocations.contains_key(serial)
    }

    /// Walk from `subject` up to a self-signed root. Returns `Cycle` if a
    /// loop is detected.
    pub fn chain(&self, serial: &str) -> Result<Vec<Cert>, PkiError> {
        let mut out = Vec::new();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut current = self
            .certs
            .get(serial)
            .ok_or_else(|| PkiError::CertNotFound(serial.into()))?
            .clone();
        loop {
            if !visited.insert(current.serial.clone()) {
                return Err(PkiError::Cycle);
            }
            let is_root = current.subject == current.issuer;
            out.push(current.clone());
            if is_root {
                break;
            }
            // Find issuer: the cert whose subject == current.issuer.
            let issuer = self
                .certs
                .values()
                .find(|c| c.subject == current.issuer);
            match issuer {
                Some(c) => current = c.clone(),
                None => return Err(PkiError::IssuerNotFound(current.issuer.clone())),
            }
        }
        Ok(out)
    }

    pub fn validate(&self, serial: &str, now: DateTime<Utc>) -> Result<(), PkiError> {
        let chain = self.chain(serial)?;
        for c in &chain {
            if now < c.not_before || now > c.not_after {
                return Err(PkiError::Expired(c.not_after));
            }
            if self.is_revoked(&c.serial) {
                return Err(PkiError::AlreadyRevoked(c.serial.clone()));
            }
        }
        Ok(())
    }

    pub fn crl_serials(&self) -> Vec<String> {
        self.revocations.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn next_serial_is_monotonic() {
        let mut c = PkiChain::new();
        let s1 = c.next_serial();
        let s2 = c.next_serial();
        assert_ne!(s1, s2);
    }

    #[test]
    fn issue_self_signed_root() {
        let mut c = PkiChain::new();
        let r = c
            .issue("CN=root", "CN=root", t(2025, 1, 1), t(2030, 1, 1), true)
            .unwrap();
        assert_eq!(r.subject, r.issuer);
    }

    #[test]
    fn issue_intermediate_requires_issuer() {
        let mut c = PkiChain::new();
        let err = c
            .issue("CN=int", "CN=missing", t(2025, 1, 1), t(2026, 1, 1), true)
            .unwrap_err();
        assert!(matches!(err, PkiError::IssuerNotFound(_)));
    }

    #[test]
    fn issue_with_invalid_validity_errors() {
        let mut c = PkiChain::new();
        let err = c
            .issue("CN=x", "CN=x", t(2026, 1, 1), t(2025, 1, 1), true)
            .unwrap_err();
        assert!(matches!(err, PkiError::Expired(_)));
    }

    #[test]
    fn revoke_marks_cert() {
        let mut c = PkiChain::new();
        let cert = c.issue("CN=x", "CN=x", t(2025, 1, 1), t(2030, 1, 1), false).unwrap();
        c.revoke(&cert.serial, RevocationReason::KeyCompromise, t(2025, 6, 1)).unwrap();
        assert!(c.is_revoked(&cert.serial));
    }

    #[test]
    fn revoke_unknown_errors() {
        let mut c = PkiChain::new();
        let err = c.revoke("nope", RevocationReason::Unspecified, t(2025, 6, 1)).unwrap_err();
        assert!(matches!(err, PkiError::CertNotFound(_)));
    }

    #[test]
    fn revoke_twice_errors() {
        let mut c = PkiChain::new();
        let cert = c.issue("CN=x", "CN=x", t(2025, 1, 1), t(2030, 1, 1), false).unwrap();
        c.revoke(&cert.serial, RevocationReason::Unspecified, t(2025, 6, 1)).unwrap();
        let err = c
            .revoke(&cert.serial, RevocationReason::Unspecified, t(2025, 7, 1))
            .unwrap_err();
        assert!(matches!(err, PkiError::AlreadyRevoked(_)));
    }

    #[test]
    fn chain_contains_root() {
        let mut c = PkiChain::new();
        let root = c.issue("CN=root", "CN=root", t(2025, 1, 1), t(2030, 1, 1), true).unwrap();
        let int_ = c.issue("CN=int", "CN=root", t(2025, 1, 1), t(2030, 1, 1), true).unwrap();
        let leaf = c.issue("CN=leaf", "CN=int", t(2025, 1, 1), t(2026, 1, 1), false).unwrap();
        let chain = c.chain(&leaf.serial).unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].subject, "CN=leaf");
        assert_eq!(chain[2].serial, root.serial);
        assert_eq!(int_.subject, "CN=int");
    }

    #[test]
    fn validate_within_validity_succeeds() {
        let mut c = PkiChain::new();
        c.issue("CN=root", "CN=root", t(2025, 1, 1), t(2030, 1, 1), true).unwrap();
        let leaf = c.issue("CN=leaf", "CN=root", t(2025, 1, 1), t(2026, 1, 1), false).unwrap();
        assert!(c.validate(&leaf.serial, t(2025, 6, 1)).is_ok());
    }

    #[test]
    fn validate_expired_errors() {
        let mut c = PkiChain::new();
        c.issue("CN=root", "CN=root", t(2025, 1, 1), t(2030, 1, 1), true).unwrap();
        let leaf = c.issue("CN=leaf", "CN=root", t(2025, 1, 1), t(2026, 1, 1), false).unwrap();
        let err = c.validate(&leaf.serial, t(2027, 6, 1)).unwrap_err();
        assert!(matches!(err, PkiError::Expired(_)));
    }

    #[test]
    fn validate_revoked_errors() {
        let mut c = PkiChain::new();
        c.issue("CN=root", "CN=root", t(2025, 1, 1), t(2030, 1, 1), true).unwrap();
        let leaf = c.issue("CN=leaf", "CN=root", t(2025, 1, 1), t(2026, 1, 1), false).unwrap();
        c.revoke(&leaf.serial, RevocationReason::KeyCompromise, t(2025, 6, 1)).unwrap();
        let err = c.validate(&leaf.serial, t(2025, 7, 1)).unwrap_err();
        assert!(matches!(err, PkiError::AlreadyRevoked(_)));
    }

    #[test]
    fn crl_serials_lists_all_revoked() {
        let mut c = PkiChain::new();
        let r = c.issue("CN=r", "CN=r", t(2025, 1, 1), t(2030, 1, 1), true).unwrap();
        let l = c.issue("CN=l", "CN=r", t(2025, 1, 1), t(2026, 1, 1), false).unwrap();
        c.revoke(&l.serial, RevocationReason::Unspecified, t(2025, 6, 1)).unwrap();
        let crl = c.crl_serials();
        assert_eq!(crl, vec![l.serial.clone()]);
        assert!(!c.is_revoked(&r.serial));
    }

    #[test]
    fn revocation_reason_codes_match_x509() {
        // RFC 5280 §5.3.1 reasonCode values.
        assert_eq!(RevocationReason::Unspecified as i32, 0);
        assert_eq!(RevocationReason::KeyCompromise as i32, 1);
        assert_eq!(RevocationReason::CACompromise as i32, 2);
    }

    #[test]
    fn chain_unknown_serial_errors() {
        let c = PkiChain::new();
        let err = c.chain("nope").unwrap_err();
        assert!(matches!(err, PkiError::CertNotFound(_)));
    }

    #[test]
    fn duration_helper_unused_warning_silenced() {
        let _ = Duration::seconds(0);
    }
}
