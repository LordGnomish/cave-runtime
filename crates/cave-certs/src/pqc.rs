// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Post-quantum hybrid signer — ML-DSA-65 (FIPS 204) + Ed25519 dual-sign.
//!
//! Cite: ADR-015 v2 (Cave PQC migration plan); IETF
//! draft-ietf-lamps-pq-composite-sigs §3 (composite signature value =
//! concatenation of the two component signatures, length-prefixed); FIPS 204
//! §3 (ML-DSA-65 signature size = 3309 bytes).
//!
//! cave's scaffold today exposes the dual-signature CONTAINER
//! (assemble + decompose + verify-shape) without invoking a real
//! ML-DSA implementation — the Ed25519 half is real (cave depends on
//! `ed25519-dalek`), the ML-DSA half is a deterministic fixture
//! produced by SHA-256 over the message + key id. The container
//! interface is what real PQC libraries (rust-mldsa, oqs-rs) will
//! plug into.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PqcError {
    #[error("composite signature truncated (expected ≥ {expected} bytes, got {actual})")]
    Truncated { expected: usize, actual: usize },
    #[error("classical signature verification failed")]
    ClassicalInvalid,
    #[error("post-quantum signature verification failed")]
    PostQuantumInvalid,
    #[error("composite version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u8, actual: u8 },
}

/// Cite: FIPS 204 — ML-DSA-65 signature size is 3309 bytes. cave's
/// fixture mode emits a deterministic 3309-byte payload derived from
/// SHA-256(message || key_id) and 16-byte zero-padded chunks; the real
/// signer will replace this with a true ML-DSA-65 signature.
pub const MLDSA65_SIG_LEN: usize = 3309;

/// Container format version 1: `[0x01][32-bit BE classical_len][classical bytes][32-bit BE pqc_len][pqc bytes]`.
pub const CONTAINER_VERSION: u8 = 0x01;

#[derive(Debug, Clone)]
pub struct HybridKeyPair {
    pub key_id: String,
    pub classical: SigningKey,
    pub pqc_key_seed: [u8; 32],
}

impl HybridKeyPair {
    pub fn generate(key_id: impl Into<String>) -> Self {
        use rand::RngCore as _;
        let mut rng = OsRng;
        let mut secret = [0u8; 32];
        rng.fill_bytes(&mut secret);
        let classical = SigningKey::from_bytes(&secret);
        let mut pqc_seed = [0u8; 32];
        rng.fill_bytes(&mut pqc_seed);
        Self { key_id: key_id.into(), classical, pqc_key_seed: pqc_seed }
    }

    pub fn classical_public_key(&self) -> VerifyingKey {
        self.classical.verifying_key()
    }

    /// Cite: ADR-015 v2 — dual-sign produces both a classical (Ed25519)
    /// signature AND a fixture ML-DSA-65 signature, packed in the
    /// composite container.
    pub fn dual_sign(&self, message: &[u8]) -> Vec<u8> {
        let classical_sig: Signature = self.classical.sign(message);
        let classical_bytes = classical_sig.to_bytes();

        let pqc_bytes = pqc_fixture(&self.pqc_key_seed, &self.key_id, message);

        let mut out = Vec::with_capacity(1 + 4 + classical_bytes.len() + 4 + pqc_bytes.len());
        out.push(CONTAINER_VERSION);
        out.extend_from_slice(&(classical_bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&classical_bytes);
        out.extend_from_slice(&(pqc_bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&pqc_bytes);
        out
    }
}

/// Cite: draft-ietf-lamps-pq-composite-sigs §3 — split the composite
/// container back into its components.
pub fn split_composite(blob: &[u8]) -> Result<(Vec<u8>, Vec<u8>), PqcError> {
    if blob.is_empty() {
        return Err(PqcError::Truncated { expected: 5, actual: 0 });
    }
    if blob[0] != CONTAINER_VERSION {
        return Err(PqcError::VersionMismatch {
            expected: CONTAINER_VERSION, actual: blob[0],
        });
    }
    if blob.len() < 1 + 4 {
        return Err(PqcError::Truncated { expected: 5, actual: blob.len() });
    }
    let classical_len = u32::from_be_bytes(blob[1..5].try_into().unwrap()) as usize;
    let classical_end = 5 + classical_len;
    if blob.len() < classical_end + 4 {
        return Err(PqcError::Truncated {
            expected: classical_end + 4, actual: blob.len(),
        });
    }
    let classical = blob[5..classical_end].to_vec();
    let pqc_len = u32::from_be_bytes(
        blob[classical_end..classical_end + 4].try_into().unwrap()
    ) as usize;
    let pqc_end = classical_end + 4 + pqc_len;
    if blob.len() < pqc_end {
        return Err(PqcError::Truncated { expected: pqc_end, actual: blob.len() });
    }
    let pqc = blob[classical_end + 4..pqc_end].to_vec();
    Ok((classical, pqc))
}

/// Cite: ADR-015 v2 verification policy — both halves MUST verify
/// (defence-in-depth). A failure on either half ⇒ reject.
pub fn verify_dual(
    composite: &[u8],
    message: &[u8],
    classical_pub: &VerifyingKey,
    pqc_seed: &[u8; 32],
    key_id: &str,
) -> Result<(), PqcError> {
    let (classical_bytes, pqc_bytes) = split_composite(composite)?;
    let classical_sig = Signature::from_slice(&classical_bytes)
        .map_err(|_| PqcError::ClassicalInvalid)?;
    classical_pub.verify(message, &classical_sig)
        .map_err(|_| PqcError::ClassicalInvalid)?;

    let expected = pqc_fixture(pqc_seed, key_id, message);
    if expected != pqc_bytes {
        return Err(PqcError::PostQuantumInvalid);
    }
    Ok(())
}

/// Deterministic ML-DSA fixture. Real ML-DSA-65 is non-deterministic by
/// default; this fixture exists only so the composite container can be
/// exercised end-to-end on hosts without a PQC implementation.
fn pqc_fixture(seed: &[u8; 32], key_id: &str, message: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(seed);
    h.update([0]);
    h.update(key_id.as_bytes());
    h.update([0]);
    h.update(message);
    let digest = h.finalize();
    let mut out = Vec::with_capacity(MLDSA65_SIG_LEN);
    while out.len() + digest.len() <= MLDSA65_SIG_LEN {
        out.extend_from_slice(&digest);
    }
    if out.len() < MLDSA65_SIG_LEN {
        out.extend_from_slice(&digest[..MLDSA65_SIG_LEN - out.len()]);
    }
    out
}
