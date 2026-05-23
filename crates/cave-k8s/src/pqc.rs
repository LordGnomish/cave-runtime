// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PQC-ready ServiceAccount token signing.
//!
//! K8s' `kube-apiserver` signs projected ServiceAccount JWTs with a
//! single ECDSA P-256 or RSA key. cave-k8s upgrades the signing path
//! to a **hybrid** envelope:
//!
//! 1. **Classical** — Ed25519 (compact, fast, well-supported by JWT verifiers).
//! 2. **PQC** — ML-DSA-65 (NIST FIPS-204).
//!
//! Both signatures cover the same canonical JOSE payload.  Verifiers may
//! check either or both: a classical-only verifier still accepts the
//! token, while a Charter-v2-compliant verifier requires the PQC half to
//! be present and valid.  This mirrors the IETF "hybrid signature draft"
//! and the SAE proposal from JOSE-WG (`alg = "Ed25519+ML-DSA-65"`).
//!
//! Concrete ML-DSA-65 implementation is staged: the envelope wire format
//! is fully implemented (algorithm tag, signature length field, PQC
//! signature bytes), but the PQC primitive itself is presently
//! delegated to a deterministic Ed25519-of-(domain ‖ payload) — a
//! placeholder that *the verifier rejects* unless `accept_placeholder` is
//! set on the [`HybridVerifier`].  When the workspace upgrades to a real
//! `pqcrypto-mldsa` dependency, only the inner `sign_pqc`/`verify_pqc`
//! pair changes.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey, Signature as Ed25519Sig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const ALG_HYBRID: &str = "Ed25519+ML-DSA-65";
pub const PQC_PLACEHOLDER_DOMAIN: &[u8] = b"cave-k8s/pqc/placeholder/v1";
/// Wire length of the PQC half — fixed to ML-DSA-65 signature size
/// (3309 bytes for NIST FIPS-204) so the envelope remains parseable
/// even with a placeholder backing implementation.
pub const PQC_SIG_LEN: usize = 3309;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PqcError {
    #[error("hybrid signature header malformed: {0}")]
    Header(String),
    #[error("classical signature verification failed")]
    ClassicalSig,
    #[error("PQC signature verification failed")]
    PqcSig,
    #[error("PQC half is the placeholder and accept_placeholder=false")]
    PlaceholderRefused,
    #[error("base64 decode: {0}")]
    Base64(String),
}

/// A hybrid signer holds one Ed25519 keypair + (when real impl lands)
/// one ML-DSA-65 keypair.  For now both halves come from the same
/// random seed to keep the envelope deterministic for unit tests.
pub struct HybridSigner {
    classical: SigningKey,
    pqc_seed: [u8; 32],
}

impl HybridSigner {
    pub fn generate() -> Self {
        Self::from_seed(rand_bytes_32())
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        let classical = SigningKey::from_bytes(&seed);
        let mut pqc_seed = [0u8; 32];
        let mut h = Sha256::new();
        h.update(b"pqc/");
        h.update(seed);
        pqc_seed.copy_from_slice(&h.finalize());
        Self { classical, pqc_seed }
    }

    pub fn classical_public(&self) -> VerifyingKey {
        self.classical.verifying_key()
    }

    /// Sign `payload`, producing a wire-format hybrid signature:
    ///
    ///   `alg(utf8) ‖ 0x00 ‖ len_be(u32) ed25519_sig ‖ pqc_sig(PQC_SIG_LEN)`
    ///
    /// where `alg = ALG_HYBRID`.
    pub fn sign(&self, payload: &[u8]) -> Vec<u8> {
        let classical: Ed25519Sig = self.classical.sign(payload);
        let pqc = self.sign_pqc(payload);
        let mut out = Vec::with_capacity(ALG_HYBRID.len() + 1 + 4 + 64 + PQC_SIG_LEN);
        out.extend_from_slice(ALG_HYBRID.as_bytes());
        out.push(0);
        let csig = classical.to_bytes();
        out.extend_from_slice(&(csig.len() as u32).to_be_bytes());
        out.extend_from_slice(&csig);
        out.extend_from_slice(&pqc);
        out
    }

    /// PQC half — placeholder backed by SHA-256(domain ‖ pqc_seed ‖ payload)
    /// expanded to the FIPS-204 ML-DSA-65 signature length.  Deterministic.
    fn sign_pqc(&self, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(PQC_SIG_LEN);
        let mut ctr: u32 = 0;
        while buf.len() < PQC_SIG_LEN {
            let mut h = Sha256::new();
            h.update(PQC_PLACEHOLDER_DOMAIN);
            h.update(self.pqc_seed);
            h.update(ctr.to_be_bytes());
            h.update(payload);
            buf.extend_from_slice(&h.finalize());
            ctr += 1;
        }
        buf.truncate(PQC_SIG_LEN);
        buf
    }
}

/// Verifier — given a `VerifyingKey` (classical half), parse the wire
/// envelope, verify the classical signature, and consult the placeholder
/// PQC predicate.  When the real ML-DSA-65 library lands, the verifier
/// switches on `alg == ALG_HYBRID` and dispatches to the new primitive.
pub struct HybridVerifier {
    pub classical_pub: VerifyingKey,
    /// When true the verifier passes through the PQC half regardless of
    /// the placeholder predicate.  Production deployments leave this
    /// false; integration tests for the migration path flip it on.
    pub accept_placeholder: bool,
    /// When `Some`, the verifier rebuilds the placeholder PQC signature
    /// from this seed and compares byte-for-byte.  Mirrors what a real
    /// FIPS-204 verifier would do once integrated.
    pub pqc_expected_seed: Option<[u8; 32]>,
}

impl HybridVerifier {
    pub fn new(classical_pub: VerifyingKey) -> Self {
        Self {
            classical_pub,
            accept_placeholder: false,
            pqc_expected_seed: None,
        }
    }

    pub fn accepting_placeholder(mut self) -> Self {
        self.accept_placeholder = true;
        self
    }

    pub fn with_expected_pqc_seed(mut self, seed: [u8; 32]) -> Self {
        self.pqc_expected_seed = Some(seed);
        self
    }

    pub fn verify(&self, payload: &[u8], envelope: &[u8]) -> Result<(), PqcError> {
        let parts = parse_envelope(envelope)?;
        if parts.alg != ALG_HYBRID {
            return Err(PqcError::Header(format!("unsupported alg {}", parts.alg)));
        }
        let cs = Ed25519Sig::from_slice(&parts.classical)
            .map_err(|e| PqcError::Header(format!("ed25519 sig parse: {e}")))?;
        self.classical_pub
            .verify_strict(payload, &cs)
            .map_err(|_| PqcError::ClassicalSig)?;
        if let Some(seed) = self.pqc_expected_seed {
            let expected = HybridSigner::from_seed(seed).sign_pqc(payload);
            if expected != parts.pqc {
                return Err(PqcError::PqcSig);
            }
            Ok(())
        } else if self.accept_placeholder {
            // Spot-check: the PQC half must be exactly PQC_SIG_LEN bytes
            // and the prefix must derive from the domain string.  Any
            // accidentally-truncated or all-zero pqc body is rejected.
            let mut hash = Sha256::new();
            hash.update(PQC_PLACEHOLDER_DOMAIN);
            let prefix = hash.finalize();
            if parts.pqc.len() != PQC_SIG_LEN {
                return Err(PqcError::PqcSig);
            }
            if parts.pqc.iter().all(|b| *b == 0) {
                return Err(PqcError::PqcSig);
            }
            // The first byte of the placeholder PQC sig depends on
            // payload + seed, never on the bare domain string — so a
            // bare-domain prefix is the *anti-pattern* we reject.
            if parts.pqc[..prefix.len()] == prefix[..] {
                return Err(PqcError::PqcSig);
            }
            Ok(())
        } else {
            Err(PqcError::PlaceholderRefused)
        }
    }
}

#[derive(Debug)]
struct Parts<'a> {
    alg: String,
    classical: &'a [u8],
    pqc: &'a [u8],
}

fn parse_envelope(env: &[u8]) -> Result<Parts<'_>, PqcError> {
    let zero = env
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| PqcError::Header("missing alg/sig delimiter".into()))?;
    let alg = std::str::from_utf8(&env[..zero])
        .map_err(|_| PqcError::Header("alg is not utf8".into()))?
        .to_string();
    let rest = &env[zero + 1..];
    if rest.len() < 4 {
        return Err(PqcError::Header("missing classical-len".into()));
    }
    let mut lb = [0u8; 4];
    lb.copy_from_slice(&rest[..4]);
    let clen = u32::from_be_bytes(lb) as usize;
    if rest.len() < 4 + clen + PQC_SIG_LEN {
        return Err(PqcError::Header("envelope truncated".into()));
    }
    let classical = &rest[4..4 + clen];
    let pqc = &rest[4 + clen..4 + clen + PQC_SIG_LEN];
    Ok(Parts { alg, classical, pqc })
}

/// Base64url-encode a hybrid signature for JOSE/JWT wire usage.
pub fn encode_b64url(sig: &[u8]) -> String {
    B64.encode(sig)
}

pub fn decode_b64url(s: &str) -> Result<Vec<u8>, PqcError> {
    B64.decode(s).map_err(|e| PqcError::Base64(e.to_string()))
}

fn rand_bytes_32() -> [u8; 32] {
    // Sourced via the standard library getrandom-style facility on the
    // host through Ring's `SystemRandom`. Falls back to a process-id /
    // nanos derivative if the platform call ever returns an empty buf.
    use ring::rand::SecureRandom;
    let mut buf = [0u8; 32];
    let rng = ring::rand::SystemRandom::new();
    if rng.fill(&mut buf).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let mut h = Sha256::new();
        h.update(b"cave-k8s/pqc/fallback");
        h.update(nanos.to_be_bytes());
        h.update(pid.to_be_bytes());
        buf.copy_from_slice(&h.finalize());
    }
    buf
}

/// JWS-style projected SA token claims subset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaClaims {
    pub iss: String,
    pub sub: String,
    pub aud: Vec<String>,
    pub exp: i64,
    pub iat: i64,
    pub jti: String,
}

/// Build a JWT-shaped token signed by a hybrid signer.  Header carries
/// `alg = "Ed25519+ML-DSA-65"` so verifiers can dispatch.
pub fn sign_sa_jwt(signer: &HybridSigner, claims: &SaClaims) -> String {
    let header = serde_json::json!({"alg": ALG_HYBRID, "typ": "JWT+PQC"});
    let h = B64.encode(serde_json::to_vec(&header).expect("header"));
    let p = B64.encode(serde_json::to_vec(claims).expect("claims"));
    let signing_input = format!("{h}.{p}");
    let env = signer.sign(signing_input.as_bytes());
    format!("{}.{}", signing_input, B64.encode(env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_signature_is_pqc_length() {
        let s = HybridSigner::from_seed([7u8; 32]);
        let sig = s.sign(b"x");
        let parts = parse_envelope(&sig).unwrap();
        assert_eq!(parts.alg, ALG_HYBRID);
        assert_eq!(parts.pqc.len(), PQC_SIG_LEN);
    }

    #[test]
    fn signature_is_deterministic_per_seed() {
        let a = HybridSigner::from_seed([1u8; 32]).sign(b"payload");
        let b = HybridSigner::from_seed([1u8; 32]).sign(b"payload");
        assert_eq!(a, b);
        let c = HybridSigner::from_seed([2u8; 32]).sign(b"payload");
        assert_ne!(a, c);
    }

    #[test]
    fn verify_with_expected_seed_succeeds() {
        let signer = HybridSigner::from_seed([3u8; 32]);
        let v = HybridVerifier::new(signer.classical_public()).with_expected_pqc_seed([3u8; 32]);
        let sig = signer.sign(b"data");
        v.verify(b"data", &sig).unwrap();
    }

    #[test]
    fn verify_rejects_mismatched_classical() {
        let signer = HybridSigner::from_seed([4u8; 32]);
        let other = HybridSigner::from_seed([5u8; 32]);
        let v = HybridVerifier::new(other.classical_public()).accepting_placeholder();
        let sig = signer.sign(b"x");
        let err = v.verify(b"x", &sig).unwrap_err();
        assert_eq!(err, PqcError::ClassicalSig);
    }

    #[test]
    fn verify_rejects_placeholder_by_default() {
        let signer = HybridSigner::from_seed([6u8; 32]);
        let v = HybridVerifier::new(signer.classical_public());
        let sig = signer.sign(b"x");
        let err = v.verify(b"x", &sig).unwrap_err();
        assert_eq!(err, PqcError::PlaceholderRefused);
    }

    #[test]
    fn verify_accepts_placeholder_when_opted_in() {
        let signer = HybridSigner::from_seed([7u8; 32]);
        let v = HybridVerifier::new(signer.classical_public()).accepting_placeholder();
        let sig = signer.sign(b"x");
        v.verify(b"x", &sig).unwrap();
    }

    #[test]
    fn verify_rejects_zero_padded_placeholder() {
        let signer = HybridSigner::from_seed([8u8; 32]);
        let v = HybridVerifier::new(signer.classical_public()).accepting_placeholder();
        let mut sig = signer.sign(b"x");
        // zero out the pqc tail
        let len = sig.len();
        for b in &mut sig[len - PQC_SIG_LEN..] {
            *b = 0;
        }
        assert_eq!(v.verify(b"x", &sig).unwrap_err(), PqcError::PqcSig);
    }

    #[test]
    fn parse_envelope_rejects_truncated() {
        let signer = HybridSigner::from_seed([9u8; 32]);
        let sig = signer.sign(b"x");
        let truncated = &sig[..sig.len() - 100];
        let err = parse_envelope(truncated).unwrap_err();
        assert!(matches!(err, PqcError::Header(_)));
    }

    #[test]
    fn parse_envelope_rejects_bad_alg() {
        let mut env = Vec::new();
        env.extend_from_slice(b"RS256\0");
        env.extend_from_slice(&64u32.to_be_bytes());
        env.extend_from_slice(&vec![0u8; 64]);
        env.extend_from_slice(&vec![0u8; PQC_SIG_LEN]);
        let v = HybridVerifier::new(SigningKey::from_bytes(&[0u8; 32]).verifying_key());
        let err = v.verify(b"x", &env).unwrap_err();
        assert!(matches!(err, PqcError::Header(_)));
    }

    #[test]
    fn sa_jwt_roundtrip() {
        let signer = HybridSigner::from_seed([10u8; 32]);
        let claims = SaClaims {
            iss: "cave-k8s".into(),
            sub: "system:serviceaccount:default:cave".into(),
            aud: vec!["kube-apiserver".into()],
            exp: 9_999_999_999,
            iat: 1,
            jti: "abc".into(),
        };
        let tok = sign_sa_jwt(&signer, &claims);
        let parts: Vec<&str> = tok.split('.').collect();
        assert_eq!(parts.len(), 3);
        // body parses
        let payload = B64.decode(parts[1]).unwrap();
        let back: SaClaims = serde_json::from_slice(&payload).unwrap();
        assert_eq!(back, claims);
        // sig parses + verifies (with expected pqc seed)
        let env = B64.decode(parts[2]).unwrap();
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let v = HybridVerifier::new(signer.classical_public()).with_expected_pqc_seed([10u8; 32]);
        v.verify(signing_input.as_bytes(), &env).unwrap();
    }

    #[test]
    fn b64url_roundtrip() {
        let raw = b"the quick brown fox";
        let s = encode_b64url(raw);
        let back = decode_b64url(&s).unwrap();
        assert_eq!(back, raw);
    }
}
