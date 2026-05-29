// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CertificateRequest CRD — carries a PEM CSR, is approved/denied by an
//! admission controller, then issued by the matching issuer.
//!
//! Cite: cert-manager v1.13.0
//! `pkg/apis/certmanager/v1/types_certificaterequest.go`.
//! The state machine: `Pending → Approved → Issued`
//!                    `Pending → Denied`.

use crate::crds::IssuerRef;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Cite: cert-manager CertificateRequest.status conditions + state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateRequestState {
    /// Waiting for approval or direct issuance.
    Pending,
    /// Approved by an approval policy — issuer may now sign.
    Approved,
    /// Denied by an approval policy — terminal, not re-issuable.
    Denied,
    /// Successfully issued: `certificate_pem` is populated.
    Issued,
}

/// Cite: cert-manager `CertificateRequest.spec.denialReason` (cave enum).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenialReason {
    PolicyViolation,
    InvalidCsr,
    IssuerUnavailable,
    Other,
}

impl DenialReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PolicyViolation => "PolicyViolation",
            Self::InvalidCsr => "InvalidCsr",
            Self::IssuerUnavailable => "IssuerUnavailable",
            Self::Other => "Other",
        }
    }
}

/// Cite: cert-manager `CertificateRequestSpec` — the immutable desired state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateRequestSpec {
    /// Cave multi-tenant boundary.
    pub tenant_id: String,
    /// Cite: cert-manager `CertificateRequest.spec.issuerRef`.
    pub issuer_ref: IssuerRef,
    /// Cite: cert-manager `CertificateRequest.spec.request` — base64 or PEM
    /// PKCS#10 CSR. cave stores as PEM string.
    pub csr_pem: String,
    /// Cite: cert-manager `CertificateRequest.spec.isCA` — request a CA cert.
    pub is_ca: bool,
    /// Cite: cert-manager `CertificateRequest.spec.duration` (optional).
    pub duration_seconds: Option<i64>,
    /// Cite: cert-manager `CertificateRequest.spec.usages`.
    pub usages: Vec<String>,
}

impl CertificateRequestSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.tenant_id.trim().is_empty() {
            return Err("tenant_id must be non-empty".into());
        }
        if self.csr_pem.trim().is_empty() {
            return Err("csr_pem must be non-empty".into());
        }
        if let Some(dur) = self.duration_seconds {
            if dur <= 0 {
                return Err("duration_seconds must be > 0".into());
            }
        }
        Ok(())
    }
}

/// Cite: cert-manager CertificateRequest resource — mutable status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateRequest {
    pub id: Uuid,
    pub tenant_id: String,
    pub spec: CertificateRequestSpec,
    pub state: CertificateRequestState,
    /// Populated once the issuer signs the request.
    pub certificate_pem: Option<String>,
    /// Populated if the request is denied or issuance fails.
    pub failure_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CertificateRequest {
    /// Create a new `CertificateRequest` in `Pending` state.
    pub fn new(tenant_id: impl Into<String>, spec: CertificateRequestSpec) -> Self {
        let now = Utc::now();
        let tenant_id = tenant_id.into();
        Self {
            id: Uuid::new_v4(),
            tenant_id,
            spec,
            state: CertificateRequestState::Pending,
            certificate_pem: None,
            failure_message: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Cite: cert-manager approval controller — Pending → Approved.
    /// Idempotent: calling on an already-Approved request is a no-op.
    pub fn approve(&mut self) {
        if self.state == CertificateRequestState::Pending {
            self.state = CertificateRequestState::Approved;
            self.updated_at = Utc::now();
        }
    }

    /// Cite: cert-manager `deny` controller — Pending → Denied (terminal).
    pub fn deny(&mut self, reason: DenialReason, message: impl Into<String>) {
        if self.state == CertificateRequestState::Pending {
            self.state = CertificateRequestState::Denied;
            self.failure_message = Some(format!("[{}] {}", reason.as_str(), message.into()));
            self.updated_at = Utc::now();
        }
    }

    /// Like `deny` but returns an error if the request is not in a deniable
    /// state (Pending). Used when callers must handle terminal states
    /// explicitly.
    pub fn try_deny(
        &mut self,
        reason: DenialReason,
        message: impl Into<String>,
    ) -> Result<(), String> {
        match self.state {
            CertificateRequestState::Pending => {
                self.deny(reason, message);
                Ok(())
            }
            CertificateRequestState::Issued => {
                Err("cannot deny an already-issued CertificateRequest".into())
            }
            CertificateRequestState::Approved => {
                Err("cannot deny an already-approved CertificateRequest; revoke instead".into())
            }
            CertificateRequestState::Denied => {
                Err("CertificateRequest is already denied".into())
            }
        }
    }

    /// Cite: cert-manager issuer controller — Approved → Issued.
    /// Stamps the issued certificate PEM (one or more PEM blocks).
    pub fn issue(&mut self, certificate_pem: impl Into<String>) {
        if self.state == CertificateRequestState::Approved {
            self.state = CertificateRequestState::Issued;
            self.certificate_pem = Some(certificate_pem.into());
            self.updated_at = Utc::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crds::IssuerRef;

    fn make_spec() -> CertificateRequestSpec {
        CertificateRequestSpec {
            tenant_id: "t1".into(),
            issuer_ref: IssuerRef::issuer("le-prod"),
            csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\n...\n-----END CERTIFICATE REQUEST-----\n".into(),
            is_ca: false,
            duration_seconds: Some(90 * 86_400),
            usages: vec!["server auth".into()],
        }
    }

    #[test]
    fn new_request_starts_pending() {
        let cr = CertificateRequest::new("t1", make_spec());
        assert_eq!(cr.state, CertificateRequestState::Pending);
        assert!(cr.certificate_pem.is_none());
    }

    #[test]
    fn approve_transitions_to_approved() {
        let mut cr = CertificateRequest::new("t1", make_spec());
        cr.approve();
        assert_eq!(cr.state, CertificateRequestState::Approved);
    }

    #[test]
    fn approve_is_idempotent() {
        let mut cr = CertificateRequest::new("t1", make_spec());
        cr.approve();
        cr.approve();
        assert_eq!(cr.state, CertificateRequestState::Approved);
    }

    #[test]
    fn deny_transitions_to_denied_with_message() {
        let mut cr = CertificateRequest::new("t1", make_spec());
        cr.deny(DenialReason::PolicyViolation, "no wildcards");
        assert_eq!(cr.state, CertificateRequestState::Denied);
        assert!(cr.failure_message.is_some());
    }

    #[test]
    fn issue_after_approve_stamps_pem() {
        let mut cr = CertificateRequest::new("t1", make_spec());
        cr.approve();
        cr.issue("cert-pem");
        assert_eq!(cr.state, CertificateRequestState::Issued);
        assert_eq!(cr.certificate_pem.as_deref(), Some("cert-pem"));
    }

    #[test]
    fn spec_validates_empty_csr() {
        let mut s = make_spec();
        s.csr_pem = "".into();
        assert!(s.validate().is_err());
    }
}
