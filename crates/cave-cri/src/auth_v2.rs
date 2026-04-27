//! Image pull auth, second layer.
//!
//! Builds on `crate::auth`'s scheme resolver. Adds:
//!
//! - **TokenCache** — per-(registry, scope) bearer cache with TTL,
//!   matching containerd's `remotes/docker/auth/fetch.go`.
//! - **AWS Signature v4** — canonical-request, string-to-sign,
//!   signing-key derivation, and the `Authorization` header for
//!   `ecr:GetAuthorizationToken` calls.
//! - **dockerconfigjson parser** — reads the `auths` map from
//!   Kubernetes' `kubernetes.io/dockerconfigjson` Secret payload (also
//!   the format kubectl writes to `~/.docker/config.json`).
//! - **Pull retry policy** — exponential backoff with jitter, the same
//!   recipe `containerd/remotes/docker/scheduler.go` uses.
//! - **Bearer-challenge parser** — extracts realm/service/scope from a
//!   `WWW-Authenticate: Bearer …` header.

use crate::auth::AuthScheme;
use crate::error::{CriError, CriResult};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

// ── TokenCache ───────────────────────────────────────────────────────────────

/// One cached registry token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl CachedToken {
    /// True if the token has not yet hit its expiry. Mirrors
    /// containerd's behaviour of expiring tokens 30 seconds before
    /// their stated expiry to avoid clock skew.
    pub fn is_valid(&self, now: DateTime<Utc>) -> bool {
        self.expires_at > now + Duration::seconds(30)
    }
}

/// Per-(registry, scope) bearer cache.
#[derive(Debug, Default)]
pub struct TokenCache {
    entries: RwLock<HashMap<TokenKey, CachedToken>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenKey {
    pub registry: String,
    pub scope: String,
}

impl TokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, registry: &str, scope: &str, token: CachedToken) {
        self.entries.write().unwrap().insert(
            TokenKey { registry: registry.into(), scope: scope.into() },
            token,
        );
    }

    /// Look up a still-valid token; returns `None` if missing or
    /// within the 30-second skew window.
    pub fn get(&self, registry: &str, scope: &str, now: DateTime<Utc>) -> Option<CachedToken> {
        let key = TokenKey { registry: registry.into(), scope: scope.into() };
        let entries = self.entries.read().unwrap();
        let token = entries.get(&key)?;
        if token.is_valid(now) {
            Some(token.clone())
        } else {
            None
        }
    }

    /// Drop expired entries. Returns the number removed.
    pub fn evict_expired(&self, now: DateTime<Utc>) -> usize {
        let mut entries = self.entries.write().unwrap();
        let before = entries.len();
        entries.retain(|_, t| t.is_valid(now));
        before - entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }
}

// ── Bearer challenge parser ──────────────────────────────────────────────────

/// `WWW-Authenticate: Bearer realm="…",service="…",scope="…"` parser.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BearerChallenge {
    pub realm: String,
    pub service: String,
    pub scope: String,
}

impl BearerChallenge {
    /// Parse a single `WWW-Authenticate` header value. Mirrors
    /// containerd's `remotes/docker/auth/parse.go`.
    pub fn parse(header: &str) -> CriResult<BearerChallenge> {
        let trimmed = header.trim();
        let body = trimmed
            .strip_prefix("Bearer ")
            .or_else(|| trimmed.strip_prefix("bearer "))
            .ok_or_else(|| CriError::Registry("WWW-Authenticate must start with Bearer".into()))?;

        let mut out = BearerChallenge::default();
        for kv in body.split(',') {
            let kv = kv.trim();
            let (key, raw_value) = match kv.split_once('=') {
                Some(p) => p,
                None => continue,
            };
            let value = raw_value.trim().trim_matches('"');
            match key.trim() {
                "realm"   => out.realm   = value.to_string(),
                "service" => out.service = value.to_string(),
                "scope"   => out.scope   = value.to_string(),
                _ => {}
            }
        }
        if out.realm.is_empty() {
            return Err(CriError::Registry("missing realm in WWW-Authenticate".into()));
        }
        Ok(out)
    }

    /// Build the URL the runtime should hit to exchange this challenge
    /// for a bearer token.
    pub fn token_url(&self) -> String {
        let mut url = self.realm.clone();
        let mut params = Vec::new();
        if !self.service.is_empty() {
            params.push(format!("service={}", self.service));
        }
        if !self.scope.is_empty() {
            params.push(format!("scope={}", self.scope));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }
        url
    }
}

// ── AWS Signature v4 ─────────────────────────────────────────────────────────

/// Inputs for an AWS Signature v4 signature. We compute everything
/// purely so it's testable without a network round-trip; the result is
/// the value of the `Authorization` header.
#[derive(Debug, Clone)]
pub struct SigV4Request<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub payload: &'a [u8],
    pub region: &'a str,
    pub service: &'a str,
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub timestamp: DateTime<Utc>,
}

impl<'a> SigV4Request<'a> {
    /// Compute the `Authorization` header value:
    /// `AWS4-HMAC-SHA256 Credential=<key>/<scope>, SignedHeaders=<headers>, Signature=<hex>`.
    pub fn authorization_header(&self) -> String {
        use sha2::{Digest, Sha256};

        let amz_date = self.timestamp.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = self.timestamp.format("%Y%m%d").to_string();

        // Canonical request:
        //   <Method>\n<CanonicalURI>\n<CanonicalQueryString>\n
        //   <CanonicalHeaders>\n<SignedHeaders>\n<HashedPayload>
        let payload_hash = hex::encode(Sha256::digest(self.payload));
        let canonical_headers = format!("host:{}\nx-amz-date:{}\n", self.host, amz_date);
        let signed_headers = "host;x-amz-date";
        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            self.method,
            if self.path.is_empty() { "/" } else { self.path },
            self.query,
            canonical_headers,
            signed_headers,
            payload_hash,
        );

        // String to sign:
        //   AWS4-HMAC-SHA256\n<amz-date>\n<scope>\n<hash(canonical_request)>
        let scope = format!("{}/{}/{}/aws4_request", date_stamp, self.region, self.service);
        let cr_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date, scope, cr_hash,
        );

        // Signing key derivation:
        //   k_date    = HMAC("AWS4"+secret, date_stamp)
        //   k_region  = HMAC(k_date, region)
        //   k_service = HMAC(k_region, service)
        //   k_signing = HMAC(k_service, "aws4_request")
        let k_secret = format!("AWS4{}", self.secret_access_key);
        let k_date    = hmac_sha256(k_secret.as_bytes(), date_stamp.as_bytes());
        let k_region  = hmac_sha256(&k_date, self.region.as_bytes());
        let k_service = hmac_sha256(&k_region, self.service.as_bytes());
        let k_signing = hmac_sha256(&k_service, b"aws4_request");

        let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key_id, scope, signed_headers, signature,
        )
    }
}

/// HMAC-SHA256 — small dependency-free implementation so we don't pull
/// the `hmac` crate just for this one signature path.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    const BLOCK_SIZE: usize = 64;
    let mut k = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let h = Sha256::digest(key);
        k[..32].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut o_pad = [0x5cu8; BLOCK_SIZE];
    let mut i_pad = [0x36u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        o_pad[i] ^= k[i];
        i_pad[i] ^= k[i];
    }
    let inner = Sha256::digest([&i_pad[..], message].concat());
    let outer = Sha256::digest([&o_pad[..], &inner[..]].concat());
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer);
    out
}

// ── dockerconfigjson parser ──────────────────────────────────────────────────

/// One entry in `~/.docker/config.json`'s `auths` map. The `auth` field
/// is base64(`username:password`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerConfigEntry {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub auth: Option<String>,
    #[serde(default)]
    pub identitytoken: Option<String>,
    #[serde(default)]
    pub registrytoken: Option<String>,
}

/// Top-level shape of `~/.docker/config.json` /
/// `kubernetes.io/dockerconfigjson` Secret payloads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerConfig {
    #[serde(default)]
    pub auths: HashMap<String, DockerConfigEntry>,
}

impl DockerConfig {
    pub fn parse(json: &str) -> CriResult<DockerConfig> {
        serde_json::from_str(json)
            .map_err(|e| CriError::Registry(format!("dockerconfigjson parse: {}", e)))
    }

    /// Materialise an `AuthScheme` for `registry`. Looks up the entry,
    /// preferring `identitytoken` (Bearer) → `registrytoken` (Bearer) →
    /// `auth` (Basic) → `username`/`password` (Basic).
    pub fn scheme_for(&self, registry: &str) -> Option<AuthScheme> {
        let entry = self.auths.get(registry)?;
        if let Some(t) = entry.identitytoken.as_deref().filter(|s| !s.is_empty()) {
            return Some(AuthScheme::Bearer { token: t.into() });
        }
        if let Some(t) = entry.registrytoken.as_deref().filter(|s| !s.is_empty()) {
            return Some(AuthScheme::Bearer { token: t.into() });
        }
        if let Some(b64) = entry.auth.as_deref().filter(|s| !s.is_empty()) {
            if let Some((u, p)) = base64_decode_userpass(b64) {
                return Some(AuthScheme::Basic { username: u, password: p });
            }
        }
        if let (Some(u), Some(p)) = (&entry.username, &entry.password) {
            return Some(AuthScheme::Basic { username: u.clone(), password: p.clone() });
        }
        None
    }
}

fn base64_decode_userpass(b64: &str) -> Option<(String, String)> {
    let bytes = base64_decode(b64)?;
    let s = String::from_utf8(bytes).ok()?;
    let (u, p) = s.split_once(':')?;
    Some((u.to_string(), p.to_string()))
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let table: [i8; 256] = {
        let mut t = [-1i8; 256];
        for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
            t[*c as usize] = i as i8;
        }
        t
    };
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input.as_bytes() {
        if c == b'=' {
            break;
        }
        let v = table[c as usize];
        if v < 0 {
            // Skip whitespace; reject anything else.
            if c.is_ascii_whitespace() { continue; }
            return None;
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1u32 << bits) - 1;
        }
    }
    Some(out)
}

// ── Pull retry policy ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullRetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub multiplier: u32,
    pub max_backoff_ms: u64,
    /// Jitter as a fraction of the planned backoff (0.0 → none).
    /// 0.2 means each delay is `delay * (0.8..1.2)`.
    pub jitter_fraction: f64,
}

impl Default for PullRetryPolicy {
    /// containerd's default — 3 attempts, 250 ms..5 s backoff with 20% jitter.
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 250,
            multiplier: 2,
            max_backoff_ms: 5_000,
            jitter_fraction: 0.2,
        }
    }
}

impl PullRetryPolicy {
    /// Backoff for the Nth attempt (1-indexed). Returns `None` once we
    /// have exhausted `max_attempts`.
    pub fn backoff_for_attempt(&self, attempt: u32) -> Option<u64> {
        if attempt == 0 || attempt >= self.max_attempts {
            return None;
        }
        let mut delay = self.initial_backoff_ms;
        for _ in 1..attempt {
            delay = delay.saturating_mul(self.multiplier as u64);
            if delay >= self.max_backoff_ms {
                delay = self.max_backoff_ms;
                break;
            }
        }
        Some(delay)
    }

    /// Apply jitter — `delay * (1 + jitter_fraction * (2*rand - 1))`.
    /// `rand` is supplied so the test can pass a deterministic value.
    pub fn jittered(&self, base_delay_ms: u64, rand_unit: f64) -> u64 {
        let r = rand_unit.clamp(0.0, 1.0);
        let factor = 1.0 + self.jitter_fraction * (2.0 * r - 1.0);
        ((base_delay_ms as f64) * factor).max(0.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(unix: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(unix, 0).unwrap()
    }

    // ── TokenCache ────────────────────────────────────────────────────────────

    #[test]
    fn token_is_valid_until_thirty_seconds_before_expiry() {
        let t = CachedToken { token: "x".into(), expires_at: ts(1000) };
        assert!(t.is_valid(ts(900)));
        assert!(!t.is_valid(ts(990))); // within 30s window
        assert!(!t.is_valid(ts(1001)));
    }

    #[test]
    fn cache_get_returns_none_for_missing() {
        let c = TokenCache::new();
        assert!(c.get("docker.io", "repo:nginx:pull", ts(0)).is_none());
    }

    #[test]
    fn cache_get_returns_inserted_token() {
        let c = TokenCache::new();
        c.insert("docker.io", "repo:nginx:pull", CachedToken { token: "tk".into(), expires_at: ts(2000) });
        let got = c.get("docker.io", "repo:nginx:pull", ts(100)).unwrap();
        assert_eq!(got.token, "tk");
    }

    #[test]
    fn cache_get_returns_none_when_expired() {
        let c = TokenCache::new();
        c.insert("r", "s", CachedToken { token: "tk".into(), expires_at: ts(1000) });
        assert!(c.get("r", "s", ts(2000)).is_none());
    }

    #[test]
    fn cache_evict_expired_removes_old_entries() {
        let c = TokenCache::new();
        c.insert("a", "s", CachedToken { token: "old".into(), expires_at: ts(100) });
        c.insert("b", "s", CachedToken { token: "fresh".into(), expires_at: ts(10_000) });
        assert_eq!(c.len(), 2);
        let removed = c.evict_expired(ts(500));
        assert_eq!(removed, 1);
        assert_eq!(c.len(), 1);
        assert!(c.get("b", "s", ts(500)).is_some());
    }

    #[test]
    fn cache_keys_distinguish_scope() {
        let c = TokenCache::new();
        c.insert("r", "scope1", CachedToken { token: "a".into(), expires_at: ts(2000) });
        c.insert("r", "scope2", CachedToken { token: "b".into(), expires_at: ts(2000) });
        assert_eq!(c.get("r", "scope1", ts(0)).unwrap().token, "a");
        assert_eq!(c.get("r", "scope2", ts(0)).unwrap().token, "b");
    }

    // ── BearerChallenge ──────────────────────────────────────────────────────

    #[test]
    fn parse_bearer_challenge_with_all_fields() {
        let h = r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull""#;
        let c = BearerChallenge::parse(h).unwrap();
        assert_eq!(c.realm,   "https://auth.docker.io/token");
        assert_eq!(c.service, "registry.docker.io");
        assert_eq!(c.scope,   "repository:library/nginx:pull");
    }

    #[test]
    fn parse_bearer_lowercase_token_type_ok() {
        let h = r#"bearer realm="https://x/y""#;
        let c = BearerChallenge::parse(h).unwrap();
        assert_eq!(c.realm, "https://x/y");
    }

    #[test]
    fn parse_bearer_missing_realm_errors() {
        let h = r#"Bearer service="x""#;
        assert!(BearerChallenge::parse(h).is_err());
    }

    #[test]
    fn parse_bearer_non_bearer_scheme_errors() {
        assert!(BearerChallenge::parse("Basic realm=\"x\"").is_err());
    }

    #[test]
    fn challenge_token_url_includes_service_and_scope() {
        let c = BearerChallenge {
            realm: "https://auth.docker.io/token".into(),
            service: "registry.docker.io".into(),
            scope: "repository:library/nginx:pull".into(),
        };
        let url = c.token_url();
        assert!(url.contains("service=registry.docker.io"));
        assert!(url.contains("scope=repository:library/nginx:pull"));
    }

    #[test]
    fn challenge_token_url_omits_empty_scope() {
        let c = BearerChallenge { realm: "https://x/y".into(), service: "s".into(), scope: "".into() };
        let url = c.token_url();
        assert!(url.contains("service=s"));
        assert!(!url.contains("scope="));
    }

    // ── SigV4 ────────────────────────────────────────────────────────────────

    fn fixture_sigv4() -> SigV4Request<'static> {
        SigV4Request {
            method: "POST",
            host: "api.ecr.us-east-1.amazonaws.com",
            path: "/",
            query: "",
            payload: b"{}",
            region: "us-east-1",
            service: "ecr",
            access_key_id: "AKIDEXAMPLE",
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn sigv4_header_starts_with_algorithm() {
        let h = fixture_sigv4().authorization_header();
        assert!(h.starts_with("AWS4-HMAC-SHA256 "));
    }

    #[test]
    fn sigv4_header_contains_credential_with_scope() {
        let h = fixture_sigv4().authorization_header();
        assert!(h.contains("Credential=AKIDEXAMPLE/20240101/us-east-1/ecr/aws4_request"));
    }

    #[test]
    fn sigv4_header_signed_headers_are_host_and_amz_date() {
        let h = fixture_sigv4().authorization_header();
        assert!(h.contains("SignedHeaders=host;x-amz-date"));
    }

    #[test]
    fn sigv4_header_signature_is_64_hex_chars() {
        let h = fixture_sigv4().authorization_header();
        let sig = h.rsplit("Signature=").next().unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sigv4_signature_changes_with_payload() {
        let mut a = fixture_sigv4();
        let mut b = fixture_sigv4();
        a.payload = b"";
        b.payload = b"{\"x\":1}";
        assert_ne!(a.authorization_header(), b.authorization_header());
    }

    #[test]
    fn sigv4_signature_changes_with_timestamp() {
        let mut b = fixture_sigv4();
        b.timestamp = Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        assert_ne!(fixture_sigv4().authorization_header(), b.authorization_header());
    }

    #[test]
    fn sigv4_signature_changes_with_region() {
        let mut b = fixture_sigv4();
        b.region = "eu-west-1";
        assert_ne!(fixture_sigv4().authorization_header(), b.authorization_header());
    }

    #[test]
    fn hmac_sha256_known_vector() {
        // RFC 4231 test case 1: key=20*0x0b, data="Hi There".
        let key = [0x0bu8; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        let expected = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";
        assert_eq!(hex::encode(mac), expected);
    }

    // ── DockerConfig ─────────────────────────────────────────────────────────

    #[test]
    fn parse_dockerconfigjson_with_auth_field() {
        let json = r#"{"auths":{"docker.io":{"auth":"YWxpY2U6c2VjcmV0"}}}"#;
        let cfg = DockerConfig::parse(json).unwrap();
        let scheme = cfg.scheme_for("docker.io").unwrap();
        match scheme {
            AuthScheme::Basic { username, password } => {
                assert_eq!(username, "alice");
                assert_eq!(password, "secret");
            }
            _ => panic!("expected Basic"),
        }
    }

    #[test]
    fn parse_dockerconfigjson_with_username_password() {
        let json = r#"{"auths":{"r.io":{"username":"u","password":"p"}}}"#;
        let cfg = DockerConfig::parse(json).unwrap();
        let scheme = cfg.scheme_for("r.io").unwrap();
        match scheme {
            AuthScheme::Basic { username, password } => {
                assert_eq!(username, "u");
                assert_eq!(password, "p");
            }
            _ => panic!("expected Basic"),
        }
    }

    #[test]
    fn parse_dockerconfigjson_prefers_identitytoken_over_auth() {
        let json = r#"{"auths":{"r.io":{"auth":"YWxpY2U6c2VjcmV0","identitytoken":"id-tk"}}}"#;
        let cfg = DockerConfig::parse(json).unwrap();
        let scheme = cfg.scheme_for("r.io").unwrap();
        match scheme {
            AuthScheme::Bearer { token } => assert_eq!(token, "id-tk"),
            _ => panic!("expected Bearer"),
        }
    }

    #[test]
    fn parse_dockerconfigjson_falls_back_to_registrytoken() {
        let json = r#"{"auths":{"r.io":{"registrytoken":"reg-tk"}}}"#;
        let cfg = DockerConfig::parse(json).unwrap();
        let scheme = cfg.scheme_for("r.io").unwrap();
        match scheme {
            AuthScheme::Bearer { token } => assert_eq!(token, "reg-tk"),
            _ => panic!("expected Bearer"),
        }
    }

    #[test]
    fn parse_dockerconfigjson_unknown_registry_returns_none() {
        let json = r#"{"auths":{"docker.io":{"username":"u","password":"p"}}}"#;
        let cfg = DockerConfig::parse(json).unwrap();
        assert!(cfg.scheme_for("ghost").is_none());
    }

    #[test]
    fn parse_dockerconfigjson_invalid_json_errors() {
        assert!(DockerConfig::parse("not json").is_err());
    }

    #[test]
    fn parse_dockerconfigjson_empty_auths_ok() {
        let cfg = DockerConfig::parse(r#"{"auths":{}}"#).unwrap();
        assert!(cfg.auths.is_empty());
    }

    // ── PullRetryPolicy ──────────────────────────────────────────────────────

    #[test]
    fn retry_policy_defaults_match_containerd() {
        let p = PullRetryPolicy::default();
        assert_eq!(p.max_attempts, 3);
        assert_eq!(p.initial_backoff_ms, 250);
        assert_eq!(p.multiplier, 2);
        assert_eq!(p.max_backoff_ms, 5_000);
    }

    #[test]
    fn backoff_returns_none_after_max_attempts() {
        let p = PullRetryPolicy::default();
        assert!(p.backoff_for_attempt(3).is_none());
        assert!(p.backoff_for_attempt(0).is_none());
    }

    #[test]
    fn backoff_grows_geometrically_until_capped() {
        let p = PullRetryPolicy { max_attempts: 10, initial_backoff_ms: 100, multiplier: 2, max_backoff_ms: 1000, jitter_fraction: 0.0 };
        assert_eq!(p.backoff_for_attempt(1), Some(100));
        assert_eq!(p.backoff_for_attempt(2), Some(200));
        assert_eq!(p.backoff_for_attempt(3), Some(400));
        assert_eq!(p.backoff_for_attempt(4), Some(800));
        // Capped at max_backoff_ms.
        assert_eq!(p.backoff_for_attempt(5), Some(1000));
        assert_eq!(p.backoff_for_attempt(6), Some(1000));
    }

    #[test]
    fn jittered_at_zero_rand_subtracts_max_jitter() {
        let p = PullRetryPolicy { jitter_fraction: 0.5, ..Default::default() };
        // r=0 → factor = 1 + 0.5 * (-1) = 0.5
        assert_eq!(p.jittered(1000, 0.0), 500);
    }

    #[test]
    fn jittered_at_one_rand_adds_max_jitter() {
        let p = PullRetryPolicy { jitter_fraction: 0.5, ..Default::default() };
        // r=1 → factor = 1 + 0.5 * 1 = 1.5
        assert_eq!(p.jittered(1000, 1.0), 1500);
    }

    #[test]
    fn jittered_at_half_rand_returns_base() {
        let p = PullRetryPolicy { jitter_fraction: 0.5, ..Default::default() };
        assert_eq!(p.jittered(1000, 0.5), 1000);
    }

    #[test]
    fn jittered_clamps_negative_rand_to_zero() {
        let p = PullRetryPolicy { jitter_fraction: 0.5, ..Default::default() };
        assert_eq!(p.jittered(1000, -5.0), p.jittered(1000, 0.0));
    }

    // ── base64_decode internal helper ────────────────────────────────────────

    #[test]
    fn base64_decode_basic_handles_padding() {
        // "fo" → "Zm8=", "f" → "Zg=="
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
    }

    #[test]
    fn base64_decode_userpass_splits_on_colon() {
        let (u, p) = base64_decode_userpass("YWxpY2U6c2VjcmV0").unwrap();
        assert_eq!(u, "alice");
        assert_eq!(p, "secret");
    }

    #[test]
    fn base64_decode_invalid_returns_none() {
        assert!(base64_decode("***bad***").is_none());
    }
}
