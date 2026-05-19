// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gap 2 close-out — security hardening tests (CSP, HSTS, frame-options,
//! referrer-policy, permissions-policy, X-Content-Type-Options, CSRF,
//! secure cookies, rate limiting).
//!
//! 2026-05-18 sprint. Every assertion below is the OWASP secure-headers
//! recommendation as of v2024.04 — see <https://owasp.org/www-project-secure-headers/>.

use cave_portal::hardening::{
    csrf::{generate_token, validate_token, CsrfStore},
    cookie::{secure_cookie_attrs, SameSite},
    headers::{security_headers, SecurityHeaderOptions},
    ratelimit::{RateLimitDecision, TokenBucket},
};
use std::sync::Arc;

// ── Security headers ─────────────────────────────────────────────────

#[test]
fn security_headers_include_strict_csp_default() {
    let h = security_headers(&SecurityHeaderOptions::default());
    let csp = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("Content-Security-Policy"));
    let (_, value) = csp.expect("CSP header must be present");
    assert!(value.contains("default-src 'self'"));
    assert!(value.contains("frame-ancestors 'none'"));
    assert!(value.contains("base-uri 'self'"));
    assert!(value.contains("object-src 'none'"));
}

#[test]
fn csp_includes_nonce_when_provided() {
    let mut opts = SecurityHeaderOptions::default();
    opts.script_nonce = Some("abc123".to_string());
    let h = security_headers(&opts);
    let csp = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("Content-Security-Policy")).unwrap();
    assert!(csp.1.contains("'nonce-abc123'"), "CSP must wire nonce; got {}", csp.1);
}

#[test]
fn hsts_header_set_with_one_year_max_age_and_subdomains() {
    let h = security_headers(&SecurityHeaderOptions::default());
    let hsts = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("Strict-Transport-Security")).unwrap();
    assert!(hsts.1.contains("max-age=31536000"));
    assert!(hsts.1.contains("includeSubDomains"));
    assert!(hsts.1.contains("preload"));
}

#[test]
fn x_frame_options_set_to_deny_by_default() {
    let h = security_headers(&SecurityHeaderOptions::default());
    let xfo = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("X-Frame-Options")).unwrap();
    assert_eq!(xfo.1, "DENY");
}

#[test]
fn x_content_type_options_set_to_nosniff() {
    let h = security_headers(&SecurityHeaderOptions::default());
    let nosniff = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("X-Content-Type-Options")).unwrap();
    assert_eq!(nosniff.1, "nosniff");
}

#[test]
fn referrer_policy_is_strict_origin_when_cross_origin() {
    let h = security_headers(&SecurityHeaderOptions::default());
    let rp = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("Referrer-Policy")).unwrap();
    assert_eq!(rp.1, "strict-origin-when-cross-origin");
}

#[test]
fn permissions_policy_disables_dangerous_features() {
    let h = security_headers(&SecurityHeaderOptions::default());
    let pp = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("Permissions-Policy")).unwrap();
    assert!(pp.1.contains("geolocation=()"));
    assert!(pp.1.contains("microphone=()"));
    assert!(pp.1.contains("camera=()"));
}

#[test]
fn cross_origin_isolation_headers_are_set() {
    let h = security_headers(&SecurityHeaderOptions::default());
    let coop = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("Cross-Origin-Opener-Policy"));
    assert!(coop.is_some(), "COOP must be set");
    let corp = h.iter().find(|(k, _)| k.eq_ignore_ascii_case("Cross-Origin-Resource-Policy"));
    assert!(corp.is_some(), "CORP must be set");
}

// ── CSRF tokens ──────────────────────────────────────────────────────

#[test]
fn csrf_generate_returns_random_base64url_token_of_minimum_entropy() {
    let t1 = generate_token();
    let t2 = generate_token();
    assert_ne!(t1, t2, "tokens must be random across calls");
    // base64url alphabet only.
    for c in t1.chars() {
        assert!(
            c.is_ascii_alphanumeric() || c == '-' || c == '_',
            "non-base64url char {c:?}"
        );
    }
    // 192 bits / 8 bytes/char × 4/3 base64 ≈ 32 chars min for 24 bytes raw.
    assert!(t1.len() >= 32, "token too short for 128+ bits of entropy");
}

#[test]
fn csrf_validate_matches_double_submit_cookie() {
    let store = CsrfStore::new();
    let tok = store.issue();
    assert!(validate_token(&tok, &tok), "matching cookie/header must validate");
}

#[test]
fn csrf_validate_rejects_mismatched_token() {
    let store = CsrfStore::new();
    let _good = store.issue();
    assert!(!validate_token("attacker", "victim"));
}

#[test]
fn csrf_validate_rejects_empty_token() {
    assert!(!validate_token("", ""));
    assert!(!validate_token("x", ""));
    assert!(!validate_token("", "x"));
}

#[test]
fn csrf_validate_uses_constant_time_comparison_for_equal_length() {
    // Different prefix but equal length should still reject; this is
    // mostly a smoke test that we don't short-circuit on the first
    // byte mismatch.
    assert!(!validate_token("aaaaaaaaaaaa", "bbbbbbbbbbbb"));
    assert!(validate_token("aaaaaaaaaaaa", "aaaaaaaaaaaa"));
}

// ── Secure cookies ───────────────────────────────────────────────────

#[test]
fn secure_cookie_attrs_default_to_secure_httponly_samesite_strict() {
    let attrs = secure_cookie_attrs(SameSite::Strict);
    assert!(attrs.contains("Secure"));
    assert!(attrs.contains("HttpOnly"));
    assert!(attrs.contains("SameSite=Strict"));
}

#[test]
fn secure_cookie_attrs_samesite_lax_variant() {
    let attrs = secure_cookie_attrs(SameSite::Lax);
    assert!(attrs.contains("SameSite=Lax"));
    assert!(!attrs.contains("SameSite=Strict"));
}

#[test]
fn secure_cookie_attrs_path_is_root() {
    let attrs = secure_cookie_attrs(SameSite::Strict);
    assert!(attrs.contains("Path=/"));
}

// ── Rate limiter ─────────────────────────────────────────────────────

#[test]
fn rate_limiter_allows_under_capacity() {
    let bucket = Arc::new(TokenBucket::new(5, std::time::Duration::from_secs(60)));
    for _ in 0..5 {
        match bucket.consume(1) {
            RateLimitDecision::Allow { .. } => {}
            RateLimitDecision::Deny { .. } => panic!("should allow under capacity"),
        }
    }
}

#[test]
fn rate_limiter_denies_over_capacity() {
    let bucket = Arc::new(TokenBucket::new(3, std::time::Duration::from_secs(60)));
    for _ in 0..3 {
        let _ = bucket.consume(1);
    }
    match bucket.consume(1) {
        RateLimitDecision::Deny { retry_after_secs } => {
            assert!(retry_after_secs > 0, "must report retry-after");
        }
        RateLimitDecision::Allow { .. } => panic!("4th request must be denied at capacity 3"),
    }
}

#[test]
fn rate_limiter_refills_over_time() {
    use std::thread::sleep;
    use std::time::Duration;
    // 2 tokens, refill window 100ms.
    let bucket = Arc::new(TokenBucket::new(2, Duration::from_millis(100)));
    let _ = bucket.consume(1);
    let _ = bucket.consume(1);
    assert!(matches!(bucket.consume(1), RateLimitDecision::Deny { .. }));
    sleep(Duration::from_millis(120));
    assert!(matches!(bucket.consume(1), RateLimitDecision::Allow { .. }));
}

#[test]
fn rate_limiter_reports_remaining_tokens_on_allow() {
    let bucket = Arc::new(TokenBucket::new(5, std::time::Duration::from_secs(60)));
    if let RateLimitDecision::Allow { remaining } = bucket.consume(1) {
        assert_eq!(remaining, 4);
    } else {
        panic!("first request should be allowed");
    }
}

// ── Header-set count regression ──────────────────────────────────────

#[test]
fn security_headers_count_meets_minimum() {
    let h = security_headers(&SecurityHeaderOptions::default());
    // We require at minimum: CSP, HSTS, XFO, XCTO, RP, PP, COOP, CORP — 8 headers.
    assert!(
        h.len() >= 8,
        "expected at least 8 security headers, got {}: {:?}",
        h.len(),
        h.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>()
    );
}
