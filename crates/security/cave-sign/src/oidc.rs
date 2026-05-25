// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OIDC identity token holder + claim extraction.
//!
//! Maps to:
//!   * pkg/providers                        → OIDC discovery (delegated to caller)
//!   * pkg/cosign/keyless                   → IdentityToken
//!   * pkg/cosign/keyless_sign.go::Subject  → subject extraction
//!
//! `cave-sign` does *not* talk to Google/GitHub/Spiffe — token acquisition is
//! the caller's job (cave-auth ships the providers). What we own here is the
//! parsing + subject-extraction logic that Fulcio needs to issue a cert.

use crate::error::{Result, SignError};
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Minimal OIDC ID-token claim set used by Fulcio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdToken {
    pub raw: String,
    pub issuer: String,
    pub subject: String,
    /// Email + email_verified for the `email` flow.
    pub email: Option<String>,
    /// Audience the token was minted for.
    pub audience: String,
    /// Unix seconds at which the token expires.
    pub exp: i64,
}

impl IdToken {
    /// Parse the *unverified* claims from a compact JWT. Signature validation
    /// is delegated to cave-auth; cave-sign only needs the claim payload to
    /// pass to Fulcio (Fulcio itself re-verifies the signature against the
    /// issuer's JWKS).
    pub fn parse(raw: &str) -> Result<Self> {
        let parts: Vec<&str> = raw.split('.').collect();
        if parts.len() != 3 {
            return Err(SignError::Oidc("token is not a compact JWT".into()));
        }
        let claims_b64 = parts[1];
        let claims_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(claims_b64.as_bytes())
            .map_err(|e| SignError::Oidc(format!("base64url claims: {}", e)))?;
        let claims: serde_json::Value =
            serde_json::from_slice(&claims_bytes).map_err(|e| SignError::Oidc(format!("json: {}", e)))?;

        let issuer = claims["iss"]
            .as_str()
            .ok_or_else(|| SignError::Oidc("missing iss".into()))?
            .to_string();
        let subject = claims["sub"]
            .as_str()
            .ok_or_else(|| SignError::Oidc("missing sub".into()))?
            .to_string();
        let audience = match &claims["aud"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(a) => a
                .first()
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| SignError::Oidc("empty aud array".into()))?,
            _ => return Err(SignError::Oidc("aud must be string or array".into())),
        };
        let exp = claims["exp"]
            .as_i64()
            .ok_or_else(|| SignError::Oidc("missing exp".into()))?;
        let email = claims["email"].as_str().map(str::to_string);

        Ok(Self {
            raw: raw.to_string(),
            issuer,
            subject,
            email,
            audience,
            exp,
        })
    }

    /// Identity that Fulcio embeds in the SAN of the issued cert. Email
    /// claim wins (matches cosign behaviour); otherwise the bare `sub`.
    pub fn identity(&self) -> &str {
        self.email.as_deref().unwrap_or(&self.subject)
    }

    pub fn is_expired_at(&self, now_unix: i64) -> bool {
        self.exp <= now_unix
    }
}

/// Encode test/fixture claims as an unsigned JWT — header.payload.signature
/// with an empty signature segment. Used by smoke tests + Fulcio mock.
pub fn build_fixture_jwt(claims: &serde_json::Value) -> String {
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(b"{\"alg\":\"none\",\"typ\":\"JWT\"}");
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(claims).unwrap());
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"");
    format!("{}.{}.{}", header, payload, sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn token_with(claims: serde_json::Value) -> String {
        build_fixture_jwt(&claims)
    }

    #[test]
    fn parse_full_claims() {
        let raw = token_with(json!({
            "iss": "https://accounts.google.com",
            "sub": "alice|1234",
            "aud": "sigstore",
            "exp": 1_999_999_999i64,
            "email": "alice@example.com",
        }));
        let t = IdToken::parse(&raw).unwrap();
        assert_eq!(t.issuer, "https://accounts.google.com");
        assert_eq!(t.subject, "alice|1234");
        assert_eq!(t.audience, "sigstore");
        assert_eq!(t.email.as_deref(), Some("alice@example.com"));
        assert_eq!(t.identity(), "alice@example.com");
    }

    #[test]
    fn identity_falls_back_to_subject() {
        let raw = token_with(json!({
            "iss": "https://token.actions.githubusercontent.com",
            "sub": "repo:cave-runtime:ref:main",
            "aud": "sigstore",
            "exp": 1_999_999_999i64,
        }));
        let t = IdToken::parse(&raw).unwrap();
        assert_eq!(t.identity(), "repo:cave-runtime:ref:main");
        assert!(t.email.is_none());
    }

    #[test]
    fn aud_array_supported() {
        let raw = token_with(json!({
            "iss": "https://x",
            "sub": "y",
            "aud": ["sigstore", "other"],
            "exp": 1_999_999_999i64,
        }));
        let t = IdToken::parse(&raw).unwrap();
        assert_eq!(t.audience, "sigstore");
    }

    #[test]
    fn expiry_detected() {
        let raw = token_with(json!({
            "iss": "x", "sub": "y", "aud": "z", "exp": 100i64,
        }));
        let t = IdToken::parse(&raw).unwrap();
        assert!(t.is_expired_at(200));
        assert!(!t.is_expired_at(50));
    }

    #[test]
    fn invalid_compact_jwt_rejected() {
        let err = IdToken::parse("nope").expect_err("must reject");
        assert!(matches!(err, SignError::Oidc(_)));
    }

    #[test]
    fn missing_iss_rejected() {
        let raw = token_with(json!({"sub":"x","aud":"z","exp":1i64}));
        assert!(IdToken::parse(&raw).is_err());
    }

    #[test]
    fn missing_sub_rejected() {
        let raw = token_with(json!({"iss":"x","aud":"z","exp":1i64}));
        assert!(IdToken::parse(&raw).is_err());
    }

    #[test]
    fn missing_exp_rejected() {
        let raw = token_with(json!({"iss":"x","sub":"y","aud":"z"}));
        assert!(IdToken::parse(&raw).is_err());
    }
}
