// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JWKS cache + key resolution.
//!
//! Complements [`crate::auth`] (which does authentication-policy
//! enforcement) with the *key resolution* half of the JWT pipeline:
//!
//! * Parse a JWKS JSON document (RFC 7517) into a typed key set.
//! * Cache JWKS per issuer with an operator-configurable TTL.
//! * Resolve `(iss, kid)` → `DecodingKey`, falling back to the first
//!   key in the set when the token has no `kid` (a permitted JWS
//!   behaviour per RFC 7515 §4.1.4).
//! * Validate `iss`, `aud`, `exp`, `nbf` against a `RequestAuthentication`.
//! * Refresh the cache when its entry is past TTL — pluggable
//!   transport so tests stay offline.
//!
//! The transport is a trait so production wires up `reqwest`-over-TLS
//! and tests inject deterministic JWKS bodies. RSA + EC + HMAC key
//! types are supported via `jsonwebtoken::DecodingKey`.

use crate::error::{MeshError, MeshResult};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// One entry in a JWKS document. Only the fields cave-mesh actually
/// uses on the validation path are typed; everything else is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jwk {
    pub kty: String,
    #[serde(default)]
    pub kid: Option<String>,
    #[serde(default)]
    pub alg: Option<String>,
    #[serde(default, rename = "use")]
    pub usage: Option<String>,
    /// RSA modulus (base64url, no padding).
    #[serde(default)]
    pub n: Option<String>,
    /// RSA exponent.
    #[serde(default)]
    pub e: Option<String>,
    /// EC curve name.
    #[serde(default)]
    pub crv: Option<String>,
    /// EC x coordinate.
    #[serde(default)]
    pub x: Option<String>,
    /// EC y coordinate.
    #[serde(default)]
    pub y: Option<String>,
    /// Symmetric secret (HS256-style).
    #[serde(default)]
    pub k: Option<String>,
}

impl Jwk {
    /// Build a [`jsonwebtoken::DecodingKey`] suitable for this JWK's
    /// key type. Errors with [`MeshError::Jwt`] when required fields
    /// are absent (e.g. an RSA JWK missing `n`/`e`).
    pub fn decoding_key(&self) -> MeshResult<DecodingKey> {
        match self.kty.as_str() {
            "RSA" => {
                let n = self
                    .n
                    .as_deref()
                    .ok_or_else(|| MeshError::Jwt("RSA JWK missing modulus (n)".into()))?;
                let e = self
                    .e
                    .as_deref()
                    .ok_or_else(|| MeshError::Jwt("RSA JWK missing exponent (e)".into()))?;
                DecodingKey::from_rsa_components(n, e)
                    .map_err(|err| MeshError::Jwt(format!("RSA JWK invalid: {err}")))
            }
            "EC" => {
                let x = self
                    .x
                    .as_deref()
                    .ok_or_else(|| MeshError::Jwt("EC JWK missing x".into()))?;
                let y = self
                    .y
                    .as_deref()
                    .ok_or_else(|| MeshError::Jwt("EC JWK missing y".into()))?;
                DecodingKey::from_ec_components(x, y)
                    .map_err(|err| MeshError::Jwt(format!("EC JWK invalid: {err}")))
            }
            "oct" => {
                let k = self
                    .k
                    .as_deref()
                    .ok_or_else(|| MeshError::Jwt("oct JWK missing key material (k)".into()))?;
                Ok(DecodingKey::from_base64_secret(k)
                    .map_err(|err| MeshError::Jwt(format!("oct JWK invalid: {err}")))?)
            }
            other => Err(MeshError::Jwt(format!("unsupported JWK kty: {other}"))),
        }
    }

    pub fn algorithm(&self) -> MeshResult<Algorithm> {
        let alg = self
            .alg
            .as_deref()
            .unwrap_or_else(|| match self.kty.as_str() {
                "RSA" => "RS256",
                "EC" => "ES256",
                "oct" => "HS256",
                _ => "",
            });
        parse_alg(alg)
    }
}

fn parse_alg(s: &str) -> MeshResult<Algorithm> {
    match s {
        "HS256" => Ok(Algorithm::HS256),
        "HS384" => Ok(Algorithm::HS384),
        "HS512" => Ok(Algorithm::HS512),
        "RS256" => Ok(Algorithm::RS256),
        "RS384" => Ok(Algorithm::RS384),
        "RS512" => Ok(Algorithm::RS512),
        "ES256" => Ok(Algorithm::ES256),
        "ES384" => Ok(Algorithm::ES384),
        other => Err(MeshError::Jwt(format!("unsupported alg: {other}"))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JwkSet {
    pub keys: Vec<Jwk>,
}

impl JwkSet {
    pub fn parse(raw: &str) -> MeshResult<Self> {
        serde_json::from_str(raw).map_err(|e| MeshError::Jwt(format!("invalid JWKS JSON: {e}")))
    }

    /// Lookup a key by `kid`. Returns the first key when `kid` is
    /// `None` and the set has exactly one key (RFC 7515 §4.1.4).
    pub fn find(&self, kid: Option<&str>) -> Option<&Jwk> {
        match kid {
            Some(k) => self.keys.iter().find(|j| j.kid.as_deref() == Some(k)),
            None if self.keys.len() == 1 => Some(&self.keys[0]),
            None => None,
        }
    }
}

/// Pluggable transport for JWKS fetches. Production wires reqwest +
/// rustls; tests inject [`StubTransport`].
pub trait JwksTransport: Send + Sync {
    fn fetch(&self, url: &str) -> MeshResult<String>;
}

#[derive(Default)]
pub struct StubTransport {
    bodies: RwLock<HashMap<String, MeshResult<String>>>,
}

impl StubTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, url: impl Into<String>, body: impl Into<String>) {
        self.bodies
            .write()
            .unwrap()
            .insert(url.into(), Ok(body.into()));
    }

    pub fn set_error(&self, url: impl Into<String>, err: MeshError) {
        self.bodies.write().unwrap().insert(url.into(), Err(err));
    }
}

impl JwksTransport for StubTransport {
    fn fetch(&self, url: &str) -> MeshResult<String> {
        let bodies = self.bodies.read().unwrap();
        match bodies.get(url) {
            Some(Ok(b)) => Ok(b.clone()),
            Some(Err(e)) => Err(MeshError::Jwt(format!("stub error: {e}"))),
            None => Err(MeshError::Jwt(format!("no stub body for {url}"))),
        }
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    set: JwkSet,
    fetched_at: Instant,
}

/// Per-issuer JWKS cache. Looks up keys, fetching from the configured
/// `jwks_uri` and refreshing past TTL.
pub struct JwksCache {
    /// `issuer → entry`.
    entries: Arc<Mutex<HashMap<String, CacheEntry>>>,
    /// `issuer → jwks_uri`.
    issuers: Arc<RwLock<HashMap<String, String>>>,
    transport: Arc<dyn JwksTransport>,
    ttl: Duration,
}

impl JwksCache {
    pub fn new(transport: Arc<dyn JwksTransport>, ttl: Duration) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            issuers: Arc::new(RwLock::new(HashMap::new())),
            transport,
            ttl,
        }
    }

    pub fn register_issuer(&self, issuer: impl Into<String>, jwks_uri: impl Into<String>) {
        self.issuers
            .write()
            .unwrap()
            .insert(issuer.into(), jwks_uri.into());
    }

    /// Ensure the JWKS for `issuer` is loaded and not past TTL.
    /// Returns a clone of the cached key set.
    pub fn fetch(&self, issuer: &str) -> MeshResult<JwkSet> {
        // Fast-path: cached and fresh.
        {
            let entries = self.entries.lock().unwrap();
            if let Some(e) = entries.get(issuer) {
                if e.fetched_at.elapsed() < self.ttl {
                    return Ok(e.set.clone());
                }
            }
        }
        // Slow-path: fetch.
        let uri = self
            .issuers
            .read()
            .unwrap()
            .get(issuer)
            .cloned()
            .ok_or_else(|| MeshError::Jwt(format!("no jwks_uri registered for {issuer}")))?;
        let body = self.transport.fetch(&uri)?;
        let set = JwkSet::parse(&body)?;
        let mut entries = self.entries.lock().unwrap();
        entries.insert(
            issuer.to_string(),
            CacheEntry {
                set: set.clone(),
                fetched_at: Instant::now(),
            },
        );
        Ok(set)
    }

    /// Force a refresh, regardless of TTL.
    pub fn force_refresh(&self, issuer: &str) -> MeshResult<JwkSet> {
        let uri = self
            .issuers
            .read()
            .unwrap()
            .get(issuer)
            .cloned()
            .ok_or_else(|| MeshError::Jwt(format!("no jwks_uri registered for {issuer}")))?;
        let body = self.transport.fetch(&uri)?;
        let set = JwkSet::parse(&body)?;
        let mut entries = self.entries.lock().unwrap();
        entries.insert(
            issuer.to_string(),
            CacheEntry {
                set: set.clone(),
                fetched_at: Instant::now(),
            },
        );
        Ok(set)
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

/// Per-issuer validation policy that drives `jsonwebtoken::Validation`.
#[derive(Debug, Clone)]
pub struct IssuerValidation {
    pub issuer: String,
    pub audiences: Vec<String>,
    pub jwks_uri: String,
    pub allowed_algs: Vec<Algorithm>,
    /// Leeway in seconds for `exp`/`nbf` to absorb clock skew.
    pub leeway_seconds: u64,
}

impl IssuerValidation {
    pub fn to_validation(&self, alg: Algorithm) -> Validation {
        let mut v = Validation::new(alg);
        v.set_issuer(&[self.issuer.clone()]);
        if !self.audiences.is_empty() {
            v.set_audience(&self.audiences);
        } else {
            // No audience restriction → don't enforce.
            v.validate_aud = false;
        }
        v.leeway = self.leeway_seconds;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rsa_jwks_body() -> &'static str {
        // A toy RSA JWK with bogus-but-syntactically-valid base64url
        // components — we exercise parse + algorithm() + find(), not
        // the cryptographic verification step.
        r#"{
          "keys": [
            {
              "kty": "RSA",
              "kid": "key-1",
              "alg": "RS256",
              "use": "sig",
              "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
              "e": "AQAB"
            },
            {
              "kty": "RSA",
              "kid": "key-2",
              "n": "AQAB",
              "e": "AQAB"
            }
          ]
        }"#
    }

    #[test]
    fn parse_jwks_returns_typed_keys() {
        let set = JwkSet::parse(rsa_jwks_body()).unwrap();
        assert_eq!(set.keys.len(), 2);
        assert_eq!(set.keys[0].kid.as_deref(), Some("key-1"));
        assert_eq!(set.keys[0].kty, "RSA");
    }

    #[test]
    fn parse_rejects_garbage() {
        let err = JwkSet::parse("not-json").unwrap_err();
        assert!(matches!(err, MeshError::Jwt(_)));
    }

    #[test]
    fn find_by_kid_returns_matching_key() {
        let set = JwkSet::parse(rsa_jwks_body()).unwrap();
        let k = set.find(Some("key-2")).unwrap();
        assert_eq!(k.kid.as_deref(), Some("key-2"));
    }

    #[test]
    fn find_without_kid_only_works_for_single_key_set() {
        let single = r#"{"keys":[{"kty":"oct","k":"c2VjcmV0"}]}"#;
        let set = JwkSet::parse(single).unwrap();
        assert!(set.find(None).is_some());
        // Multi-key set: ambiguous, returns None.
        let multi = JwkSet::parse(rsa_jwks_body()).unwrap();
        assert!(multi.find(None).is_none());
    }

    #[test]
    fn rsa_key_builds_decoding_key() {
        let set = JwkSet::parse(rsa_jwks_body()).unwrap();
        let k = set.find(Some("key-1")).unwrap();
        // The toy n/e are valid base64url; jsonwebtoken accepts.
        k.decoding_key()
            .expect("RSA key with valid b64 components should build");
    }

    #[test]
    fn missing_n_for_rsa_errors() {
        let raw = r#"{"keys":[{"kty":"RSA","e":"AQAB"}]}"#;
        let set = JwkSet::parse(raw).unwrap();
        let k = &set.keys[0];
        assert!(matches!(
            k.decoding_key().map(|_| ()).unwrap_err(),
            MeshError::Jwt(_)
        ));
    }

    #[test]
    fn unsupported_kty_errors() {
        let raw = r#"{"keys":[{"kty":"PGP"}]}"#;
        let set = JwkSet::parse(raw).unwrap();
        assert!(matches!(
            set.keys[0].decoding_key().map(|_| ()).unwrap_err(),
            MeshError::Jwt(_)
        ));
    }

    #[test]
    fn algorithm_defaults_per_kty_when_alg_absent() {
        let raw = r#"{"keys":[{"kty":"RSA","n":"AQ","e":"AQ"},{"kty":"oct","k":"c2VjcmV0"}]}"#;
        let set = JwkSet::parse(raw).unwrap();
        assert_eq!(set.keys[0].algorithm().unwrap(), Algorithm::RS256);
        assert_eq!(set.keys[1].algorithm().unwrap(), Algorithm::HS256);
    }

    #[test]
    fn cache_fetches_and_returns_on_subsequent_call_within_ttl() {
        let stub = Arc::new(StubTransport::new());
        stub.set("https://idp.example/.well-known/jwks.json", rsa_jwks_body());
        let cache = JwksCache::new(stub, Duration::from_secs(60));
        cache.register_issuer(
            "https://idp.example",
            "https://idp.example/.well-known/jwks.json",
        );
        let first = cache.fetch("https://idp.example").unwrap();
        let second = cache.fetch("https://idp.example").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn cache_refresh_after_ttl_picks_up_new_keys() {
        let stub = Arc::new(StubTransport::new());
        stub.set("https://idp.example/.well-known/jwks.json", rsa_jwks_body());
        let cache = JwksCache::new(stub.clone(), Duration::from_millis(20));
        cache.register_issuer(
            "https://idp.example",
            "https://idp.example/.well-known/jwks.json",
        );
        let first = cache.fetch("https://idp.example").unwrap();
        assert_eq!(first.keys.len(), 2);
        // Rotate keys
        stub.set(
            "https://idp.example/.well-known/jwks.json",
            r#"{"keys":[{"kty":"oct","k":"c2VjcmV0","kid":"newkey"}]}"#,
        );
        std::thread::sleep(Duration::from_millis(30));
        let after = cache.fetch("https://idp.example").unwrap();
        assert_eq!(after.keys.len(), 1);
        assert_eq!(after.keys[0].kid.as_deref(), Some("newkey"));
    }

    #[test]
    fn force_refresh_ignores_ttl() {
        let stub = Arc::new(StubTransport::new());
        stub.set("https://idp.example/.well-known/jwks.json", rsa_jwks_body());
        let cache = JwksCache::new(stub.clone(), Duration::from_secs(3600));
        cache.register_issuer(
            "https://idp.example",
            "https://idp.example/.well-known/jwks.json",
        );
        cache.fetch("https://idp.example").unwrap();
        stub.set(
            "https://idp.example/.well-known/jwks.json",
            r#"{"keys":[{"kty":"oct","k":"c2VjcmV0","kid":"forced"}]}"#,
        );
        let after = cache.force_refresh("https://idp.example").unwrap();
        assert_eq!(after.keys[0].kid.as_deref(), Some("forced"));
    }

    #[test]
    fn fetch_unknown_issuer_errors() {
        let stub = Arc::new(StubTransport::new());
        let cache = JwksCache::new(stub, Duration::from_secs(60));
        assert!(matches!(
            cache.fetch("nope").unwrap_err(),
            MeshError::Jwt(_)
        ));
    }

    #[test]
    fn transport_error_surfaces_to_caller() {
        let stub = Arc::new(StubTransport::new());
        let cache = JwksCache::new(stub, Duration::from_secs(60));
        cache.register_issuer("iss", "https://nowhere");
        assert!(matches!(cache.fetch("iss").unwrap_err(), MeshError::Jwt(_)));
    }

    #[test]
    fn issuer_validation_builds_with_audiences_and_leeway() {
        let iv = IssuerValidation {
            issuer: "iss".into(),
            audiences: vec!["a".into(), "b".into()],
            jwks_uri: "x".into(),
            allowed_algs: vec![Algorithm::RS256],
            leeway_seconds: 30,
        };
        let v = iv.to_validation(Algorithm::RS256);
        assert_eq!(v.leeway, 30);
        assert!(v.validate_aud);
    }

    #[test]
    fn issuer_validation_disables_aud_check_when_empty() {
        let iv = IssuerValidation {
            issuer: "iss".into(),
            audiences: vec![],
            jwks_uri: "x".into(),
            allowed_algs: vec![Algorithm::RS256],
            leeway_seconds: 0,
        };
        let v = iv.to_validation(Algorithm::RS256);
        assert!(!v.validate_aud);
    }
}
