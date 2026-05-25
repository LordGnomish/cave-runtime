// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Type shapes line-ported from
// proto/spire/types/{registrationentry,bundle,spiffeid,selector,x509svid,jwtsvid}.proto
// and the Go structs in pkg/common/idutil + pkg/server/datastore.
//
//! Core SPIFFE/SPIRE data model.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// SPIFFE ID — fully-qualified `spiffe://<trust-domain>/<path>`.
///
/// Validated via [`crate::spiffe_id::parse_spiffe_id`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpiffeId(pub String);

impl SpiffeId {
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SpiffeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Trust domain — the authority component of a SPIFFE ID (lowercase host).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrustDomain(pub String);

impl TrustDomain {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Returns the `spiffe://<td>` root identity URI for this trust domain.
    pub fn id_string(&self) -> String {
        format!("spiffe://{}", self.0)
    }
}

/// Selector kinds known to SPIRE (mirrors `pkg/agent/plugin/workloadattestor`).
pub const SELECTOR_K8S: &str = "k8s";
pub const SELECTOR_UNIX: &str = "unix";
pub const SELECTOR_DOCKER: &str = "docker";
pub const SELECTOR_X509_POP: &str = "x509_pop";

/// A registration-entry selector — `kind` + `value` pair (e.g.
/// `k8s:pod-label:app=foo`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Selector {
    pub kind: String,
    pub value: String,
}

impl Selector {
    pub fn new<K: Into<String>, V: Into<String>>(kind: K, value: V) -> Self {
        Self {
            kind: kind.into(),
            value: value.into(),
        }
    }
    /// `kind:value` canonical form.
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.kind, self.value)
    }
}

/// SPIRE registration entry — proto/spire/types/registrationentry.proto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationEntry {
    pub id: String,
    pub spiffe_id: SpiffeId,
    pub parent_id: SpiffeId,
    pub selectors: Vec<Selector>,
    pub ttl_seconds: u32,
    pub federates_with: Vec<TrustDomain>,
    pub admin: bool,
    pub downstream: bool,
    pub dns_names: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revision_number: u64,
    pub jwt_svid_ttl_seconds: u32,
    pub x509_svid_ttl_seconds: u32,
    pub hint: Option<String>,
    pub store_svid: bool,
}

impl Default for RegistrationEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            spiffe_id: SpiffeId::new(""),
            parent_id: SpiffeId::new(""),
            selectors: Vec::new(),
            ttl_seconds: 3600,
            federates_with: Vec::new(),
            admin: false,
            downstream: false,
            dns_names: Vec::new(),
            expires_at: None,
            revision_number: 0,
            jwt_svid_ttl_seconds: 300,
            x509_svid_ttl_seconds: 3600,
            hint: None,
            store_svid: false,
        }
    }
}

/// Trust-domain bundle — proto/spire/types/bundle.proto.
///
/// Carries X.509 authorities (DER) and JWT authorities (per-key-id PKIX DER).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub trust_domain: TrustDomain,
    pub x509_authorities: Vec<X509Authority>,
    pub jwt_authorities: Vec<JwtAuthority>,
    /// Refresh hint in seconds; clients should re-fetch before expiry.
    pub refresh_hint_seconds: u64,
    /// Sequence number — increases on rotation.
    pub sequence_number: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X509Authority {
    /// DER-encoded X.509 cert.
    pub asn1_der: Vec<u8>,
    pub tainted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtAuthority {
    pub key_id: String,
    /// PKIX-DER public key (P-256 / Ed25519 / RSA).
    pub public_key_der: Vec<u8>,
    pub expires_at: Option<DateTime<Utc>>,
    pub tainted: bool,
}

/// X.509-SVID — Cert + chain + key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X509Svid {
    pub spiffe_id: SpiffeId,
    /// Leaf cert DER.
    pub leaf_der: Vec<u8>,
    /// Intermediates (root excluded).
    pub intermediates_der: Vec<Vec<u8>>,
    /// PKCS#8 DER private key.
    pub private_key_der: Vec<u8>,
    pub hint: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// JWT-SVID — token + claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtSvid {
    pub spiffe_id: SpiffeId,
    pub token: String,
    pub audience: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub issued_at: DateTime<Utc>,
    pub hint: Option<String>,
}

/// Decoded JWT-SVID claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtSvidClaims {
    pub sub: String,
    pub aud: Vec<String>,
    pub exp: i64,
    pub iat: i64,
}

/// Agent attestation record — `pkg/server/datastore.AttestedNode`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestedNode {
    pub spiffe_id: SpiffeId,
    pub attestation_type: String,
    pub serial_number: String,
    pub cert_not_after: DateTime<Utc>,
    pub new_serial_number: Option<String>,
    pub new_cert_not_after: Option<DateTime<Utc>>,
    pub banned: bool,
    pub selectors: Vec<Selector>,
}

/// Federation relationship — `pkg/common/bundleutil`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationRelationship {
    pub trust_domain: TrustDomain,
    pub bundle_endpoint_url: String,
    pub bundle_endpoint_profile: BundleEndpointProfile,
    pub trust_domain_bundle: Option<Bundle>,
}

/// SPIFFE bundle endpoint profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BundleEndpointProfile {
    /// HTTPS-based bundle endpoint (server cert from public PKI).
    HttpsWeb,
    /// SPIFFE-authenticated endpoint (server uses its own SPIFFE cert).
    HttpsSpiffe { endpoint_spiffe_id: SpiffeId },
}

/// Workload-attestation outcome — list of selectors discovered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkloadAttestation {
    pub pid: i32,
    pub selectors: Vec<Selector>,
}

/// Node-attestation challenge/response state — `pkg/agent/plugin/nodeattestor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAttestation {
    pub attestation_type: String,
    pub agent_id: SpiffeId,
    pub selectors: Vec<Selector>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_canonical() {
        let s = Selector::new("k8s", "pod-label:app=foo");
        assert_eq!(s.canonical(), "k8s:pod-label:app=foo");
    }

    #[test]
    fn trust_domain_id() {
        let td = TrustDomain::new("example.org");
        assert_eq!(td.id_string(), "spiffe://example.org");
    }

    #[test]
    fn registration_entry_default_ttl() {
        let e = RegistrationEntry::default();
        assert_eq!(e.ttl_seconds, 3600);
        assert_eq!(e.x509_svid_ttl_seconds, 3600);
        assert_eq!(e.jwt_svid_ttl_seconds, 300);
        assert!(!e.admin);
        assert!(!e.downstream);
    }

    #[test]
    fn spiffe_id_display() {
        let id = SpiffeId::new("spiffe://example.org/foo");
        assert_eq!(format!("{}", id), "spiffe://example.org/foo");
    }

    #[test]
    fn bundle_endpoint_profile_eq() {
        assert_eq!(BundleEndpointProfile::HttpsWeb, BundleEndpointProfile::HttpsWeb);
        let p = BundleEndpointProfile::HttpsSpiffe {
            endpoint_spiffe_id: SpiffeId::new("spiffe://example.org/spire/server"),
        };
        if let BundleEndpointProfile::HttpsSpiffe { endpoint_spiffe_id } = &p {
            assert!(endpoint_spiffe_id.as_str().contains("spire/server"));
        } else {
            panic!("variant mismatch");
        }
    }
}
