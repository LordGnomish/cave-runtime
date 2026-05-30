// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD portable-coverage fills for `cave-identity` (theme: security).
//!
//! Upstream: spiffe/spire v1.15.0 (Apache-2.0,
//! source_sha b7db9650aa98598ee7af21d7a75fbab8f6b70d42).
//!
//! Each test below exercises an already-implemented public cave fn whose
//! error/edge branch is currently unexercised by the in-crate unit tests.
//! No source changes are implied — these assert existing behaviour:
//!
//!   * `jwt_svid::verify` — expired-token branch (`claims.exp < now`),
//!     upstream analog `jwtsvid` expiry / `TestIsSVIDExpired`.
//!   * `jwt_svid::verify` — unknown-kid branch (kid absent from bundle
//!     authorities), upstream analog `TestValidateJWTSVID` unknown-key.
//!   * `policy::admit_entry` — parent_id foreign-trust-domain rejection
//!     (distinct from the spiffe_id mismatch path), upstream analog
//!     `entry/v1` parent validation.
//!   * `bundle::unmarshal` — malformed x509-svid (missing `x5c`) and the
//!     symmetric malformed jwt-svid (missing `x`) JWK entries, upstream
//!     analog `bundleutil` malformed-jwk.
//!
//! Constraints: integration tests see only `cave_identity`'s public API plus
//! its dev-dependencies (`serde_json`, `proptest`). `chrono` and `base64` are
//! NOT dev-deps, so JWT tokens are hand-assembled with a local base64url-no-pad
//! encoder and only public, chrono-free constructors are used. Gaps that
//! require constructing a `DateTime<Utc>` or a bootstrapped `ServerCa`
//! (x509 verify no-intermediates, rotate_if_needed no-op, taint_root bundle
//! propagation) are intentionally omitted from this integration target.

use cave_identity::bundle::{self, BundleDoc, JwkEntry};
use cave_identity::error::IdentityError;
use cave_identity::jwt_svid;
use cave_identity::models::{Bundle, RegistrationEntry, Selector, SpiffeId, TrustDomain};
use cave_identity::policy::{self, Caller, PolicyConfig};

/// Minimal RFC 4648 base64url (no padding) encoder — mirrors the
/// `URL_SAFE_NO_PAD` engine `jwt_svid` uses internally. Local so the test
/// pulls in no new external crate.
fn b64url_no_pad(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3f) as usize] as char);
        }
    }
    out
}

/// Assemble a 3-part JWT-SVID string from header/claims JSON. The signature
/// segment is arbitrary valid base64url — `verify` rejects these tokens on the
/// exp/kid checks, which fire before signature verification.
fn craft_token(header_json: &str, claims_json: &str) -> String {
    format!(
        "{}.{}.{}",
        b64url_no_pad(header_json.as_bytes()),
        b64url_no_pad(claims_json.as_bytes()),
        b64url_no_pad(&[0xaa, 0xbb, 0xcc]),
    )
}

/// A trust bundle with no JWT authorities — forces `verify`'s kid lookup to
/// miss. `Bundle` carries no `DateTime` field when both authority vecs are
/// empty, so it is constructible without `chrono`.
fn empty_bundle() -> Bundle {
    Bundle {
        trust_domain: TrustDomain::new("example.org"),
        x509_authorities: vec![],
        jwt_authorities: vec![],
        refresh_hint_seconds: 60,
        sequence_number: 1,
    }
}

// ---------------------------------------------------------------------------
// jwt_svid::verify — expired token (src/jwt_svid.rs: `claims.exp < now`)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_expired_token() {
    // exp well in the past (2001-09-09T01:46:40Z); aud matches and alg is
    // accepted, so the only reachable failure is the expiry guard, which is
    // checked before audience matching and the kid lookup.
    let header = r#"{"alg":"ES256","typ":"JWT","kid":"jwt-0"}"#;
    let claims = r#"{"sub":"spiffe://example.org/svc","aud":["api.example"],"exp":1000000000,"iat":999999000}"#;
    let token = craft_token(header, claims);

    let err = jwt_svid::verify(&token, "api.example", &empty_bundle()).unwrap_err();
    match err {
        IdentityError::JwtInvalid(msg) => assert_eq!(msg, "expired"),
        other => panic!("expected JwtInvalid(\"expired\"), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// jwt_svid::verify — unknown kid (src/jwt_svid.rs: bundle authority lookup)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_unknown_kid() {
    // exp far in the future (2100-01-01) so the expiry guard passes; aud
    // matches so audience matching passes; the bundle has zero jwt authorities
    // so the kid lookup misses and reports the offending kid verbatim.
    let header = r#"{"alg":"ES256","typ":"JWT","kid":"ghost-kid"}"#;
    let claims = r#"{"sub":"spiffe://example.org/svc","aud":["api.example"],"exp":4102444800,"iat":1700000000}"#;
    let token = craft_token(header, claims);

    let err = jwt_svid::verify(&token, "api.example", &empty_bundle()).unwrap_err();
    match err {
        IdentityError::JwtInvalid(msg) => assert_eq!(msg, "unknown kid: ghost-kid"),
        other => panic!("expected JwtInvalid(\"unknown kid: ghost-kid\"), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// policy::admit_entry — parent_id foreign trust-domain
// (src/policy.rs: parent-id trust-domain check, distinct from spiffe_id)
// ---------------------------------------------------------------------------

#[test]
fn admit_entry_rejects_foreign_parent_trust_domain() {
    let cfg = PolicyConfig::new(TrustDomain::new("example.org"));
    let caller = Caller {
        spiffe_id: SpiffeId::new("spiffe://example.org/admin"),
        admin: true,
    };
    // spiffe_id is in-domain (passes the first td check) but parent_id lives in
    // a different trust domain — only the parent-id branch can fire here.
    let entry = RegistrationEntry {
        id: "e1".into(),
        spiffe_id: SpiffeId::new("spiffe://example.org/svc"),
        parent_id: SpiffeId::new("spiffe://other.org/agent"),
        selectors: vec![Selector::new("k8s", "ns:default")],
        x509_svid_ttl_seconds: 3600,
        jwt_svid_ttl_seconds: 300,
        ..Default::default()
    };

    let err = policy::admit_entry(&cfg, &caller, entry).unwrap_err();
    match err {
        IdentityError::PolicyViolation(msg) => {
            assert_eq!(
                msg,
                "parent_id trust-domain mismatch (want example.org; got other.org)"
            );
        }
        other => panic!("expected PolicyViolation for parent_id, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// bundle::unmarshal — x509-svid entry missing x5c
// (src/bundle.rs: `x5c.as_ref().ok_or_else(... "x509-svid missing x5c")`)
// ---------------------------------------------------------------------------

#[test]
fn unmarshal_rejects_x509_entry_missing_x5c() {
    let doc = BundleDoc {
        keys: vec![JwkEntry {
            kty: "RSA".into(),
            key_use: "x509-svid".into(),
            kid: "x509-0".into(),
            crv: None,
            x: None,
            y: None,
            n: None,
            e: None,
            x5c: None,
            spiffe_tainted: None,
        }],
        spiffe_refresh_hint: 0,
        spiffe_sequence: 0,
    };

    let err = bundle::unmarshal(&TrustDomain::new("example.org"), &doc).unwrap_err();
    match err {
        IdentityError::Internal(msg) => assert_eq!(msg, "x509-svid missing x5c"),
        other => panic!("expected Internal(\"x509-svid missing x5c\"), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// bundle::unmarshal — jwt-svid entry missing x (symmetric malformed-jwk branch)
// (src/bundle.rs: `x.as_ref().ok_or_else(... "jwt-svid missing x")`)
// ---------------------------------------------------------------------------

#[test]
fn unmarshal_rejects_jwt_entry_missing_x() {
    let doc = BundleDoc {
        keys: vec![JwkEntry {
            kty: "EC".into(),
            key_use: "jwt-svid".into(),
            kid: "jwt-0".into(),
            crv: Some("P-256".into()),
            x: None,
            y: None,
            n: None,
            e: None,
            x5c: None,
            spiffe_tainted: None,
        }],
        spiffe_refresh_hint: 0,
        spiffe_sequence: 0,
    };

    let err = bundle::unmarshal(&TrustDomain::new("example.org"), &doc).unwrap_err();
    match err {
        IdentityError::Internal(msg) => assert_eq!(msg, "jwt-svid missing x"),
        other => panic!("expected Internal(\"jwt-svid missing x\"), got {other:?}"),
    }
}
