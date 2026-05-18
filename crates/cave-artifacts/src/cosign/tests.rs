// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cosign module unit + integration tests.
//!
//! Real ECDSA round-trip, real Ed25519 half of the hybrid composite
//! (the ML-DSA half is a fixture — tests detect tampering by mutating
//! the composite and asserting the right typed error fires).

#![cfg(test)]

use super::manifest::{build_payload, manifest_digest, signature_tag, SignatureIndex};
use super::routes::{router, CosignState};
use super::{sign, verify, Alg, CosignError, KeyPair, PublicKeyHandle, Signature};
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use serde_json::json;
use tower::ServiceExt;

// ── Pure-fn round trips ──────────────────────────────────────────────────

#[test]
fn ecdsa_p256_sign_verify_roundtrip() {
    let key = KeyPair::generate(Alg::EcdsaP256);
    let pub_handle = key.public_handle().unwrap();
    let payload = build_payload("ghcr.io/owner/img:1", "sha256:00112233");
    let sig = sign(&key, &payload).unwrap();
    verify(&pub_handle, &payload, &sig).expect("signature must verify");
}

#[test]
fn ml_dsa_65_sign_verify_roundtrip() {
    let key = KeyPair::generate(Alg::MlDsa65);
    let pub_handle = key.public_handle().unwrap();
    let payload = build_payload("ghcr.io/owner/img:2", "sha256:99887766");
    let sig = sign(&key, &payload).unwrap();
    verify(&pub_handle, &payload, &sig).expect("hybrid composite must verify");
}

#[test]
fn ecdsa_verify_rejects_tampered_payload() {
    let key = KeyPair::generate(Alg::EcdsaP256);
    let pub_handle = key.public_handle().unwrap();
    let payload = build_payload("ref", "sha256:abc");
    let sig = sign(&key, &payload).unwrap();
    let tampered = build_payload("ref", "sha256:def");
    let err = verify(&pub_handle, &tampered, &sig).unwrap_err();
    assert!(
        matches!(err, CosignError::EcdsaVerifyFailed),
        "expected EcdsaVerifyFailed, got {err:?}"
    );
}

#[test]
fn ml_dsa_verify_rejects_tampered_payload() {
    let key = KeyPair::generate(Alg::MlDsa65);
    let pub_handle = key.public_handle().unwrap();
    let payload = build_payload("ref", "sha256:abc");
    let sig = sign(&key, &payload).unwrap();
    let tampered = build_payload("ref", "sha256:def");
    let err = verify(&pub_handle, &tampered, &sig).unwrap_err();
    // Either Ed25519 fails first (ClassicalInvalid) or composite fixture
    // mismatches (PostQuantumInvalid) — both are valid rejections.
    assert!(
        matches!(err, CosignError::PqcCompositeFailed(_)),
        "expected PqcCompositeFailed, got {err:?}"
    );
}

#[test]
fn ecdsa_verify_rejects_swapped_signature() {
    let key_a = KeyPair::generate(Alg::EcdsaP256);
    let key_b = KeyPair::generate(Alg::EcdsaP256);
    let pub_a = key_a.public_handle().unwrap();
    let payload = build_payload("ref", "sha256:abc");
    let sig_b = sign(&key_b, &payload).unwrap();
    // Signing with key_b but verifying against key_a's public key must fail.
    let err = verify(&pub_a, &payload, &sig_b).unwrap_err();
    assert!(matches!(err, CosignError::EcdsaVerifyFailed));
}

#[test]
fn cross_alg_mismatch_is_typed() {
    let ecdsa_key = KeyPair::generate(Alg::EcdsaP256);
    let pqc_key = KeyPair::generate(Alg::MlDsa65);
    let payload = build_payload("ref", "sha256:abc");
    let pqc_sig = sign(&pqc_key, &payload).unwrap();
    let ecdsa_pub = ecdsa_key.public_handle().unwrap();
    let err = verify(&ecdsa_pub, &payload, &pqc_sig).unwrap_err();
    assert!(matches!(
        err,
        CosignError::AlgorithmMismatch { sig_alg, key_alg }
            if sig_alg == "ml-dsa-65" && key_alg == "ecdsa-p256"
    ));
}

// ── Manifest helpers ────────────────────────────────────────────────────

#[test]
fn build_payload_pins_digest_in_critical_image() {
    let bytes = build_payload("registry/foo:bar", "sha256:abc123");
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["critical"]["image"]["docker-manifest-digest"], "sha256:abc123");
    assert_eq!(v["critical"]["identity"]["docker-reference"], "registry/foo:bar");
    assert_eq!(v["critical"]["type"], "cosign container image signature");
}

#[test]
fn manifest_digest_matches_sha256() {
    let bytes = b"hello world";
    let d = manifest_digest(bytes);
    assert!(d.starts_with("sha256:"));
    assert_eq!(d.len(), 7 + 64);
}

#[test]
fn signature_tag_strips_sha256_prefix() {
    let tag = signature_tag("sha256:deadbeef").unwrap();
    assert_eq!(tag, "sha256-deadbeef.sig");
    assert!(signature_tag("invalid").is_err());
}

#[test]
fn signature_index_attaches_and_lists() {
    let idx = SignatureIndex::new();
    let key = KeyPair::generate(Alg::EcdsaP256);
    let payload = build_payload("ref", "sha256:abc");
    let sig = sign(&key, &payload).unwrap();
    idx.attach("sha256:abc", &payload, sig);
    assert_eq!(idx.count("sha256:abc"), 1);
    assert!(idx.list("sha256:abc")[0].payload_b64.len() > 0);
    assert_eq!(idx.remove("sha256:abc"), 1);
    assert_eq!(idx.count("sha256:abc"), 0);
}

#[test]
fn alg_parse_round_trips_known_aliases() {
    assert_eq!(Alg::parse("ecdsa-p256"), Some(Alg::EcdsaP256));
    assert_eq!(Alg::parse("ECDSA-P256"), Some(Alg::EcdsaP256));
    assert_eq!(Alg::parse("ecdsa"), Some(Alg::EcdsaP256));
    assert_eq!(Alg::parse("ml-dsa-65"), Some(Alg::MlDsa65));
    assert_eq!(Alg::parse("MLDSA65"), Some(Alg::MlDsa65));
    assert_eq!(Alg::parse("hybrid"), Some(Alg::MlDsa65));
    assert!(Alg::parse("rsa").is_none());
}

#[test]
fn pqc_public_handle_exposes_seed_and_classical() {
    let key = KeyPair::generate(Alg::MlDsa65);
    match key.public_handle().unwrap() {
        PublicKeyHandle::MlDsa65 {
            classical_b64,
            pqc_seed_b64,
            expected_pqc_len,
            ..
        } => {
            assert!(!classical_b64.is_empty());
            assert!(!pqc_seed_b64.is_empty());
            assert_eq!(expected_pqc_len, 3309);
        }
        PublicKeyHandle::EcdsaP256 { .. } => panic!("wrong variant"),
    }
}

// ── HTTP integration tests ──────────────────────────────────────────────

mod http {
    use super::*;
    use std::sync::Arc;

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        if bytes.is_empty() {
            return json!(null);
        }
        serde_json::from_slice(&bytes).unwrap_or(json!(null))
    }

    #[tokio::test]
    async fn health_advertises_both_algorithms() {
        let app = router(CosignState::new());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/cosign/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["status"], "ok");
        let algs = body["supported_algorithms"].as_array().unwrap();
        assert!(algs.iter().any(|v| v == "ecdsa-p256"));
        assert!(algs.iter().any(|v| v == "ml-dsa-65"));
        assert_eq!(body["pqc_backend"], "fixture");
    }

    #[tokio::test]
    async fn keypair_generate_sign_verify_e2e_ecdsa() {
        let state = CosignState::new();
        let app = router(state.clone());

        // Generate
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/keypair")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"alg": "ecdsa-p256"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let key = body_json(resp).await;
        let key_id = key["id"].as_str().unwrap().to_string();

        // Sign
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "key_id": key_id,
                            "reference": "registry/img:tag",
                            "digest": "sha256:abc",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let signed = body_json(resp).await;
        let sig = signed["signature"].clone();
        let payload_b64 = signed["payload_b64"].as_str().unwrap().to_string();
        assert_eq!(signed["signature_tag"], "sha256-abc.sig");

        // Verify
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "key_id": key_id,
                            "signature": sig,
                            "payload_b64": payload_b64,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["valid"], true);

        // Counters reflect the activity
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/cosign/v1/counters")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let c = body_json(resp).await;
        assert_eq!(c["keys_generated_classic"], 1);
        assert_eq!(c["signatures_issued_classic"], 1);
        assert_eq!(c["verifications_passed_classic"], 1);
    }

    #[tokio::test]
    async fn signatures_listed_by_digest_after_sign() {
        let state = CosignState::new();
        let app = router(state.clone());

        // Generate + sign
        let key_id: String = {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/api/cosign/v1/keypair")
                        .header("content-type", "application/json")
                        .body(Body::from(json!({"alg": "ml-dsa-65"}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            body_json(resp).await["id"].as_str().unwrap().to_string()
        };
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "key_id": key_id,
                            "reference": "ref",
                            "digest": "sha256:1234",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/cosign/v1/signatures/sha256:1234")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn verify_with_wrong_key_returns_unauthorized() {
        let state = CosignState::new();
        let app = router(state.clone());

        // Two keys
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/keypair")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"alg": "ecdsa-p256"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let key_a = body_json(resp).await["id"].as_str().unwrap().to_string();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/keypair")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"alg": "ecdsa-p256"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let key_b = body_json(resp).await["id"].as_str().unwrap().to_string();

        // Sign with A
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "key_id": key_a,
                            "reference": "r",
                            "digest": "sha256:dd",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let signed = body_json(resp).await;
        let sig = signed["signature"].clone();
        let payload_b64 = signed["payload_b64"].as_str().unwrap().to_string();

        // Verify with B → unauthorized
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "key_id": key_b,
                            "signature": sig,
                            "payload_b64": payload_b64,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unknown_algorithm_is_400() {
        let app = router(CosignState::new());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/cosign/v1/keypair")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"alg": "rsa"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
