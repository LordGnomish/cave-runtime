// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PQC auto-seal lifecycle: ML-KEM-768 wraps the barrier master key for
//! automatic unseal, while a Shamir recovery-share quorum backs the master key
//! up for the quorum-loss / migration case — mirroring OpenBao auto-seal
//! (auto-unseal via wrapper + recovery-key flow).

use cave_vault::core::pqc_seal::PqcSeal;
use cave_vault::core::seal::AutoSealType;

#[test]
fn mlkem768_is_a_recovery_key_auto_seal() {
    let t = AutoSealType::MlKem768;
    assert_eq!(t.barrier_type(), "mlkem768");
    assert!(t.stores_keys_remotely());
    assert!(t.supports_recovery_key());
    assert_ne!(t, AutoSealType::Shamir);
}

#[test]
fn initialize_then_auto_unseal_recovers_same_master_key() {
    let mut seal = PqcSeal::generate();
    let init = seal.initialize(5, 3).unwrap();
    assert_eq!(init.recovery_shares.len(), 5);
    assert!(init.root_token.starts_with("hvs."));
    assert_eq!(init.master_key.len(), 32);

    // Auto-unseal: the process holds the decapsulation key, so it unwraps the
    // stored envelope with no human interaction.
    let mk = seal.auto_unseal().unwrap();
    assert_eq!(mk, init.master_key);
}

#[test]
fn recovery_quorum_reconstructs_master_key() {
    let mut seal = PqcSeal::generate();
    let init = seal.initialize(5, 3).unwrap();

    // Any 3 of the 5 recovery shares reconstruct the master key.
    let subset = vec![
        init.recovery_shares[4].clone(),
        init.recovery_shares[1].clone(),
        init.recovery_shares[2].clone(),
    ];
    let recovered = PqcSeal::recover_master_key(&subset).unwrap();
    assert_eq!(recovered, init.master_key);
}

#[test]
fn recovery_with_fewer_than_threshold_does_not_recover() {
    let mut seal = PqcSeal::generate();
    let init = seal.initialize(5, 3).unwrap();
    let too_few = vec![init.recovery_shares[0].clone(), init.recovery_shares[1].clone()];
    // Two shares cannot reconstruct a 3-of-5 secret: either an error or a
    // wrong value, but never the real master key.
    match PqcSeal::recover_master_key(&too_few) {
        Ok(v) => assert_ne!(v, init.master_key),
        Err(_) => {}
    }
}

#[test]
fn auto_unseal_before_initialize_errors() {
    let seal = PqcSeal::generate();
    assert!(seal.auto_unseal().is_err());
}

#[test]
fn seed_persisted_keypair_still_auto_unseals() {
    // Persist the keypair seed + wrapped envelope, drop the live seal, and
    // restore it: auto-unseal must still recover the master key.
    let (seed, wrapped_json, master_key) = {
        let mut seal = PqcSeal::generate();
        let init = seal.initialize(3, 2).unwrap();
        (seal.seed_bytes(), seal.wrapped_key_json().unwrap(), init.master_key)
    };
    let restored = PqcSeal::from_persisted(&seed, &wrapped_json).unwrap();
    assert_eq!(restored.auto_unseal().unwrap(), master_key);
}
