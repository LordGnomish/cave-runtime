// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-sign smoke tests — exercise the four user-required scenarios:
//!   1. Keypair sign + verify roundtrip
//!   2. Rekor log entry mock + inclusion proof
//!   3. SLSA attestation fixture (DSSE + Statement v1)
//!   4. Bundle format parse + emit

use cave_sign::attestation::{
    DsseEnvelope, build_slsa_provenance, dsse_pae, sign_attestation, subject_sha256,
    verify_envelope,
};
use cave_sign::blob::{sign_blob_keypair, sign_blob_keypair_with_rekor, verify_blob};
use cave_sign::bundle::{BundleTriple, CosignBundle};
use cave_sign::keyless::KeylessSigner;
use cave_sign::fulcio::FulcioClient;
use cave_sign::models::{Attestation, KeyAlgorithm, PredicateType, SigKind};
use cave_sign::oidc::{IdToken, build_fixture_jwt};
use cave_sign::policy::{Policy, Rule};
use cave_sign::rekor::RekorClient;
use cave_sign::signature::Keypair;
use cave_sign::tlog::{build_witness, verify_against_rekor};
use cave_sign::verify::{VerifyRequest, verify};
use serde_json::json;

#[test]
fn smoke_1_keypair_sign_verify_roundtrip() {
    for alg in [KeyAlgorithm::EcdsaP256, KeyAlgorithm::Ed25519] {
        let kp = Keypair::from_seed(alg, &[7u8; 32]).unwrap();
        let payload = b"smoke: keypair roundtrip";
        let result = sign_blob_keypair(payload, &kp).unwrap();
        assert!(result.artifact_digest.starts_with("sha256:"));
        assert_eq!(result.signature.kind, SigKind::Keypair);
        verify_blob(payload, &result.bundle).unwrap();
        let tampered: Vec<u8> = payload.iter().copied().chain(std::iter::once(0xFF)).collect();
        assert!(verify_blob(&tampered, &result.bundle).is_err());
    }
}

#[test]
fn smoke_2_rekor_log_entry_mock_with_inclusion_proof() {
    let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[8u8; 32]).unwrap();
    let rk = RekorClient::default();
    // Build 4 entries → ensure the witness tree is non-trivial.
    let mut sigs = Vec::new();
    for i in 0..4 {
        sigs.push(sign_blob_keypair_with_rekor(&[i; 16], &kp, &rk).unwrap());
    }
    for s in &sigs {
        assert!(s.signature.log_index.is_some());
        verify_against_rekor(&s.bundle, &rk).unwrap();
    }
    let witness = build_witness(&sigs[2].bundle, &rk).unwrap();
    assert_eq!(witness.tree_size, 4);
    assert_eq!(witness.log_index, 2);
    assert_eq!(witness.sibling_hashes.len(), 2);
}

#[test]
fn smoke_3_slsa_attestation_fixture() {
    let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[9u8; 32]).unwrap();
    let att = Attestation {
        media_type: "application/vnd.in-toto+json".into(),
        predicate_type: PredicateType::SlsaProvenance,
        subject: vec![subject_sha256(
            "ghcr.io/cave/runtime",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )],
        predicate: build_slsa_provenance(
            "https://github.com/cave/runtime/.github/workflows/build.yml@refs/heads/main",
            "https://github.com/slsa-framework/slsa-github-generator/.../v2.1.0",
            "https://github.com/cave/runtime/actions/runs/9999999999",
        ),
    };
    let env: DsseEnvelope = sign_attestation(&att, &kp, "cave-keyid-smoke").unwrap();
    assert_eq!(env.payload_type, "application/vnd.in-toto+json");
    assert_eq!(env.signatures.len(), 1);
    let back = verify_envelope(&env, KeyAlgorithm::EcdsaP256, kp.public_key_bytes()).unwrap();
    assert_eq!(back.predicate_type, PredicateType::SlsaProvenance);
    assert_eq!(back.subject.len(), 1);
    let pae = dsse_pae("t", b"hi");
    assert_eq!(pae, b"DSSEv1 1 t 2 hi");
}

#[test]
fn smoke_4_bundle_format_parse_emit() {
    let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[10u8; 32]).unwrap();
    let rk = RekorClient::default();
    let signed = sign_blob_keypair_with_rekor(b"bundle-smoke", &kp, &rk).unwrap();
    let json = signed.bundle.encode_json().unwrap();
    let back = CosignBundle::decode_json(&json).unwrap();
    assert_eq!(back, signed.bundle);
    assert!(back.has_rekor_entry());
    let triple = BundleTriple::from_bundle(&signed.bundle).unwrap();
    assert_eq!(triple.sig_b64, signed.signature.sig_b64);
    assert!(triple.cert_pem.contains("BEGIN PUBLIC KEY"));
    assert!(triple.bundle_json.contains("\"artifact_digest\""));
}

#[test]
fn smoke_5_keyless_end_to_end_with_policy() {
    let signer = KeylessSigner::new(FulcioClient::default());
    let rk = RekorClient::default();
    let token_raw = build_fixture_jwt(&json!({
        "iss":"https://oidc.cave.svc","sub":"alice","aud":"sigstore",
        "exp": chrono::Utc::now().timestamp() + 3600,
        "email":"alice@example.com",
    }));
    let token = IdToken::parse(&token_raw).unwrap();
    let ks = signer.sign_blob(b"keyless-payload", &token, &rk).unwrap();
    let policy = Policy::new("cave-prod")
        .require(Rule::CertificateIdentity { glob: "*@example.com".into() })
        .require(Rule::CertificateIssuer { exact: "https://oidc.cave.svc".into() })
        .require(Rule::RequireKeyless)
        .require(Rule::RequireRekorEntry);
    let vr = verify(VerifyRequest {
        payload: b"keyless-payload",
        bundle: &ks.bundle,
        rekor: Some(&rk),
        policy: Some(&policy),
    })
    .unwrap();
    assert!(vr.valid);
    assert_eq!(vr.signer.as_deref(), Some("alice@example.com"));
}
