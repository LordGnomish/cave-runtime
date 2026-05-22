// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shamir secret sharing + seal lifecycle — parity tests against
//! openbao v2.5.3.
//!
//! Upstream packages: `sdk/helper/shamir/shamir.go` (split/combine) and
//! `vault/seal.go` (initialize/unseal/seal state machine).

use cave_vault::core::seal::{SealState, SealStatus, combine_shares, split_secret};

/// Cite: openbao `sdk/helper/shamir/shamir.go:192` (Split) +
/// `sdk/helper/shamir/shamir.go:251` (Combine) — round-trip a 32-byte
/// secret through Shamir(5, 3) using shares 1, 3 and 5 (mid-cluster
/// disaster-recovery scenario).
#[test]
fn shamir_5_of_3_round_trip_picking_disjoint_shares() {
    let secret: [u8; 32] = *b"openbao master key abcdefghijklm";
    let shares = split_secret(&secret, 5, 3).expect("split");
    assert_eq!(shares.len(), 5);

    let pick = vec![shares[0].clone(), shares[2].clone(), shares[4].clone()];
    let recovered = combine_shares(&pick).expect("combine");
    assert_eq!(recovered, secret.to_vec());
}

/// Cite: openbao `sdk/helper/shamir/shamir.go:251` (Combine) — fewer than
/// `threshold` shares MUST NOT recover the secret. We assert at minimum
/// that the wrong reconstruction differs from the original.
#[test]
fn shamir_below_threshold_does_not_reconstruct() {
    let secret: [u8; 16] = *b"sixteen-byte-key";
    let shares = split_secret(&secret, 5, 3).unwrap();

    // 2 shares (k-1) — combine still produces SOMETHING but it MUST
    // not equal the secret (it's a degree-2 polynomial under-determined).
    let only_two = vec![shares[0].clone(), shares[1].clone()];
    let attempt = combine_shares(&only_two).unwrap();
    assert_ne!(
        attempt,
        secret.to_vec(),
        "k-1 shares cannot recover the secret"
    );
}

/// Cite: openbao `sdk/helper/shamir/shamir.go:192` (Split) — invalid
/// parameters (k > n, k < 2) are rejected without leaking shares.
#[test]
fn shamir_rejects_invalid_share_parameters() {
    let secret = b"abc";
    assert!(split_secret(secret, 3, 5).is_err(), "k > n is illegal");
    assert!(split_secret(secret, 3, 1).is_err(), "k < 2 is illegal");
}

/// Cite: openbao `vault/seal.go:51` (Seal interface) and
/// `vault/seal.go:114` (defaultSeal.Init) — initialize sets up Shamir
/// parameters, generates the master key + root token, and leaves the
/// vault SEALED awaiting unseal shares.
#[test]
fn seal_initialize_5_of_3_generates_root_and_keeps_sealed() {
    let mut s = SealState::default();
    let (root, shares) = s.initialize(5, 3).unwrap();
    assert!(root.starts_with("hvs."));
    assert_eq!(shares.len(), 5);
    assert!(s.is_initialized());
    assert!(
        s.is_sealed(),
        "newly initialized vault remains sealed until threshold shares submitted"
    );
    assert_eq!(s.threshold, 3);
    assert_eq!(s.shares, 5);
}

/// Cite: openbao `vault/seal.go` unseal flow — submitting `threshold`
/// shares unseals the vault. Below threshold returns `false` (still sealed)
/// and accumulates progress; reaching threshold returns `true` and
/// transitions the state to Unsealed.
#[test]
fn seal_unseal_progress_then_unsealed_at_threshold() {
    let mut s = SealState::default();
    let (_root, shares) = s.initialize(5, 3).unwrap();

    let r1 = s.unseal(&shares[0]).unwrap();
    assert!(!r1, "1/3 — still sealed");
    assert_eq!(s.unseal_progress, 1);

    let r2 = s.unseal(&shares[2]).unwrap();
    assert!(!r2, "2/3 — still sealed");
    assert_eq!(s.unseal_progress, 2);

    let r3 = s.unseal(&shares[4]).unwrap();
    assert!(r3, "3/3 — unsealed");
    assert!(!s.is_sealed());
    assert_eq!(s.status, SealStatus::Unsealed);
    assert_eq!(s.unseal_progress, 0, "progress is reset after unseal");
}

/// Cite: openbao `vault/seal.go` reseal flow — calling seal() on an
/// unsealed vault wipes the in-memory master key and rotates the unseal
/// nonce, requiring a fresh threshold of shares to unseal again.
#[test]
fn reseal_clears_master_key_and_rotates_nonce() {
    let mut s = SealState::default();
    let (_, shares) = s.initialize(5, 3).unwrap();
    for sh in shares.iter().take(3) {
        s.unseal(sh).unwrap();
    }
    assert!(!s.is_sealed());
    let nonce_before = s.unseal_nonce.clone();
    assert!(s.master_key.is_some());

    s.seal();
    assert!(s.is_sealed());
    assert!(s.master_key.is_none(), "master key wiped on reseal");
    assert_ne!(s.unseal_nonce, nonce_before, "nonce rotated");
    assert_eq!(s.unseal_progress, 0);
}
