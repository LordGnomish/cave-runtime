// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Acceptance tests for the post-quantum (ML-KEM-768) seal-wrap barrier.
//!
//! The PQC seal wraps the Vault barrier master key under an ML-KEM-768
//! (FIPS 203, NIST category 3) key-encapsulation mechanism in a KEM-DEM
//! hybrid envelope. These tests drive the public API end-to-end:
//! keypair generation, wrap → unwrap round-trip, tamper detection,
//! wrong-key rejection, and deterministic reproducibility from a seed.

use cave_vault::core::pqc_seal::{PqcSealKeypair, PqcWrappedKey};

#[test]
fn wrap_then_unwrap_recovers_master_key() {
    let kp = PqcSealKeypair::generate();
    let master_key = b"32-byte vault barrier master key".to_vec();
    assert_eq!(master_key.len(), 32);

    let wrapped: PqcWrappedKey = kp.seal_wrap(&master_key).unwrap();
    // The wrapped form must NOT leak the plaintext master key.
    assert!(!wrapped.ciphertext.windows(master_key.len()).any(|w| w == &master_key[..]));
    // ML-KEM-768 ciphertext is 1088 bytes; AES-GCM nonce is 12 bytes.
    assert_eq!(wrapped.kem_ciphertext.len(), 1088);
    assert_eq!(wrapped.nonce.len(), 12);

    let recovered = kp.seal_unwrap(&wrapped).unwrap();
    assert_eq!(recovered, master_key);
}

#[test]
fn wrap_is_randomised_per_call() {
    let kp = PqcSealKeypair::generate();
    let mk = b"another 32 byte master key here!!".to_vec();
    let a = kp.seal_wrap(&mk).unwrap();
    let b = kp.seal_wrap(&mk).unwrap();
    // Fresh encapsulation each call → different KEM ciphertext and nonce.
    assert_ne!(a.kem_ciphertext, b.kem_ciphertext);
    assert_ne!(a.nonce, b.nonce);
    // Both still unwrap to the same master key.
    assert_eq!(kp.seal_unwrap(&a).unwrap(), mk);
    assert_eq!(kp.seal_unwrap(&b).unwrap(), mk);
}

#[test]
fn tampered_ciphertext_fails_to_unwrap() {
    let kp = PqcSealKeypair::generate();
    let mk = b"master-key-for-tamper-detection!".to_vec();
    let mut wrapped = kp.seal_wrap(&mk).unwrap();
    // Flip one bit of the AES-GCM ciphertext: the auth tag must reject it.
    wrapped.ciphertext[0] ^= 0x01;
    assert!(kp.seal_unwrap(&wrapped).is_err());
}

#[test]
fn tampered_kem_ciphertext_fails_to_unwrap() {
    let kp = PqcSealKeypair::generate();
    let mk = b"master-key-for-kem-ct-tampering!".to_vec();
    let mut wrapped = kp.seal_wrap(&mk).unwrap();
    // Mutating the KEM ciphertext yields a different shared secret →
    // a different AES key → GCM tag mismatch.
    wrapped.kem_ciphertext[10] ^= 0xFF;
    assert!(kp.seal_unwrap(&wrapped).is_err());
}

#[test]
fn wrong_keypair_cannot_unwrap() {
    let alice = PqcSealKeypair::generate();
    let mallory = PqcSealKeypair::generate();
    let mk = b"only-alice-should-recover-this!!!".to_vec();
    let wrapped = alice.seal_wrap(&mk).unwrap();
    assert!(mallory.seal_unwrap(&wrapped).is_err());
    assert_eq!(alice.seal_unwrap(&wrapped).unwrap(), mk);
}

#[test]
fn keypair_round_trips_through_seed_bytes() {
    let kp = PqcSealKeypair::generate();
    let seed = kp.seed_bytes();
    assert_eq!(seed.len(), 64);

    // A keypair restored from the seed must unwrap what the original sealed.
    let mk = b"seed-restored-keypair-master-key".to_vec();
    let wrapped = kp.seal_wrap(&mk).unwrap();

    let restored = PqcSealKeypair::from_seed_bytes(&seed);
    assert_eq!(restored.public_key_bytes(), kp.public_key_bytes());
    assert_eq!(restored.seal_unwrap(&wrapped).unwrap(), mk);
}

#[test]
fn public_key_is_ml_kem_768_size() {
    let kp = PqcSealKeypair::generate();
    // ML-KEM-768 encapsulation key is 1184 bytes.
    assert_eq!(kp.public_key_bytes().len(), 1184);
}

#[test]
fn anyone_with_public_key_can_seal_for_holder() {
    // Separation of duties: an operator with only the public key can wrap
    // the master key; only the decapsulation-key holder can unwrap.
    let holder = PqcSealKeypair::generate();
    let pubkey = holder.public_key_bytes();

    let mk = b"sealed-by-public-key-only-please".to_vec();
    let wrapped = PqcSealKeypair::seal_wrap_to_public(&pubkey, &mk).unwrap();
    assert_eq!(holder.seal_unwrap(&wrapped).unwrap(), mk);
}
