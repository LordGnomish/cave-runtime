// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-acme — Challenge tests pinned to RFC 8555 §8 + RFC 8737.

use cave_acme::{Challenge, ChallengeStatus, ChallengeType, Jwk};

const TENANT: &str = "tenant-acme-prod";

fn jwk() -> Jwk {
    Jwk::EC { crv: "P-256".into(),
        x: "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU".into(),
        y: "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0".into() }
}

fn challenge(kind: ChallengeType, token: &str) -> Challenge {
    Challenge {
        id: format!("ch-{}-{}", kind.as_str(), token),
        kind, status: ChallengeStatus::Pending,
        url: format!("/acme/chall/{}/{}", token, kind.as_str()),
        token: token.into(), validated_at: None, error: None,
    }
}

/// Cite: RFC 8555 §8.3 — HTTP-01 publishes keyAuth at
/// `/.well-known/acme-challenge/<token>`. Response body is the bare
/// keyAuth (no JSON wrapper).
#[test]
fn http01_resource_path_and_body() {
    let ch = challenge(ChallengeType::Http01, "TOK-http-1");
    assert_eq!(ch.http01_resource_path(), "/.well-known/acme-challenge/TOK-http-1");
    let body = ch.http01_response_body(&jwk());
    assert!(body.starts_with("TOK-http-1."));
    assert!(body.contains(&jwk().thumbprint()));
    let _ = TENANT;
}

/// Cite: RFC 8555 §8.4 — DNS-01 publishes a TXT record at
/// `_acme-challenge.<domain>` containing `base64url(SHA-256(keyAuth))`.
#[test]
fn dns01_record_name_and_value() {
    let ch = challenge(ChallengeType::Dns01, "TOK-dns-1");
    let domain = format!("svc.{}.cave-runtime.test", TENANT);
    assert_eq!(ch.dns01_record_name(&domain), format!("_acme-challenge.{}", domain));
    let value = ch.dns01_record_value(&jwk());
    // base64url of SHA-256 ⇒ 43 characters, no padding.
    assert_eq!(value.len(), 43);
    assert!(!value.contains('='));
    assert!(!value.contains('+'));
    assert!(!value.contains('/'));
}

/// Cite: RFC 8737 §3 — TLS-ALPN-01 places a 32-byte SHA-256 of the
/// keyAuth in the cert extension `id-pe-acmeIdentifier`.
#[test]
fn tls_alpn01_extension_value_is_32_byte_sha256() {
    let ch = challenge(ChallengeType::TlsAlpn01, "TOK-alpn-1");
    let ext = ch.tls_alpn01_extension_value(&jwk());
    assert_eq!(ext.len(), 32);
    // Different challenge token ⇒ different extension value
    let ch2 = challenge(ChallengeType::TlsAlpn01, "TOK-alpn-2");
    assert_ne!(ch.tls_alpn01_extension_value(&jwk()), ch2.tls_alpn01_extension_value(&jwk()));
}

/// Cite: RFC 8555 §7.1.6 — challenge status enum + the canonical type
/// strings emitted on the wire.
#[test]
fn challenge_type_canonical_wire_strings() {
    assert_eq!(ChallengeType::Http01.as_str(), "http-01");
    assert_eq!(ChallengeType::Dns01.as_str(), "dns-01");
    assert_eq!(ChallengeType::TlsAlpn01.as_str(), "tls-alpn-01");
    // Round-trip JSON status.
    let json = serde_json::to_string(&ChallengeStatus::Pending).unwrap();
    assert_eq!(json, "\"pending\"");
    let back: ChallengeStatus = serde_json::from_str("\"valid\"").unwrap();
    assert_eq!(back, ChallengeStatus::Valid);
}
