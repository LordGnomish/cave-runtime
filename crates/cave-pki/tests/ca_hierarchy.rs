// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-pki — Root → Platform → Tenant CA hierarchy tests.

use cave_pki::{Ca, CaKind, KeyAlgorithm, PkiError};

const TENANT_A: &str = "tenant-acme-prod";
const TENANT_B: &str = "tenant-beta-staging";

/// Cite: openbao `pki/path_root.go::pathCAGenerateRoot` —
/// single-root invariant (re-issuing returns BackendError).
#[test]
fn root_ca_can_only_be_generated_once() {
    let mut ca = Ca::new();
    let root1 = ca.generate_root("Cave Sovereign Root", KeyAlgorithm::EcdsaP384, 20).unwrap();
    let err = ca.generate_root("Cave Sovereign Root", KeyAlgorithm::EcdsaP384, 20).unwrap_err();
    assert_eq!(err, PkiError::RootAlreadyExists);

    let handle = ca.handle(&root1).unwrap();
    assert_eq!(handle.kind, CaKind::Root);
    assert!(handle.hardware_backed, "Root CA key MUST be HSM-backed (NIST SP 800-57)");
    assert_eq!(ca.root_serial(), Some(root1.as_str()));
    let _ = TENANT_A;
}

/// Cite: openbao `pki/path_intermediate.go::pathGenerateIntermediate`
/// — platform intermediate requires a root parent.
#[test]
fn platform_intermediate_requires_root_parent() {
    let mut ca = Ca::new();
    let err = ca.generate_platform_intermediate("Cave Platform CA", KeyAlgorithm::EcdsaP384)
        .unwrap_err();
    assert_eq!(err, PkiError::ParentNotFound("root".into()));

    ca.generate_root("Cave Sovereign Root", KeyAlgorithm::EcdsaP384, 20).unwrap();
    let plat = ca.generate_platform_intermediate("Cave Platform CA", KeyAlgorithm::EcdsaP384).unwrap();
    let handle = ca.handle(&plat).unwrap();
    assert_eq!(handle.kind, CaKind::PlatformIntermediate);
    assert_eq!(handle.issuer_serial.as_deref(), Some(ca.root_serial().unwrap()));
    assert!(!handle.hardware_backed,
        "platform intermediate is online; only root requires HSM");
}

/// Cite: openbao `pki/path_intermediate.go` + cave invariant — exactly
/// one tenant intermediate per tenant. Re-issuance is idempotent.
#[test]
fn tenant_intermediate_is_one_per_tenant_idempotent() {
    let mut ca = Ca::new();
    ca.generate_root("Cave Sovereign Root", KeyAlgorithm::EcdsaP384, 20).unwrap();
    let plat = ca.generate_platform_intermediate("Cave Platform CA", KeyAlgorithm::EcdsaP384).unwrap();

    let t_a = ca.generate_tenant_intermediate(TENANT_A, KeyAlgorithm::EcdsaP256).unwrap();
    let t_a_dup = ca.generate_tenant_intermediate(TENANT_A, KeyAlgorithm::EcdsaP256).unwrap();
    assert_eq!(t_a, t_a_dup, "second issuance is idempotent");

    let t_b = ca.generate_tenant_intermediate(TENANT_B, KeyAlgorithm::Ed25519).unwrap();
    assert_ne!(t_a, t_b);
    assert_eq!(ca.tenant_count(), 2);

    // Each tenant intermediate is rooted at the platform intermediate.
    let t_a_handle = ca.handle(&t_a).unwrap();
    assert_eq!(t_a_handle.issuer_serial.as_deref(), Some(plat.as_str()));
    assert_eq!(t_a_handle.tenant_id, TENANT_A);
}

/// Cite: openbao `pki/path_keys.go` algorithm enum + cave PQC ADR —
/// every named key algorithm parses, and the PQC hybrid is recognised.
#[test]
fn key_algorithm_parsing_includes_pqc_hybrid() {
    use KeyAlgorithm::*;
    for (s, expected) in [
        ("ecdsa-p256", EcdsaP256),
        ("p384", EcdsaP384),
        ("rsa-2048", Rsa2048),
        ("RSA4096", Rsa4096),
        ("ed25519", Ed25519),
        ("hybrid-mldsa65-ed25519", HybridMlDsa65Ed25519),
        ("pqc-hybrid", HybridMlDsa65Ed25519),
    ] {
        assert_eq!(KeyAlgorithm::parse(s).unwrap(), expected);
    }
    assert!(KeyAlgorithm::parse("not-an-algo").is_err());
    assert!(EcdsaP256.requires_hsm_for_root(), "every root MUST be HSM-backed");
}
