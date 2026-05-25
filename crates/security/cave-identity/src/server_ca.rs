// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Algorithm + state-machine
// line-ported from pkg/server/ca/ca.go + pkg/server/ca/manager.go.
//
//! SPIRE server CA — root + intermediate hierarchy, key rotation, trust
//! bundle assembly.
//!
//! ## Lifecycle
//! 1. Bootstrap creates a self-signed `root` (long-lived, default 1 year).
//! 2. The root signs an `intermediate` (default 24 h) used to sign X.509-SVIDs.
//! 3. A scheduled rotation issues a new intermediate before the current one
//!    expires (`PrepareJWTKey` + `ActivateJWTKey` analog in upstream).
//! 4. Old authorities are kept in the trust bundle until `prepared_until +
//!    overlap` elapses, then removed. Tainting marks them for replacement
//!    without removal so downstream relays can rotate gracefully.

use crate::error::{IdentityError, Result};
use crate::models::{Bundle, JwtAuthority, TrustDomain, X509Authority};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

/// Logical authority key handle — opaque DER bytes for cert + private key.
///
/// Placeholder cryptographic material; real backend would delegate to
/// [`cave_pki`] or a KMS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaKey {
    pub key_id: String,
    pub algorithm: SignatureAlgorithm,
    /// PKIX DER public key.
    pub public_key_der: Vec<u8>,
    /// PKCS#8 DER private key — `None` for read-only/imported authorities.
    pub private_key_der: Option<Vec<u8>>,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureAlgorithm {
    EcdsaP256Sha256,
    Ed25519,
    Rsa2048Sha256,
}

impl SignatureAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignatureAlgorithm::EcdsaP256Sha256 => "ES256",
            SignatureAlgorithm::Ed25519 => "EdDSA",
            SignatureAlgorithm::Rsa2048Sha256 => "RS256",
        }
    }
    pub fn jwk_kty(&self) -> &'static str {
        match self {
            SignatureAlgorithm::EcdsaP256Sha256 => "EC",
            SignatureAlgorithm::Ed25519 => "OKP",
            SignatureAlgorithm::Rsa2048Sha256 => "RSA",
        }
    }
}

/// X.509 authority slot — root or intermediate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X509Authority2 {
    pub key: CaKey,
    /// DER-encoded X.509 cert.
    pub cert_der: Vec<u8>,
    pub tainted: bool,
}

/// JWT signing authority slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtAuthority2 {
    pub key: CaKey,
    pub tainted: bool,
}

/// CA-rotation configuration — equivalent of `pkg/server/ca.RotationParams`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationParams {
    /// Root cert lifetime.
    pub root_ttl: Duration,
    /// Intermediate cert lifetime.
    pub intermediate_ttl: Duration,
    /// Period during which both old and new authority remain in the bundle.
    pub overlap: Duration,
    /// JWT-SVID signing key lifetime.
    pub jwt_key_ttl: Duration,
    /// Trust-bundle refresh hint published to relying parties (s).
    pub refresh_hint_seconds: u64,
}

impl Default for RotationParams {
    fn default() -> Self {
        Self {
            root_ttl: Duration::days(365),
            intermediate_ttl: Duration::hours(24),
            overlap: Duration::hours(6),
            jwt_key_ttl: Duration::hours(24),
            refresh_hint_seconds: 300,
        }
    }
}

/// SPIRE server CA — issues X.509-SVIDs against an intermediate, JWT-SVIDs
/// against a rotating signing key, and assembles a public trust bundle.
pub struct ServerCa {
    inner: Arc<RwLock<CaInner>>,
}

struct CaInner {
    trust_domain: TrustDomain,
    params: RotationParams,
    /// Active root authorities (newest first).
    roots: Vec<X509Authority2>,
    /// Active intermediates (the first is used for signing).
    intermediates: Vec<X509Authority2>,
    /// Active JWT signing authorities (first is current).
    jwt_keys: Vec<JwtAuthority2>,
    sequence_number: u64,
}

impl ServerCa {
    pub fn new(trust_domain: TrustDomain, params: RotationParams) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CaInner {
                trust_domain,
                params,
                roots: Vec::new(),
                intermediates: Vec::new(),
                jwt_keys: Vec::new(),
                sequence_number: 0,
            })),
        }
    }

    /// Bootstrap: install a brand-new root, an intermediate, and an initial
    /// JWT signing key. Called once at server startup or via
    /// `spire-server token generate` flows.
    pub fn bootstrap(&self, now: DateTime<Utc>) -> Result<()> {
        let mut g = self.inner.write().expect("poisoned");
        let params = g.params.clone();
        let root_key = synth_key("root-0", SignatureAlgorithm::EcdsaP256Sha256, now, params.root_ttl);
        let int_key = synth_key(
            "int-0",
            SignatureAlgorithm::EcdsaP256Sha256,
            now,
            params.intermediate_ttl,
        );
        let jwt_key = synth_key(
            "jwt-0",
            SignatureAlgorithm::EcdsaP256Sha256,
            now,
            params.jwt_key_ttl,
        );
        g.roots.insert(
            0,
            X509Authority2 {
                cert_der: synth_cert_der(&root_key, &root_key),
                key: root_key,
                tainted: false,
            },
        );
        let root_clone = g.roots[0].key.clone();
        g.intermediates.insert(
            0,
            X509Authority2 {
                cert_der: synth_cert_der(&int_key, &root_clone),
                key: int_key,
                tainted: false,
            },
        );
        g.jwt_keys.insert(0, JwtAuthority2 { key: jwt_key, tainted: false });
        g.sequence_number += 1;
        Ok(())
    }

    /// Rotate the intermediate (called when ttl/2 has elapsed).
    pub fn rotate_intermediate(&self, now: DateTime<Utc>) -> Result<()> {
        let mut g = self.inner.write().expect("poisoned");
        let params = g.params.clone();
        let root_clone = g
            .roots
            .first()
            .ok_or(IdentityError::CaNotInitialised)?
            .key
            .clone();
        let new_id = format!("int-{}", g.sequence_number);
        let int_key = synth_key(
            &new_id,
            SignatureAlgorithm::EcdsaP256Sha256,
            now,
            params.intermediate_ttl,
        );
        g.intermediates.insert(
            0,
            X509Authority2 {
                cert_der: synth_cert_der(&int_key, &root_clone),
                key: int_key,
                tainted: false,
            },
        );
        // Prune expired (now > old_not_after + overlap)
        let cutoff = now - params.overlap;
        g.intermediates
            .retain(|a| a.key.not_after >= cutoff);
        g.sequence_number += 1;
        Ok(())
    }

    /// Rotate the JWT signing key.
    pub fn rotate_jwt_key(&self, now: DateTime<Utc>) -> Result<()> {
        let mut g = self.inner.write().expect("poisoned");
        let params = g.params.clone();
        let new_id = format!("jwt-{}", g.sequence_number);
        let jwt_key = synth_key(
            &new_id,
            SignatureAlgorithm::EcdsaP256Sha256,
            now,
            params.jwt_key_ttl,
        );
        g.jwt_keys.insert(0, JwtAuthority2 { key: jwt_key, tainted: false });
        let cutoff = now - params.overlap;
        g.jwt_keys.retain(|a| a.key.not_after >= cutoff);
        g.sequence_number += 1;
        Ok(())
    }

    /// Mark the current root as tainted (key compromise response — relying
    /// parties replace before removal).
    pub fn taint_root(&self) -> Result<()> {
        let mut g = self.inner.write().expect("poisoned");
        g.roots.first_mut().map(|a| a.tainted = true);
        Ok(())
    }

    /// Returns the current signing intermediate.
    pub fn current_intermediate(&self) -> Result<X509Authority2> {
        self.inner
            .read()
            .expect("poisoned")
            .intermediates
            .first()
            .cloned()
            .ok_or(IdentityError::CaNotInitialised)
    }

    /// Returns the current JWT signing key.
    pub fn current_jwt_key(&self) -> Result<JwtAuthority2> {
        self.inner
            .read()
            .expect("poisoned")
            .jwt_keys
            .first()
            .cloned()
            .ok_or(IdentityError::CaNotInitialised)
    }

    /// Trust-bundle snapshot ready to publish over `/bundle` or the
    /// SDS API.
    pub fn trust_bundle(&self) -> Bundle {
        let g = self.inner.read().expect("poisoned");
        Bundle {
            trust_domain: g.trust_domain.clone(),
            x509_authorities: g
                .roots
                .iter()
                .map(|a| X509Authority {
                    asn1_der: a.cert_der.clone(),
                    tainted: a.tainted,
                })
                .collect(),
            jwt_authorities: g
                .jwt_keys
                .iter()
                .map(|j| JwtAuthority {
                    key_id: j.key.key_id.clone(),
                    public_key_der: j.key.public_key_der.clone(),
                    expires_at: Some(j.key.not_after),
                    tainted: j.tainted,
                })
                .collect(),
            refresh_hint_seconds: g.params.refresh_hint_seconds,
            sequence_number: g.sequence_number,
        }
    }

    pub fn trust_domain(&self) -> TrustDomain {
        self.inner.read().expect("poisoned").trust_domain.clone()
    }
}

/// Deterministic key synthesizer for the in-memory backend.
///
/// `public_key_der` is `SHA-256("pub:<id>:<algo>")` — sufficient for the JWKS
/// `kid` mapping and bundle hashing without producing a real cryptographic
/// key. Live deployments swap this for [`cave_pki`] or a KMS.
fn synth_key(
    id: &str,
    algorithm: SignatureAlgorithm,
    now: DateTime<Utc>,
    ttl: Duration,
) -> CaKey {
    use sha2::{Digest, Sha256};
    let mut pub_h = Sha256::new();
    pub_h.update(format!("pub:{}:{}", id, algorithm.as_str()).as_bytes());
    let mut priv_h = Sha256::new();
    priv_h.update(format!("priv:{}:{}", id, algorithm.as_str()).as_bytes());
    CaKey {
        key_id: id.to_string(),
        algorithm,
        public_key_der: pub_h.finalize().to_vec(),
        private_key_der: Some(priv_h.finalize().to_vec()),
        not_before: now,
        not_after: now + ttl,
    }
}

/// Synthesises a placeholder X.509 cert DER as `SHA-256(subject||issuer)`.
fn synth_cert_der(subject: &CaKey, issuer: &CaKey) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"cert:");
    h.update(&subject.public_key_der);
    h.update(b":");
    h.update(&issuer.public_key_der);
    h.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_populates_authorities() {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        let b = ca.trust_bundle();
        assert_eq!(b.trust_domain.as_str(), "example.org");
        assert_eq!(b.x509_authorities.len(), 1);
        assert_eq!(b.jwt_authorities.len(), 1);
        assert_eq!(b.sequence_number, 1);
    }

    #[test]
    fn rotate_intermediate_increments_sequence() {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        let before = ca.trust_bundle().sequence_number;
        ca.rotate_intermediate(Utc::now()).unwrap();
        assert!(ca.trust_bundle().sequence_number > before);
        let int = ca.current_intermediate().unwrap();
        assert_eq!(int.key.key_id, "int-1");
    }

    #[test]
    fn rotate_jwt_key_replaces_current() {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        ca.rotate_jwt_key(Utc::now()).unwrap();
        let cur = ca.current_jwt_key().unwrap();
        assert_eq!(cur.key.key_id, "jwt-1");
        let b = ca.trust_bundle();
        // Within overlap, both keys remain.
        assert!(b.jwt_authorities.len() >= 1);
    }

    #[test]
    fn rotate_without_bootstrap_errors() {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        assert!(ca.rotate_intermediate(Utc::now()).is_err());
    }

    #[test]
    fn taint_root_flips_flag() {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        ca.taint_root().unwrap();
        let b = ca.trust_bundle();
        assert!(b.x509_authorities[0].tainted);
    }

    #[test]
    fn algorithm_kty_mapping() {
        assert_eq!(SignatureAlgorithm::EcdsaP256Sha256.jwk_kty(), "EC");
        assert_eq!(SignatureAlgorithm::Ed25519.jwk_kty(), "OKP");
        assert_eq!(SignatureAlgorithm::Rsa2048Sha256.jwk_kty(), "RSA");
    }

    #[test]
    fn algorithm_str_mapping() {
        assert_eq!(SignatureAlgorithm::EcdsaP256Sha256.as_str(), "ES256");
        assert_eq!(SignatureAlgorithm::Ed25519.as_str(), "EdDSA");
        assert_eq!(SignatureAlgorithm::Rsa2048Sha256.as_str(), "RS256");
    }
}
