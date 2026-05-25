// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SPIFFE SVID enrolment.
//!
//! Mirrors the workload-identity issuance side of `security/pkg/pki/ca/ca.go`
//! plus the SPIFFE-ID parser from `security/pkg/server/ca/authenticate/`.
//! Real X.509 issuance happens in `cave-auth`; this module models the
//! enrolment request shape, SPIFFE-ID validation, and the issued SVID
//! envelope so the rest of the Ambient stack can be tested end-to-end.

use crate::ambient::types::{Cite, TenantId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SvidError {
    #[error("SPIFFE ID must start with `spiffe://`, got {0}")]
    BadScheme(String),
    #[error("SPIFFE ID {0} has empty trust domain")]
    NoTrustDomain(String),
    #[error("SPIFFE ID {0} must follow `spiffe://<td>/ns/<ns>/sa/<sa>`")]
    BadShape(String),
    #[error("CSR principal {csr_principal} does not match enrolment principal {enrol_principal}")]
    PrincipalMismatch {
        csr_principal: String,
        enrol_principal: String,
    },
    #[error("tenant {tenant} not authorised to enrol {principal}")]
    TenantDenied { tenant: TenantId, principal: String },
    #[error("requested TTL {requested_secs}s exceeds issuer max {max_secs}s")]
    TtlTooLong { requested_secs: i64, max_secs: i64 },
}

/// Parsed SPIFFE ID. `spiffe://<trust_domain>/ns/<namespace>/sa/<service_account>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpiffeId {
    pub trust_domain: String,
    pub namespace: String,
    pub service_account: String,
}

impl SpiffeId {
    pub fn parse(s: &str) -> Result<Self, SvidError> {
        let rest = s
            .strip_prefix("spiffe://")
            .ok_or_else(|| SvidError::BadScheme(s.into()))?;
        let mut parts = rest.splitn(2, '/');
        let trust_domain = parts.next().unwrap_or("");
        if trust_domain.is_empty() {
            return Err(SvidError::NoTrustDomain(s.into()));
        }
        let path = parts.next().unwrap_or("");
        let segments: Vec<&str> = path.split('/').collect();
        if segments.len() != 4 || segments[0] != "ns" || segments[2] != "sa" {
            return Err(SvidError::BadShape(s.into()));
        }
        Ok(Self {
            trust_domain: trust_domain.into(),
            namespace: segments[1].into(),
            service_account: segments[3].into(),
        })
    }

    pub fn as_uri(&self) -> String {
        format!(
            "spiffe://{}/ns/{}/sa/{}",
            self.trust_domain, self.namespace, self.service_account
        )
    }
}

/// Enrolment request. The `csr_principal` is the SPIFFE URI the workload's
/// CSR claims; the issuer must verify it against `enrol_principal` (which
/// the issuer derives from the workload's k8s service-account token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolRequest {
    pub csr_principal: String,
    pub enrol_principal: String,
    pub ttl_seconds: i64,
}

/// Issued SVID envelope. The actual cert chain lives in `cave-auth`; here we
/// keep just the metadata the Ambient stack reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Svid {
    pub principal: String,
    pub trust_domain: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Svid {
    pub fn is_valid_at(&self, t: DateTime<Utc>) -> bool {
        t >= self.issued_at && t < self.expires_at
    }
}

/// Issuer policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuerPolicy {
    pub trust_domain: String,
    pub max_ttl_seconds: i64,
    pub tenant: TenantId,
}

impl IssuerPolicy {
    /// Issue an SVID. Mirrors `ca.IstioCA.Sign` upstream — same shape, same
    /// validation order, no real crypto.
    pub fn issue(&self, req: &EnrolRequest, now: DateTime<Utc>) -> Result<Svid, SvidError> {
        if req.csr_principal != req.enrol_principal {
            return Err(SvidError::PrincipalMismatch {
                csr_principal: req.csr_principal.clone(),
                enrol_principal: req.enrol_principal.clone(),
            });
        }
        if req.ttl_seconds > self.max_ttl_seconds {
            return Err(SvidError::TtlTooLong {
                requested_secs: req.ttl_seconds,
                max_secs: self.max_ttl_seconds,
            });
        }
        let parsed = SpiffeId::parse(&req.enrol_principal)?;
        if parsed.trust_domain != self.trust_domain {
            return Err(SvidError::TenantDenied {
                tenant: self.tenant.clone(),
                principal: req.enrol_principal.clone(),
            });
        }
        // Tenant convention: `parsed.namespace == issuer.tenant.as_str()`.
        if parsed.namespace != self.tenant.as_str() {
            return Err(SvidError::TenantDenied {
                tenant: self.tenant.clone(),
                principal: req.enrol_principal.clone(),
            });
        }
        Ok(Svid {
            principal: parsed.as_uri(),
            trust_domain: self.trust_domain.clone(),
            issued_at: now,
            expires_at: now + Duration::seconds(req.ttl_seconds),
        })
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::istio("security/pkg/pki/ca/ca.go", "IstioCA.Sign");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn issuer(tenant: &str) -> IssuerPolicy {
        IssuerPolicy {
            trust_domain: "cluster.local".into(),
            max_ttl_seconds: 3600,
            tenant: TenantId::new(tenant).expect("test fixture"),
        }
    }

    #[test]
    fn parses_well_formed_spiffe_id() {
        let (_cite, _t) = ambient_test_ctx!(
            "security/pkg/server/ca/authenticate/identity.go",
            "ParseSpiffeId",
            "tenant-svid-parse"
        );
        let id = SpiffeId::parse("spiffe://cluster.local/ns/acme/sa/web").unwrap();
        assert_eq!(id.trust_domain, "cluster.local");
        assert_eq!(id.namespace, "acme");
        assert_eq!(id.service_account, "web");
        assert_eq!(id.as_uri(), "spiffe://cluster.local/ns/acme/sa/web");
    }

    #[test]
    fn rejects_non_spiffe_scheme() {
        let (_cite, _t) = ambient_test_ctx!(
            "security/pkg/server/ca/authenticate/identity.go",
            "ParseSpiffeId",
            "tenant-svid-scheme"
        );
        assert!(matches!(
            SpiffeId::parse("https://cluster.local/x"),
            Err(SvidError::BadScheme(_))
        ));
    }

    #[test]
    fn rejects_malformed_path_shape() {
        let (_cite, _t) = ambient_test_ctx!(
            "security/pkg/server/ca/authenticate/identity.go",
            "ParseSpiffeId",
            "tenant-svid-shape"
        );
        assert!(matches!(
            SpiffeId::parse("spiffe://cluster.local/foo/bar"),
            Err(SvidError::BadShape(_))
        ));
    }

    #[test]
    fn issues_svid_for_matching_principal_and_ttl() {
        let (_cite, _t) = ambient_test_ctx!("security/pkg/pki/ca/ca.go", "IstioCA.Sign", "acme");
        let now = Utc::now();
        let svid = issuer("acme")
            .issue(
                &EnrolRequest {
                    csr_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
                    enrol_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
                    ttl_seconds: 600,
                },
                now,
            )
            .unwrap();
        assert_eq!(svid.principal, "spiffe://cluster.local/ns/acme/sa/web");
        assert!(svid.is_valid_at(now));
        assert!(svid.is_valid_at(now + Duration::seconds(599)));
        assert!(!svid.is_valid_at(now + Duration::seconds(601)));
    }

    #[test]
    fn principal_mismatch_blocks_issuance() {
        let (_cite, _t) = ambient_test_ctx!("security/pkg/pki/ca/ca.go", "verifyPrincipal", "acme");
        let err = issuer("acme")
            .issue(
                &EnrolRequest {
                    csr_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
                    enrol_principal: "spiffe://cluster.local/ns/acme/sa/admin".into(),
                    ttl_seconds: 600,
                },
                Utc::now(),
            )
            .unwrap_err();
        assert!(matches!(err, SvidError::PrincipalMismatch { .. }));
    }

    #[test]
    fn cross_tenant_enrolment_is_refused() {
        let (_cite, _t) = ambient_test_ctx!(
            "security/pkg/pki/ca/ca.go",
            "tenantScope",
            "tenant-svid-cross"
        );
        // Issuer is for `acme`; request is for `evil`.
        let err = issuer("acme")
            .issue(
                &EnrolRequest {
                    csr_principal: "spiffe://cluster.local/ns/evil/sa/x".into(),
                    enrol_principal: "spiffe://cluster.local/ns/evil/sa/x".into(),
                    ttl_seconds: 600,
                },
                Utc::now(),
            )
            .unwrap_err();
        assert!(matches!(err, SvidError::TenantDenied { .. }));
    }

    #[test]
    fn ttl_above_issuer_max_is_clamped_to_error() {
        let (_cite, _t) = ambient_test_ctx!("security/pkg/pki/ca/ca.go", "IstioCA.Sign", "acme");
        let err = issuer("acme")
            .issue(
                &EnrolRequest {
                    csr_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
                    enrol_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
                    ttl_seconds: 10_000,
                },
                Utc::now(),
            )
            .unwrap_err();
        assert!(matches!(err, SvidError::TtlTooLong { .. }));
    }
}
