// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CRD models — mirror `cert-manager/cert-manager/pkg/apis/certmanager/v1/types_*.go`.
//!
//! Covered CRDs (cert-manager v1.20.2):
//!   * Certificate            (`types_certificate.go`)
//!   * CertificateRequest     (`types_certificaterequest.go`)
//!   * Issuer / ClusterIssuer (`types_issuer.go`)
//!
//! Cite per-field comments to upstream Go fields where structure may
//! drift over time.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::error::{CertManagerError, CertManagerResult};

// ─── Certificate ────────────────────────────────────────────────────────────

/// Cite: `pkg/apis/certmanager/v1/types_certificate.go::Certificate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Certificate {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub spec: CertificateSpec,
    #[serde(default)]
    pub status: Option<CertificateStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

/// Cite: `pkg/apis/certmanager/v1/types_certificate.go::CertificateSpec`.
///
/// Field names follow cert-manager's snake_case-of-camelCase mapping so a
/// straight YAML→TOML lift stays mechanical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateSpec {
    /// `secretName` — the Secret to materialise into.
    pub secret_name: String,
    /// `issuerRef` — Issuer or ClusterIssuer to delegate the request to.
    pub issuer_ref: IssuerRef,
    /// `dnsNames` — SAN dnsNames. Must contain ≥ 1 entry (cert-manager
    /// `validation.ValidateCertificate`).
    #[serde(default)]
    pub dns_names: Vec<String>,
    /// `ipAddresses` — SAN ipAddresses.
    #[serde(default)]
    pub ip_addresses: Vec<String>,
    /// `uris` — SAN URIs.
    #[serde(default)]
    pub uris: Vec<String>,
    /// `emailAddresses` — SAN rfc822Names.
    #[serde(default)]
    pub email_addresses: Vec<String>,
    /// `commonName` — optional CN (deprecated by RFC 6125 but still
    /// supported by cert-manager).
    #[serde(default)]
    pub common_name: Option<String>,
    /// `duration` in seconds. Cite: cert-manager default is 90 days
    /// (`pkg/util/cm/constants.go::DefaultCertificateDuration`).
    pub duration_seconds: i64,
    /// `renewBefore` in seconds. Default 2/3 of duration.
    pub renew_before_seconds: i64,
    /// `usages` — keyUsages + extKeyUsages.
    #[serde(default)]
    pub usages: Vec<Usage>,
    /// `privateKey.policy` — key reuse policy.
    #[serde(default)]
    pub private_key: PrivateKeyPolicy,
    /// `isCA` — request a CA certificate (basicConstraints CA:TRUE).
    #[serde(default)]
    pub is_ca: bool,
    /// `subject` — optional X.509 subject.
    #[serde(default)]
    pub subject: Option<X509Subject>,
    /// `secretTemplate.labels / annotations` to copy onto the Secret.
    #[serde(default)]
    pub secret_template_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_template_annotations: BTreeMap<String, String>,
}

impl CertificateSpec {
    /// Cite: cert-manager `internal/apis/certmanager/validation/certificate.go::ValidateCertificate`.
    pub fn validate(&self) -> CertManagerResult<()> {
        if self.dns_names.is_empty()
            && self.ip_addresses.is_empty()
            && self.uris.is_empty()
            && self.email_addresses.is_empty()
            && self.common_name.is_none()
        {
            return Err(CertManagerError::EmptyDnsNames);
        }
        for dns in &self.dns_names {
            if dns.is_empty() {
                return Err(CertManagerError::InvalidDnsName {
                    name: dns.clone(),
                    reason: "empty".into(),
                });
            }
            if dns.contains('/') || dns.contains(' ') {
                return Err(CertManagerError::InvalidDnsName {
                    name: dns.clone(),
                    reason: "must not contain `/` or whitespace".into(),
                });
            }
            if dns.len() > 253 {
                return Err(CertManagerError::InvalidDnsName {
                    name: dns.clone(),
                    reason: "longer than 253 chars (RFC 1035)".into(),
                });
            }
        }
        if self.duration_seconds <= 0 {
            return Err(CertManagerError::InvalidSpec(
                "duration must be > 0".into(),
            ));
        }
        if self.renew_before_seconds < 0 {
            return Err(CertManagerError::InvalidSpec(
                "renewBefore must be >= 0".into(),
            ));
        }
        if self.renew_before_seconds >= self.duration_seconds {
            return Err(CertManagerError::RenewBeforeExceedsDuration {
                renew_before_seconds: self.renew_before_seconds,
                duration_seconds: self.duration_seconds,
            });
        }
        Ok(())
    }
}

/// Cite: `pkg/apis/certmanager/v1/types.go::IssuerRef`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuerRef {
    pub name: String,
    /// `Issuer` (namespaced) or `ClusterIssuer` (cluster-scoped).
    pub kind: IssuerRefKind,
    /// cert-manager's only group is `cert-manager.io`.
    #[serde(default = "default_group")]
    pub group: String,
}

fn default_group() -> String {
    "cert-manager.io".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssuerRefKind {
    Issuer,
    ClusterIssuer,
}

/// Cite: `pkg/apis/certmanager/v1/types_certificate.go::KeyUsage` +
/// `ExtKeyUsage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Usage {
    DigitalSignature,
    KeyEncipherment,
    DataEncipherment,
    KeyAgreement,
    CertSign,
    CrlSign,
    ServerAuth,
    ClientAuth,
    CodeSigning,
    EmailProtection,
    SMime,
    TimeStamping,
    OcspSigning,
}

impl Usage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DigitalSignature => "digital signature",
            Self::KeyEncipherment => "key encipherment",
            Self::DataEncipherment => "data encipherment",
            Self::KeyAgreement => "key agreement",
            Self::CertSign => "cert sign",
            Self::CrlSign => "crl sign",
            Self::ServerAuth => "server auth",
            Self::ClientAuth => "client auth",
            Self::CodeSigning => "code signing",
            Self::EmailProtection => "email protection",
            Self::SMime => "s/mime",
            Self::TimeStamping => "timestamping",
            Self::OcspSigning => "ocsp signing",
        }
    }
}

/// Cite: `pkg/apis/certmanager/v1/types_certificate.go::CertificatePrivateKey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateKeyPolicy {
    pub rotation: RotationPolicy,
    pub algorithm: KeyAlgo,
    pub size: KeySize,
    pub encoding: KeyEncoding,
}

impl Default for PrivateKeyPolicy {
    fn default() -> Self {
        Self {
            rotation: RotationPolicy::Never,
            algorithm: KeyAlgo::Ecdsa,
            size: KeySize::Ecdsa256,
            encoding: KeyEncoding::Pkcs8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RotationPolicy {
    /// Reuse the existing key across renewals (default).
    Never,
    /// Generate a fresh key on every issuance / renewal.
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum KeyAlgo {
    Rsa,
    Ecdsa,
    Ed25519,
}

/// Encoded as cert-manager does — a single field carrying either RSA bit
/// length or ECDSA curve size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeySize {
    Rsa2048,
    Rsa3072,
    Rsa4096,
    Ecdsa256,
    Ecdsa384,
    Ecdsa521,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum KeyEncoding {
    Pkcs1,
    Pkcs8,
}

/// Cite: `pkg/apis/certmanager/v1/types_certificate.go::X509Subject`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct X509Subject {
    #[serde(default)]
    pub organizations: Vec<String>,
    #[serde(default)]
    pub organizational_units: Vec<String>,
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub localities: Vec<String>,
    #[serde(default)]
    pub provinces: Vec<String>,
    #[serde(default)]
    pub street_addresses: Vec<String>,
    #[serde(default)]
    pub postal_codes: Vec<String>,
    #[serde(default)]
    pub serial_number: Option<String>,
}

/// Cite: `pkg/apis/certmanager/v1/types_certificate.go::CertificateStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateStatus {
    /// Reflects ready / issuing.
    pub conditions: Vec<CertificateCondition>,
    /// Sha-256 of the materialised cert PEM (stand-in for the real chain
    /// hash). Populated after the first successful issuance.
    pub serial: Option<String>,
    pub not_before: Option<DateTime<Utc>>,
    pub not_after: Option<DateTime<Utc>>,
    pub renewal_time: Option<DateTime<Utc>>,
    pub revision: u64,
    pub last_failure_message: Option<String>,
    /// Materialised Secret reference.
    pub secret_ref: Option<SecretRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateCondition {
    #[serde(rename = "type")]
    pub kind: CertificateConditionType,
    pub status: ConditionStatus,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CertificateConditionType {
    /// `Ready`  — Secret materialised + not expired + renewBefore not hit.
    Ready,
    /// `Issuing` — a new CertificateRequest is in flight.
    Issuing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub namespace: String,
}

// ─── CertificateRequest ─────────────────────────────────────────────────────

/// Cite: `pkg/apis/certmanager/v1/types_certificaterequest.go::CertificateRequest`.
///
/// One CertificateRequest per Certificate revision. The certificate
/// controller projects a Certificate into a CertificateRequest, hands
/// it to the issuer, and waits for `Ready=True`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateRequest {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    /// Owner Certificate.
    pub certificate_id: Uuid,
    pub revision: u64,
    pub issuer_ref: IssuerRef,
    /// Same usages as the parent.
    pub usages: Vec<Usage>,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub uris: Vec<String>,
    pub email_addresses: Vec<String>,
    pub common_name: Option<String>,
    pub duration_seconds: i64,
    pub is_ca: bool,
    pub created_at: DateTime<Utc>,
    pub status: CertificateRequestStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CertificateRequestStatus {
    pub conditions: Vec<CertificateRequestCondition>,
    /// Issued chain (PEM, leaf-first per RFC 5246 §7.4.2).
    pub certificate_chain_pem: Option<String>,
    /// CA chain (PEM, leaf-most intermediate first → root).
    pub ca_pem: Option<String>,
    pub failure_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateRequestCondition {
    pub kind: CertificateRequestConditionType,
    pub status: ConditionStatus,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CertificateRequestConditionType {
    Ready,
    Denied,
    Approved,
    InvalidRequest,
}

// ─── Issuer / ClusterIssuer ─────────────────────────────────────────────────

/// Cite: `pkg/apis/certmanager/v1/types_issuer.go::Issuer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuerResource {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub spec: IssuerSpec,
    pub created_at: DateTime<Utc>,
}

/// Cluster-scoped variant — same shape, no `namespace`. Cite:
/// `pkg/apis/certmanager/v1/types_issuer.go::ClusterIssuer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterIssuer {
    pub id: Uuid,
    pub name: String,
    pub tenant_id: String,
    pub spec: IssuerSpec,
    pub created_at: DateTime<Utc>,
}

/// Cite: `pkg/apis/certmanager/v1/types_issuer.go::IssuerSpec`.
///
/// Carries exactly one issuer kind (sum type). cert-manager uses a
/// pointer-per-kind struct; we use an enum so the type system
/// enforces "exactly one".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum IssuerSpec {
    /// Cite: `pkg/apis/certmanager/v1/types_issuer.go::ACMEIssuer`.
    Acme {
        /// ACMEv2 directory URL.
        directory_url: String,
        /// Account-key keychain handle. Always `keychain:` scheme.
        account_key_keychain_handle: String,
        /// Contact emails (mailto:… prefix added at fire time).
        email: Vec<String>,
        /// Pre-agreed TOS.
        terms_of_service_agreed: bool,
        /// Per-domain solver configuration (HTTP-01 or DNS-01).
        solvers: Vec<AcmeSolver>,
    },
    /// Cite: `pkg/apis/certmanager/v1/types_issuer.go::CAIssuer`.
    Ca {
        /// Name of the secret holding the CA cert + key (in cave-pki).
        secret_name: String,
        /// Optional CA crl distribution points.
        #[serde(default)]
        crl_distribution_points: Vec<String>,
    },
    /// Cite: `pkg/apis/certmanager/v1/types_issuer.go::VaultIssuer`.
    Vault {
        /// Vault server URL.
        server: String,
        /// PKI mount path (e.g. `pki_int`).
        path: String,
        /// PKI role name.
        role: String,
        /// Keychain handle for the Vault token. Always `keychain:` scheme.
        token_keychain_handle: String,
    },
    /// Cite: `pkg/apis/certmanager/v1/types_issuer.go::SelfSignedIssuer`.
    SelfSigned {
        /// Optional CRL distribution points.
        #[serde(default)]
        crl_distribution_points: Vec<String>,
    },
    /// Cite: `pkg/apis/certmanager/v1/types_issuer.go::VenafiIssuer`.
    ///
    /// Model only — see `[[partial]] venafi-issuer` in
    /// `parity.manifest.toml`.
    Venafi {
        zone: String,
        token_keychain_handle: String,
    },
}

impl IssuerSpec {
    pub fn kind(&self) -> IssuerKind {
        match self {
            Self::Acme { .. } => IssuerKind::Acme,
            Self::Ca { .. } => IssuerKind::Ca,
            Self::Vault { .. } => IssuerKind::Vault,
            Self::SelfSigned { .. } => IssuerKind::SelfSigned,
            Self::Venafi { .. } => IssuerKind::Venafi,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssuerKind {
    Acme,
    Ca,
    Vault,
    SelfSigned,
    Venafi,
}

/// Cite: `pkg/apis/certmanager/v1/types_acme.go::ACMEChallengeSolver`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcmeSolver {
    /// dnsZone match selector. Empty → matches all.
    #[serde(default)]
    pub dns_zones: Vec<String>,
    pub challenge: AcmeChallengeSolver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AcmeChallengeSolver {
    /// HTTP-01 served via an Ingress / Gateway. Cite: cert-manager
    /// `pkg/issuer/acme/dns/dns.go` + `pkg/issuer/acme/http/http.go`.
    Http01 {
        ingress_class: Option<String>,
        service_type: Option<String>,
    },
    /// DNS-01 via one of cert-manager's DNS providers. cave-cert-manager
    /// owns the cave-dns provider only — others land as
    /// `[[scope_cuts]]` in `parity.manifest.toml`.
    Dns01 { provider: DnsProvider },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum DnsProvider {
    /// In-tree provider that writes TXT records into cave-dns.
    CaveDns { zone: String },
    /// `webhook` solver — out-of-tree DNS providers register via a
    /// gRPC webhook. Model only (Phase 2).
    Webhook { group_name: String, solver_name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_spec() -> CertificateSpec {
        CertificateSpec {
            secret_name: "tls".into(),
            issuer_ref: IssuerRef {
                name: "letsencrypt".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: default_group(),
            },
            dns_names: vec!["example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 90 * 24 * 3600,
            renew_before_seconds: 30 * 24 * 3600,
            usages: vec![Usage::ServerAuth, Usage::DigitalSignature],
            private_key: PrivateKeyPolicy::default(),
            is_ca: false,
            subject: None,
            secret_template_labels: BTreeMap::new(),
            secret_template_annotations: BTreeMap::new(),
        }
    }

    #[test]
    fn validate_ok() {
        ok_spec().validate().unwrap();
    }

    #[test]
    fn validate_requires_at_least_one_identifier() {
        let mut s = ok_spec();
        s.dns_names.clear();
        match s.validate() {
            Err(CertManagerError::EmptyDnsNames) => {}
            other => panic!("expected EmptyDnsNames, got {:?}", other),
        }
    }

    #[test]
    fn validate_rejects_slash_in_dns_name() {
        let mut s = ok_spec();
        s.dns_names = vec!["bad/name".into()];
        assert!(matches!(
            s.validate(),
            Err(CertManagerError::InvalidDnsName { .. })
        ));
    }

    #[test]
    fn validate_rejects_long_dns_name() {
        let mut s = ok_spec();
        s.dns_names = vec!["a".repeat(254)];
        assert!(matches!(
            s.validate(),
            Err(CertManagerError::InvalidDnsName { .. })
        ));
    }

    #[test]
    fn validate_renew_before_must_be_less_than_duration() {
        let mut s = ok_spec();
        s.renew_before_seconds = s.duration_seconds;
        assert!(matches!(
            s.validate(),
            Err(CertManagerError::RenewBeforeExceedsDuration { .. })
        ));
    }

    #[test]
    fn validate_rejects_zero_duration() {
        let mut s = ok_spec();
        s.duration_seconds = 0;
        assert!(matches!(s.validate(), Err(CertManagerError::InvalidSpec(_))));
    }

    #[test]
    fn issuer_kind_extracted_from_spec() {
        let spec = IssuerSpec::SelfSigned {
            crl_distribution_points: vec![],
        };
        assert_eq!(spec.kind(), IssuerKind::SelfSigned);
    }

    #[test]
    fn dns_name_email_only_accepted() {
        let mut s = ok_spec();
        s.dns_names.clear();
        s.email_addresses.push("ops@example.com".into());
        s.validate().unwrap();
    }

    #[test]
    fn usage_strings_match_x509_terms() {
        assert_eq!(Usage::ServerAuth.as_str(), "server auth");
        assert_eq!(Usage::CertSign.as_str(), "cert sign");
    }
}
