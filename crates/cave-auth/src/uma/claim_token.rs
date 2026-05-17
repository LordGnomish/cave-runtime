// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../authorization/util/Tokens.java + Kantara UMA-Grant §3.3
//
//! UMA 2.0 pushed-claims handler.
//!
//! Per UMA-Grant §3.3, a client may submit `claim_token` (with a matching
//! `claim_token_format`) at the `/token` endpoint when requesting an RPT.
//!
//! Supported `claim_token_format`:
//!   - `urn:ietf:params:oauth:token-type:jwt` — a JWT whose body is a flat
//!     JSON object of claims. Signature verification is OUT of scope for
//!     this module — that's the upstream IdP's job; the receiver MUST only
//!     trust the issuer it has federated with.
//!   - `http://openid.net/specs/openid-connect-core-1_0.html#IDToken` —
//!     synonym, treated identically.
//!   - `application/x-www-form-urlencoded` — Keycloak custom: a query-string
//!     of `k=v&k=v` pairs.

use std::collections::HashMap;

use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const FORMAT_JWT: &str =
    "urn:ietf:params:oauth:token-type:jwt";
pub const FORMAT_OIDC_ID_TOKEN: &str =
    "http://openid.net/specs/openid-connect-core-1_0.html#IDToken";
pub const FORMAT_FORM: &str = "application/x-www-form-urlencoded";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PushedClaims {
    inner: HashMap<String, String>,
}

impl PushedClaims {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let mut inner = HashMap::new();
        for (k, v) in pairs {
            inner.insert((*k).into(), (*v).into());
        }
        Self { inner }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.inner.get(key)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn insert(&mut self, k: impl Into<String>, v: impl Into<String>) {
        self.inner.insert(k.into(), v.into());
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClaimTokenError {
    #[error("unknown claim_token_format {0:?}")]
    UnknownFormat(String),
    #[error("claim_token is not a valid JWT (need 3 segments, found {0})")]
    BadJwtShape(usize),
    #[error("claim_token JWT body is not valid base64url: {0}")]
    Base64(String),
    #[error("claim_token JWT body is not valid JSON: {0}")]
    Json(String),
    #[error("claim_token JWT body must be a flat JSON object")]
    NotObject,
    #[error("claim_token form body is not valid utf-8: {0}")]
    Utf8(String),
}

/// Parses a `claim_token` + `claim_token_format` pair into pushed claims.
///
/// The function is purely a structural decode — issuer trust + signature
/// verification are handled by the caller (the RPT issuer) before invoking.
pub fn parse(claim_token: &str, format: &str) -> Result<PushedClaims, ClaimTokenError> {
    match format {
        FORMAT_JWT | FORMAT_OIDC_ID_TOKEN => parse_jwt_body(claim_token),
        FORMAT_FORM => parse_form_body(claim_token),
        other => Err(ClaimTokenError::UnknownFormat(other.to_string())),
    }
}

fn parse_jwt_body(jwt: &str) -> Result<PushedClaims, ClaimTokenError> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(ClaimTokenError::BadJwtShape(parts.len()));
    }
    let body_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1].as_bytes())
        .map_err(|e| ClaimTokenError::Base64(e.to_string()))?;
    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes).map_err(|e| ClaimTokenError::Json(e.to_string()))?;
    let obj = body.as_object().ok_or(ClaimTokenError::NotObject)?;
    let mut out = PushedClaims::empty();
    for (k, v) in obj {
        let s = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.insert(k, s);
    }
    Ok(out)
}

fn parse_form_body(form: &str) -> Result<PushedClaims, ClaimTokenError> {
    let mut out = PushedClaims::empty();
    for pair in form.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = percent_decode(k)?;
        let v = percent_decode(v)?;
        out.insert(k, v);
    }
    Ok(out)
}

fn percent_decode(s: &str) -> Result<String, ClaimTokenError> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push(((h << 4) | l) as u8);
                    i += 3;
                    continue;
                }
                return Err(ClaimTokenError::Utf8(format!("bad % escape at offset {i}")));
            }
            b'+' => out.push(b' '),
            _ => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).map_err(|e| ClaimTokenError::Utf8(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.as_bytes())
    }

    fn jwt(body: &str) -> String {
        format!("{}.{}.{}", b64(r#"{"alg":"none"}"#), b64(body), b64("sig"))
    }

    #[test]
    fn jwt_claims_round_trip() {
        let claims = parse(&jwt(r#"{"sub":"alice","dept":"eng"}"#), FORMAT_JWT).unwrap();
        assert_eq!(claims.get("sub").map(|s| s.as_str()), Some("alice"));
        assert_eq!(claims.get("dept").map(|s| s.as_str()), Some("eng"));
    }

    #[test]
    fn oidc_id_token_format_is_alias() {
        let claims = parse(&jwt(r#"{"sub":"x"}"#), FORMAT_OIDC_ID_TOKEN).unwrap();
        assert_eq!(claims.get("sub").map(|s| s.as_str()), Some("x"));
    }

    #[test]
    fn unknown_format_rejected() {
        let err = parse("anything", "nope").unwrap_err();
        assert!(matches!(err, ClaimTokenError::UnknownFormat(_)));
    }

    #[test]
    fn malformed_jwt_rejected() {
        let err = parse("only.two", FORMAT_JWT).unwrap_err();
        assert!(matches!(err, ClaimTokenError::BadJwtShape(2)));
    }

    #[test]
    fn jwt_body_must_be_object() {
        let err = parse(&jwt("[1,2,3]"), FORMAT_JWT).unwrap_err();
        assert!(matches!(err, ClaimTokenError::NotObject));
    }

    #[test]
    fn form_parse_simple() {
        let claims = parse("dept=eng&country=tr", FORMAT_FORM).unwrap();
        assert_eq!(claims.get("dept").map(|s| s.as_str()), Some("eng"));
        assert_eq!(claims.get("country").map(|s| s.as_str()), Some("tr"));
    }

    #[test]
    fn form_percent_decoded() {
        let claims = parse("title=hello%20world", FORMAT_FORM).unwrap();
        assert_eq!(claims.get("title").map(|s| s.as_str()), Some("hello world"));
    }

    #[test]
    fn form_plus_is_space() {
        let claims = parse("a=b+c", FORMAT_FORM).unwrap();
        assert_eq!(claims.get("a").map(|s| s.as_str()), Some("b c"));
    }

    #[test]
    fn jwt_numbers_become_string() {
        let claims = parse(&jwt(r#"{"age":30}"#), FORMAT_JWT).unwrap();
        assert_eq!(claims.get("age").map(|s| s.as_str()), Some("30"));
    }

    #[test]
    fn empty_pushed_claims() {
        let p = PushedClaims::empty();
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn from_pairs_helper() {
        let p = PushedClaims::from_pairs(&[("a", "1"), ("b", "2")]);
        assert_eq!(p.len(), 2);
    }
}
