// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-certs — PQC hybrid signer (ML-DSA-65 + Ed25519) tests.
//!
//! The Ed25519 half is real (ed25519-dalek); the ML-DSA-65 half is the
//! deterministic fixture documented in src/pqc.rs (real ML-DSA lands
//! when the rust-mldsa / oqs-rs dependency is wired in).

use cave_certs::pqc::{
    split_composite, verify_dual, HybridKeyPair, PqcError, CONTAINER_VERSION, MLDSA65_SIG_LEN,
};

const TENANT: &str = "tenant-acme-prod";

/// Cite: ADR-015 v2 — composite container packs both halves.
/// Total length must be `1 + 4 + 64 + 4 + MLDSA65_SIG_LEN` (Ed25519
/// signature is 64 bytes; ML-DSA-65 is 3309 bytes).
#[test]
fn dual_sign_round_trip_verifies_and_carries_both_halves() {
    let kp = HybridKeyPair::generate(format!("{}-signer", TENANT));
    let message = b"Cave Runtime - release manifest 2026-04-26";

    let composite = kp.dual_sign(message);
    assert_eq!(composite[0], CONTAINER_VERSION);
    assert_eq!(composite.len(), 1 + 4 + 64 + 4 + MLDSA65_SIG_LEN,
        "container = ver + 4-byte classical_len + 64-byte sig + 4-byte pqc_len + 3309-byte sig");

    let (classical, pqc) = split_composite(&composite).unwrap();
    assert_eq!(classical.len(), 64, "Ed25519 sig is 64 bytes");
    assert_eq!(pqc.len(), MLDSA65_SIG_LEN);

    verify_dual(&composite, message,
        &kp.classical_public_key(), &kp.pqc_key_seed, &kp.key_id).unwrap();
}

/// Cite: ADR-015 v2 verification policy — both halves MUST verify.
/// Tampering with EITHER half ⇒ rejection (defence in depth). Also,
/// version mismatch and truncation are caught explicitly.
#[test]
fn verification_rejects_tampering_and_version_mismatch() {
    let kp = HybridKeyPair::generate(format!("{}-signer", TENANT));
    let message = b"protected payload";
    let composite = kp.dual_sign(message);

    // Tamper with the classical half (bit-flip mid-signature).
    let mut tampered = composite.clone();
    tampered[10] ^= 0x01;
    let err = verify_dual(&tampered, message,
        &kp.classical_public_key(), &kp.pqc_key_seed, &kp.key_id).unwrap_err();
    assert_eq!(err, PqcError::ClassicalInvalid);

    // Tamper with the PQC half.
    let mut tampered = composite.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    let err = verify_dual(&tampered, message,
        &kp.classical_public_key(), &kp.pqc_key_seed, &kp.key_id).unwrap_err();
    assert_eq!(err, PqcError::PostQuantumInvalid);

    // Wrong message ⇒ classical fails first.
    let err = verify_dual(&composite, b"different message",
        &kp.classical_public_key(), &kp.pqc_key_seed, &kp.key_id).unwrap_err();
    assert_eq!(err, PqcError::ClassicalInvalid);

    // Version byte mismatch.
    let mut bad_version = composite.clone();
    bad_version[0] = 0xFF;
    let err = split_composite(&bad_version).unwrap_err();
    assert_eq!(err, PqcError::VersionMismatch { expected: CONTAINER_VERSION, actual: 0xFF });

    // Truncation
    let truncated = &composite[..50];
    let err = split_composite(truncated).unwrap_err();
    assert!(matches!(err, PqcError::Truncated { .. }));
}
