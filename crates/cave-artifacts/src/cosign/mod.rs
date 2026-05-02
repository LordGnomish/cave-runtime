//! Cosign-compatible signature module for cave-artifacts.
//!
//! Implements two algorithm tracks against the same wire surface:
//!
//! 1. **`ecdsa-p256`** — ECDSA over NIST P-256 (FIPS 186-5), real
//!    implementation backed by the [`p256`] crate. Sign produces a
//!    DER-encoded signature; verify takes the public key in SEC1
//!    uncompressed point encoding.
//!
//! 2. **`ml-dsa-65`** (a.k.a. dual-sign) — composite container holding
//!    both an Ed25519 signature (real) and an ML-DSA-65 signature
//!    (fixture today, slot reserved for a real PQC backend). This
//!    re-uses [`cave_certs::pqc`] verbatim so the workspace ships
//!    exactly one PQC composite implementation.
//!
//!    The workspace currently lacks a wired `pqcrypto-mldsa` /
//!    `oqs-rs` dependency — the ML-DSA half is a deterministic
//!    SHA-256-derived 3309-byte payload (FIPS 204 size). Swapping the
//!    fixture for a real signer is a contained change inside
//!    `cave_certs::pqc::pqc_fixture` once the dep lands.
//!
//! The signature container conforms to the Cosign "simple signing"
//! payload structure (cite: sigstore/cosign#1070, RFC 9162 §4):
//!
//! ```text
//! { "critical": { "identity": {...}, "image": {"docker-manifest-digest": "sha256:..."}, "type": "cosign container image signature" }, "optional": { ... } }
//! ```
//!
//! and is attached to the subject manifest by an OCI tag derived
//! from the digest (`sha256-XXXX.sig`).

pub mod manifest;
pub mod routes;
#[cfg(test)]
mod tests;

use cave_certs::pqc::{HybridKeyPair, PqcError, MLDSA65_SIG_LEN};
use ed25519_dalek::VerifyingKey;
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use p256::ecdsa::{Signature as P256Signature, SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey};
use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use routes::{router, CosignState};

pub const MODULE_NAME: &str = "cosign";

/// Algorithm selector for [`KeyPair`] / [`sign`] / [`verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Alg {
    /// FIPS 186-5 ECDSA over NIST P-256, real signer/verifier.
    EcdsaP256,
    /// ADR-015 hybrid: Ed25519 (real) + ML-DSA-65 (fixture today).
    /// Cite: draft-ietf-lamps-pq-composite-sigs §3 composite container.
    MlDsa65,
}

impl Alg {
    pub fn as_str(self) -> &'static str {
        match self {
            Alg::EcdsaP256 => "ecdsa-p256",
            Alg::MlDsa65 => "ml-dsa-65",
        }
    }

    pub fn parse(s: &str) -> Option<Alg> {
        match s.to_ascii_lowercase().as_str() {
            "ecdsa-p256" | "ecdsa_p256" | "ecdsa" => Some(Alg::EcdsaP256),
            "ml-dsa-65" | "ml_dsa_65" | "mldsa65" | "mldsa-65" | "pqc" | "hybrid" => Some(Alg::MlDsa65),
            _ => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum CosignError {
    #[error("unknown algorithm: {0}")]
    UnknownAlgorithm(String),

    #[error("key not found: {0}")]
    KeyNotFound(String),

    #[error("signature not found for digest: {0}")]
    SignatureNotFound(String),

    #[error("ECDSA signature verification failed")]
    EcdsaVerifyFailed,

    #[error("PQC composite verification failed: {0}")]
    PqcCompositeFailed(#[from] PqcError),

    #[error("malformed key material: {0}")]
    MalformedKey(String),

    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },

    #[error("payload missing required field: {0}")]
    PayloadInvalid(String),

    #[error("algorithm mismatch on verify: signature was {sig_alg}, key is {key_alg}")]
    AlgorithmMismatch { sig_alg: String, key_alg: String },
}

/// In-memory keypair handle. Real deployments pluck the secret half from
/// a KMS; the public half is what gets advertised.
pub enum KeyPair {
    EcdsaP256 {
        id: String,
        signing: P256SigningKey,
    },
    MlDsa65 {
        id: String,
        hybrid: HybridKeyPair,
    },
}

impl KeyPair {
    /// Generate a fresh keypair for the given algorithm. The ID is a
    /// UUID v4 so callers can track keys without external state.
    pub fn generate(alg: Alg) -> Self {
        let id = Uuid::new_v4().to_string();
        match alg {
            Alg::EcdsaP256 => {
                let signing = P256SigningKey::random(&mut rand::rngs::OsRng);
                KeyPair::EcdsaP256 { id, signing }
            }
            Alg::MlDsa65 => {
                let hybrid = HybridKeyPair::generate(id.clone());
                KeyPair::MlDsa65 { id, hybrid }
            }
        }
    }

    pub fn id(&self) -> &str {
        match self {
            KeyPair::EcdsaP256 { id, .. } | KeyPair::MlDsa65 { id, .. } => id,
        }
    }

    pub fn alg(&self) -> Alg {
        match self {
            KeyPair::EcdsaP256 { .. } => Alg::EcdsaP256,
            KeyPair::MlDsa65 { .. } => Alg::MlDsa65,
        }
    }

    /// Public-key handle suitable for advertising. ECDSA returns the
    /// SubjectPublicKeyInfo PEM. PQC returns a JSON envelope carrying
    /// the Ed25519 verifying key (base64) plus the ML-DSA fixture seed
    /// reference (the seed itself stays secret in real deployments —
    /// here the test surface exposes it for round-trip verification).
    pub fn public_handle(&self) -> Result<PublicKeyHandle, CosignError> {
        match self {
            KeyPair::EcdsaP256 { id, signing } => {
                let vk = signing.verifying_key();
                let pem = vk
                    .to_public_key_pem(p256::pkcs8::LineEnding::LF)
                    .map_err(|e| CosignError::MalformedKey(e.to_string()))?;
                Ok(PublicKeyHandle::EcdsaP256 {
                    id: id.clone(),
                    spki_pem: pem,
                })
            }
            KeyPair::MlDsa65 { id, hybrid } => {
                let classical_pub = hybrid.classical_public_key();
                Ok(PublicKeyHandle::MlDsa65 {
                    id: id.clone(),
                    classical_b64: base64_std(classical_pub.as_bytes()),
                    pqc_seed_b64: base64_std(&hybrid.pqc_key_seed),
                    expected_pqc_len: MLDSA65_SIG_LEN,
                })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "alg", rename_all = "kebab-case")]
pub enum PublicKeyHandle {
    EcdsaP256 {
        id: String,
        /// PEM-encoded SubjectPublicKeyInfo (PKCS#8 SPKI), the same
        /// thing `openssl ec -pubout` emits.
        spki_pem: String,
    },
    MlDsa65 {
        id: String,
        /// Base64 of the 32-byte Ed25519 public key.
        classical_b64: String,
        /// Base64 of the 32-byte ML-DSA fixture seed (real PQC
        /// deployments will not export this — see module docs).
        pqc_seed_b64: String,
        expected_pqc_len: usize,
    },
}

impl PublicKeyHandle {
    pub fn id(&self) -> &str {
        match self {
            PublicKeyHandle::EcdsaP256 { id, .. } | PublicKeyHandle::MlDsa65 { id, .. } => id,
        }
    }

    pub fn alg(&self) -> Alg {
        match self {
            PublicKeyHandle::EcdsaP256 { .. } => Alg::EcdsaP256,
            PublicKeyHandle::MlDsa65 { .. } => Alg::MlDsa65,
        }
    }
}

/// Wire-format signature emitted by [`sign`]. ECDSA signatures use the
/// fixed-size IEEE-P1363 (r||s) encoding; PQC signatures carry the full
/// composite container as produced by `cave_certs::pqc::dual_sign`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "alg", rename_all = "kebab-case")]
pub enum Signature {
    EcdsaP256 {
        key_id: String,
        /// Base64 of the IEEE-P1363 encoded ECDSA signature (64 bytes).
        sig_b64: String,
    },
    MlDsa65 {
        key_id: String,
        /// Base64 of the composite container `[ver][len][ed25519]
        /// [len][mldsa]`.
        composite_b64: String,
    },
}

impl Signature {
    pub fn alg(&self) -> Alg {
        match self {
            Signature::EcdsaP256 { .. } => Alg::EcdsaP256,
            Signature::MlDsa65 { .. } => Alg::MlDsa65,
        }
    }

    pub fn key_id(&self) -> &str {
        match self {
            Signature::EcdsaP256 { key_id, .. } | Signature::MlDsa65 { key_id, .. } => key_id,
        }
    }
}

/// Sign `payload` with `key`. Pure function over the bytes — the caller
/// is responsible for constructing the Cosign payload envelope first
/// (use [`manifest::build_payload`]) and providing the JSON bytes.
pub fn sign(key: &KeyPair, payload: &[u8]) -> Result<Signature, CosignError> {
    match key {
        KeyPair::EcdsaP256 { id, signing } => {
            let sig: P256Signature = signing.sign(payload);
            Ok(Signature::EcdsaP256 {
                key_id: id.clone(),
                sig_b64: base64_std(&sig.to_bytes()),
            })
        }
        KeyPair::MlDsa65 { id, hybrid } => {
            let composite = hybrid.dual_sign(payload);
            Ok(Signature::MlDsa65 {
                key_id: id.clone(),
                composite_b64: base64_std(&composite),
            })
        }
    }
}

/// Verify `payload` against the signature using the public-key handle
/// returned earlier from [`KeyPair::public_handle`]. Returns Ok on
/// success, an error variant on any mismatch — never panics.
pub fn verify(
    key: &PublicKeyHandle,
    payload: &[u8],
    sig: &Signature,
) -> Result<(), CosignError> {
    if key.alg() != sig.alg() {
        return Err(CosignError::AlgorithmMismatch {
            sig_alg: sig.alg().as_str().into(),
            key_alg: key.alg().as_str().into(),
        });
    }
    match (key, sig) {
        (PublicKeyHandle::EcdsaP256 { spki_pem, .. }, Signature::EcdsaP256 { sig_b64, .. }) => {
            let vk = P256VerifyingKey::from_public_key_pem(spki_pem)
                .map_err(|e| CosignError::MalformedKey(e.to_string()))?;
            let sig_bytes = base64_decode(sig_b64)?;
            let sig = P256Signature::from_slice(&sig_bytes)
                .map_err(|_| CosignError::EcdsaVerifyFailed)?;
            vk.verify(payload, &sig)
                .map_err(|_| CosignError::EcdsaVerifyFailed)
        }
        (
            PublicKeyHandle::MlDsa65 {
                id,
                classical_b64,
                pqc_seed_b64,
                ..
            },
            Signature::MlDsa65 { composite_b64, .. },
        ) => {
            let classical_bytes = base64_decode(classical_b64)?;
            let classical_arr: [u8; 32] = classical_bytes
                .as_slice()
                .try_into()
                .map_err(|_| CosignError::MalformedKey("classical pubkey != 32 bytes".into()))?;
            let classical_pub = VerifyingKey::from_bytes(&classical_arr)
                .map_err(|e| CosignError::MalformedKey(e.to_string()))?;
            let seed_bytes = base64_decode(pqc_seed_b64)?;
            let seed_arr: [u8; 32] = seed_bytes
                .as_slice()
                .try_into()
                .map_err(|_| CosignError::MalformedKey("pqc seed != 32 bytes".into()))?;
            let composite = base64_decode(composite_b64)?;
            cave_certs::pqc::verify_dual(&composite, payload, &classical_pub, &seed_arr, id)
                .map_err(CosignError::PqcCompositeFailed)
        }
        _ => unreachable!("alg mismatch caught above"),
    }
}

fn base64_std(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.encode(bytes)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, CosignError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD
        .decode(s.trim())
        .map_err(|e| CosignError::MalformedKey(format!("base64: {e}")))
}
