// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/+ RFC 9449 testsuite
//
//! Upstream-test port for DPoP.
//!
//! These mirror named Java test cases from `services/src/test/java/.../protocol/oidc/`
//! plus the assertions called out explicitly in RFC 9449. Each `#[test]`
//! corresponds to one upstream verifier-side scenario.

use base64::Engine;
use cave_auth::dpop::verify::VerifyConfig;
use cave_auth::dpop::{DpopProof, DpopVerifyError, Jwk, ReplayGuard, verify_proof};

fn b64u(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn build_proof(htm: &str, htu: &str, iat: i64, jti: &str) -> DpopProof {
    let h = r#"{"alg":"ES256","typ":"dpop+jwt","jwk":{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB"}}"#;
    let p = format!(r#"{{"jti":"{jti}","htm":"{htm}","htu":"{htu}","iat":{iat}}}"#);
    let sig = b64u(&[0u8; 64]);
    let raw = format!("{}.{}.{}", b64u(h.as_bytes()), b64u(p.as_bytes()), sig);
    DpopProof::parse(&raw).unwrap()
}

/// Mirrors `DpopValidatorTest::testProofHasRequiredHeader` in upstream.
#[test]
fn rfc9449_proof_has_required_header() {
    let proof = build_proof("POST", "https://r/x", 1000, "j");
    assert_eq!(proof.header.typ, "dpop+jwt");
    assert_eq!(proof.header.alg, "ES256");
}

/// Mirrors `DpopValidatorTest::testJwkInHeader` — RFC 9449 §4.2.
#[test]
fn rfc9449_jwk_in_header() {
    let proof = build_proof("POST", "https://r/x", 1000, "j");
    match proof.header.jwk {
        Jwk::Ec { ref crv, .. } => assert_eq!(crv, "P-256"),
        _ => panic!("expected EC jwk"),
    }
}

/// Mirrors `DpopValidatorTest::testHtmMatchesRequest`.
#[test]
fn rfc9449_htm_matches_request() {
    let proof = build_proof("POST", "https://r/x", 1000, "j");
    let guard = ReplayGuard::with_default_window();
    let cfg = VerifyConfig {
        http_method: "POST",
        http_uri: "https://r/x",
        now_seconds: 1000,
        clock_skew_seconds: 60,
        bound_access_token: None,
        expected_jkt: None,
    };
    assert!(verify_proof(&proof, &cfg, &guard).is_ok());
}

/// Mirrors `DpopValidatorTest::testHtuFragmentStripped` — RFC 9449 §4.3 step 7.
#[test]
fn rfc9449_htu_fragment_stripped() {
    let proof = build_proof("POST", "https://r/x#fragment", 1000, "j");
    let guard = ReplayGuard::with_default_window();
    let cfg = VerifyConfig {
        http_method: "POST",
        http_uri: "https://r/x",
        now_seconds: 1000,
        clock_skew_seconds: 60,
        bound_access_token: None,
        expected_jkt: None,
    };
    assert!(verify_proof(&proof, &cfg, &guard).is_ok());
}

/// Mirrors `DpopValidatorTest::testIatRejectedWhenStale` — RFC 9449 §4.3 step 6.
#[test]
fn rfc9449_iat_rejected_when_stale() {
    let proof = build_proof("POST", "https://r/x", 1000, "j");
    let guard = ReplayGuard::with_default_window();
    let cfg = VerifyConfig {
        http_method: "POST",
        http_uri: "https://r/x",
        now_seconds: 9999,
        clock_skew_seconds: 60,
        bound_access_token: None,
        expected_jkt: None,
    };
    let err = verify_proof(&proof, &cfg, &guard).unwrap_err();
    assert!(matches!(err, DpopVerifyError::IatOutOfWindow { .. }));
}

/// Mirrors `DpopValidatorTest::testJtiReplayDetected` — RFC 9449 §11.1.
#[test]
fn rfc9449_jti_replay_detected() {
    let proof = build_proof("POST", "https://r/x", 1000, "the-jti");
    let guard = ReplayGuard::with_default_window();
    let cfg = VerifyConfig {
        http_method: "POST",
        http_uri: "https://r/x",
        now_seconds: 1000,
        clock_skew_seconds: 60,
        bound_access_token: None,
        expected_jkt: None,
    };
    verify_proof(&proof, &cfg, &guard).unwrap();
    let err = verify_proof(&proof, &cfg, &guard).unwrap_err();
    assert!(matches!(err, DpopVerifyError::ReplayedJti(_)));
}

/// Mirrors `DpopValidatorTest::testRequestRejectsWrongTyp` — §4.3 step 4.
#[test]
fn rfc9449_wrong_typ_rejected() {
    let h = r#"{"alg":"ES256","typ":"JWT","jwk":{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB"}}"#;
    let p = r#"{"jti":"j","htm":"POST","htu":"https://r/x","iat":1}"#;
    let raw = format!(
        "{}.{}.{}",
        b64u(h.as_bytes()),
        b64u(p.as_bytes()),
        b64u(&[0u8; 64])
    );
    assert!(DpopProof::parse(&raw).is_err());
}

/// Mirrors `JwkThumbprintTest::testRfc7638Vector` — RFC 7638 §3.1 reference.
#[test]
fn rfc7638_thumbprint_vector() {
    use cave_auth::dpop::jkt_thumbprint;
    let jwk = Jwk::Rsa {
        n: "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw".into(),
        e: "AQAB".into(),
    };
    assert_eq!(
        jkt_thumbprint(&jwk),
        "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
    );
}
