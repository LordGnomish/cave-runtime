// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../oidc/utils/PkceUtils.java
//
//! PKCE (RFC 7636) — code_challenge / code_verifier validation.
//!
//! Supports both `S256` (SHA-256+base64url) and the legacy `plain`
//! method. Validation rules:
//! - verifier length: 43..=128 characters (RFC 7636 §4.1)
//! - verifier chars: unreserved set `ALPHA / DIGIT / "-" / "." / "_" / "~"`
//! - challenge for `S256`: BASE64URL(SHA-256(verifier)), no padding
//!
//! Used by `authorize.rs` (to store the challenge) and `token_endpoint`
//! (to verify the verifier on `code` exchange).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

/// The set of code-challenge methods accepted by the authorization endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkceMethod {
    Plain,
    S256,
}

impl PkceMethod {
    pub fn parse(s: &str) -> Result<Self, &'static str> {
        match s {
            "plain" | "PLAIN" => Ok(Self::Plain),
            "S256" | "s256" => Ok(Self::S256),
            _ => Err("invalid_request"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::S256 => "S256",
        }
    }
}

/// Validate that a code_verifier conforms to the RFC 7636 grammar.
pub fn is_valid_verifier(verifier: &str) -> bool {
    let len = verifier.len();
    if !(43..=128).contains(&len) {
        return false;
    }
    verifier.bytes().all(|b| matches!(
        b,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
    ))
}

/// Compute the code_challenge expected for the given verifier + method.
pub fn compute_challenge(verifier: &str, method: PkceMethod) -> String {
    match method {
        PkceMethod::Plain => verifier.to_string(),
        PkceMethod::S256 => {
            let digest = Sha256::digest(verifier.as_bytes());
            URL_SAFE_NO_PAD.encode(digest)
        }
    }
}

/// Verify a code_verifier against a previously stored code_challenge.
///
/// Returns `Ok(())` on success; on failure returns an OAuth error
/// code suitable for the `error` field of a token-error response.
pub fn verify(
    verifier: &str,
    challenge: &str,
    method: PkceMethod,
) -> Result<(), &'static str> {
    if !is_valid_verifier(verifier) {
        return Err("invalid_grant");
    }
    let expected = compute_challenge(verifier, method);
    // Constant-time-ish compare: lengths first then byte-wise.
    if expected.len() != challenge.len() {
        return Err("invalid_grant");
    }
    let mut diff = 0u8;
    for (a, b) in expected.bytes().zip(challenge.bytes()) {
        diff |= a ^ b;
    }
    if diff == 0 {
        Ok(())
    } else {
        Err("invalid_grant")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: keycloak/keycloak PkceUtilsTest.java:parseMethodS256
    #[test]
    fn pkce_method_parses_s256() {
        assert_eq!(PkceMethod::parse("S256").unwrap(), PkceMethod::S256);
        assert_eq!(PkceMethod::parse("s256").unwrap(), PkceMethod::S256);
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:parseMethodPlain
    #[test]
    fn pkce_method_parses_plain() {
        assert_eq!(PkceMethod::parse("plain").unwrap(), PkceMethod::Plain);
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:parseMethodUnknownRejects
    #[test]
    fn pkce_method_rejects_unknown() {
        assert!(PkceMethod::parse("md5").is_err());
        assert!(PkceMethod::parse("").is_err());
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:verifierLengthBoundary
    #[test]
    fn verifier_length_boundaries() {
        // 42 chars: too short
        assert!(!is_valid_verifier(&"a".repeat(42)));
        // 43 chars: min valid
        assert!(is_valid_verifier(&"a".repeat(43)));
        // 128 chars: max valid
        assert!(is_valid_verifier(&"a".repeat(128)));
        // 129 chars: too long
        assert!(!is_valid_verifier(&"a".repeat(129)));
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:verifierBadCharsetRejects
    #[test]
    fn verifier_rejects_invalid_chars() {
        // Plus sign not in unreserved set.
        let v: String = std::iter::repeat('+').take(43).collect();
        assert!(!is_valid_verifier(&v));
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:s256ChallengeMatchesRfc
    #[test]
    fn s256_challenge_matches_rfc_example() {
        // RFC 7636 Appendix B example:
        // verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        // challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(compute_challenge(verifier, PkceMethod::S256), expected);
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:plainChallengeIsIdentity
    #[test]
    fn plain_challenge_is_identity() {
        let verifier = "abcdefghijklmnopqrstuvwxyz1234567890ABCDEFG";
        assert_eq!(compute_challenge(verifier, PkceMethod::Plain), verifier);
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:verifyS256Matches
    #[test]
    fn verify_s256_matches() {
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let c = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify(v, c, PkceMethod::S256).is_ok());
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:verifyS256Mismatch
    #[test]
    fn verify_s256_mismatch_rejected() {
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        // Modify one char of the expected challenge.
        let c = "X9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(verify(v, c, PkceMethod::S256).unwrap_err(), "invalid_grant");
    }

    // upstream: keycloak/keycloak PkceUtilsTest.java:verifyInvalidVerifierShape
    #[test]
    fn verify_rejects_short_verifier() {
        assert!(verify("short", "anything", PkceMethod::S256).is_err());
    }
}
