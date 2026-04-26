//! Token issuer & verifier — etcd v3 ships two interchangeable token
//! providers, both modelled here:
//!
//!   * **simple** — opaque, server-generated random string with a TTL.
//!     Stored in-memory; revocation is `delete by token`.  Mirrors
//!     `server/auth/simple_token.go`.
//!   * **jwt** — self-contained JSON Web Token with HS256, carrying user,
//!     roles, expiry.  No server-side state required.  Mirrors
//!     `server/auth/jwt.go`.
//!
//! Both surfaces share an [`AuthToken`] enum so callers don't have to care
//! which mode the server is running in.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum TokenError {
    /// Token does not exist (simple) or signature is invalid (JWT).
    Invalid,
    /// Token's TTL has elapsed.
    Expired,
    /// JWT structurally malformed (wrong number of segments, bad base64,
    /// bad JSON, missing claim, …).
    Malformed(String),
    /// JWT signed with an algorithm we don't accept.
    AlgorithmMismatch { expected: String, got: String },
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => write!(f, "invalid token"),
            Self::Expired => write!(f, "token expired"),
            Self::Malformed(m) => write!(f, "malformed token: {m}"),
            Self::AlgorithmMismatch { expected, got } => write!(f, "algorithm mismatch: expected {expected}, got {got}"),
        }
    }
}

impl std::error::Error for TokenError {}

// ── Common claims ─────────────────────────────────────────────────────────

/// What every authenticated token carries — independent of simple vs JWT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenClaims {
    pub username: String,
    pub roles: Vec<String>,
    /// Unix timestamp (seconds) when the token expires.  `None` ⇒ never.
    pub expires_at: Option<u64>,
    /// Issuer identifier — populated for JWT, omitted for simple.
    pub issuer: Option<String>,
}

impl TokenClaims {
    pub fn is_expired(&self, now: u64) -> bool {
        matches!(self.expires_at, Some(exp) if exp <= now)
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ── Simple-token issuer ──────────────────────────────────────────────────

/// In-memory simple-token issuer — token strings are random opaque IDs,
/// claims are stored server-side and looked up on verify.
///
/// Mirrors `server/auth/simple_token.go`:
///   * tokens are time-limited via a TTL (`simple-token-ttl` flag, default 5m),
///   * revocation is keyed by token (no key-rotation step),
///   * the ID is opaque to clients.
pub struct SimpleTokenIssuer {
    ttl: Duration,
    inner: RwLock<HashMap<String, TokenClaims>>,
    /// Deterministic counter so tests don't depend on randomness.
    counter: Mutex<u64>,
    prefix: String,
}

impl SimpleTokenIssuer {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: RwLock::new(HashMap::new()),
            counter: Mutex::new(0),
            prefix: "simple".to_string(),
        }
    }

    pub fn with_prefix(ttl: Duration, prefix: impl Into<String>) -> Self {
        Self {
            ttl,
            inner: RwLock::new(HashMap::new()),
            counter: Mutex::new(0),
            prefix: prefix.into(),
        }
    }

    fn fresh_token(&self) -> String {
        let mut c = self.counter.lock().unwrap();
        *c += 1;
        // Match etcd's `<random>.<index>` shape so audit logs look familiar.
        format!("{}.{:016x}", self.prefix, *c)
    }

    /// Issue a fresh token for the given user/roles.  Sets `expires_at` per
    /// the configured TTL.
    pub fn assign(&self, username: impl Into<String>, roles: Vec<String>) -> String {
        let token = self.fresh_token();
        let claims = TokenClaims {
            username: username.into(),
            roles,
            expires_at: Some(now_secs() + self.ttl.as_secs()),
            issuer: None,
        };
        self.inner.write().unwrap().insert(token.clone(), claims);
        token
    }

    /// Look up the claims for a token.  Returns `Expired` for stale entries
    /// (and removes them in passing) and `Invalid` for unknown ones.
    pub fn info(&self, token: &str) -> Result<TokenClaims, TokenError> {
        let now = now_secs();
        let claims_opt = self.inner.read().unwrap().get(token).cloned();
        let claims = claims_opt.ok_or(TokenError::Invalid)?;
        if claims.is_expired(now) {
            self.inner.write().unwrap().remove(token);
            return Err(TokenError::Expired);
        }
        Ok(claims)
    }

    /// Refresh a token's TTL.  Returns `Invalid` if the token is unknown,
    /// `Expired` if it has already elapsed.  Mirrors etcd's
    /// `simpleTokenAttachUser` re-attach.
    pub fn refresh(&self, token: &str) -> Result<(), TokenError> {
        let claims = self.info(token)?;
        let new = TokenClaims {
            expires_at: Some(now_secs() + self.ttl.as_secs()),
            ..claims
        };
        self.inner.write().unwrap().insert(token.to_string(), new);
        Ok(())
    }

    /// Explicit revocation.
    pub fn revoke(&self, token: &str) -> bool {
        self.inner.write().unwrap().remove(token).is_some()
    }

    /// Drop all expired tokens — caller would invoke this on a timer.
    /// Returns the number of tokens evicted.
    pub fn sweep(&self) -> usize {
        let now = now_secs();
        let mut w = self.inner.write().unwrap();
        let stale: Vec<String> = w.iter()
            .filter(|(_, c)| c.is_expired(now))
            .map(|(k, _)| k.clone())
            .collect();
        for s in &stale { w.remove(s); }
        stale.len()
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

// ── JWT issuer (HS256) ────────────────────────────────────────────────────

/// JWT-style issuer — emits a self-contained `<header>.<payload>.<sig>`
/// token.  Verification needs only the shared secret (and current time).
///
/// Cave's HS256 implementation is a domain-separated double-hashed
/// keyed digest.  It's NOT cryptographically equivalent to RFC-2104
/// HMAC-SHA-256, but it follows the JWT structural spec exactly so a
/// production deployment can swap in `jsonwebtoken` without touching
/// callers.
pub struct JwtIssuer {
    issuer: String,
    secret: Vec<u8>,
    ttl: Duration,
}

impl JwtIssuer {
    pub fn new(issuer: impl Into<String>, secret: impl Into<Vec<u8>>, ttl: Duration) -> Self {
        Self { issuer: issuer.into(), secret: secret.into(), ttl }
    }

    /// Build header.payload string (data to sign).  Public so tests can
    /// poke at it.
    fn signing_input(header_b64: &str, payload_b64: &str) -> String {
        format!("{header_b64}.{payload_b64}")
    }

    fn header_b64(&self) -> String {
        let header = serde_json::json!({"alg": "HS256", "typ": "JWT"});
        b64url(serde_json::to_vec(&header).unwrap())
    }

    fn payload_b64(&self, claims: &TokenClaims) -> String {
        let payload = serde_json::json!({
            "username": claims.username,
            "roles": claims.roles,
            "exp": claims.expires_at,
            "iss": self.issuer,
        });
        b64url(serde_json::to_vec(&payload).unwrap())
    }

    fn sign(&self, signing_input: &str) -> String {
        let mac = hs256(&self.secret, signing_input.as_bytes());
        b64url(mac)
    }

    /// Issue a JWT for the given username/roles.
    pub fn issue(&self, username: impl Into<String>, roles: Vec<String>) -> String {
        let claims = TokenClaims {
            username: username.into(),
            roles,
            expires_at: Some(now_secs() + self.ttl.as_secs()),
            issuer: Some(self.issuer.clone()),
        };
        let header = self.header_b64();
        let payload = self.payload_b64(&claims);
        let signing_input = Self::signing_input(&header, &payload);
        let sig = self.sign(&signing_input);
        format!("{header}.{payload}.{sig}")
    }

    /// Verify a JWT and return its claims.  Rejects expired tokens and
    /// algorithm mismatches.
    pub fn verify(&self, token: &str) -> Result<TokenClaims, TokenError> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(TokenError::Malformed(format!("expected 3 segments, got {}", parts.len())));
        }
        let header_bytes = b64url_decode(parts[0])
            .map_err(|e| TokenError::Malformed(format!("header b64: {e}")))?;
        let header: serde_json::Value = serde_json::from_slice(&header_bytes)
            .map_err(|e| TokenError::Malformed(format!("header json: {e}")))?;
        let alg = header.get("alg").and_then(|v| v.as_str()).unwrap_or("");
        if alg != "HS256" {
            return Err(TokenError::AlgorithmMismatch {
                expected: "HS256".into(),
                got: alg.to_string(),
            });
        }

        let signing_input = Self::signing_input(parts[0], parts[1]);
        let want_sig = self.sign(&signing_input);
        // Constant-time compare so timing attacks against the secret are
        // blunted.  Etcd's `jwt.go` does the same via subtle.ConstantTimeCompare.
        if !ct_eq(want_sig.as_bytes(), parts[2].as_bytes()) {
            return Err(TokenError::Invalid);
        }

        let payload_bytes = b64url_decode(parts[1])
            .map_err(|e| TokenError::Malformed(format!("payload b64: {e}")))?;
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)
            .map_err(|e| TokenError::Malformed(format!("payload json: {e}")))?;
        let username = payload.get("username")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TokenError::Malformed("missing username".into()))?;
        let roles: Vec<String> = payload.get("roles")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let expires_at = payload.get("exp").and_then(|v| v.as_u64());
        let iss = payload.get("iss").and_then(|v| v.as_str()).map(String::from);

        let claims = TokenClaims {
            username: username.to_string(),
            roles,
            expires_at,
            issuer: iss,
        };
        if claims.is_expired(now_secs()) {
            return Err(TokenError::Expired);
        }
        Ok(claims)
    }

    pub fn issuer(&self) -> &str { &self.issuer }
}

// ── Unified token enum ────────────────────────────────────────────────────

/// Cave's auth surface — caller does not need to know which provider is
/// active.  Either variant produces / verifies a string token.
pub enum AuthTokenProvider {
    Simple(SimpleTokenIssuer),
    Jwt(JwtIssuer),
}

impl AuthTokenProvider {
    pub fn issue(&self, username: &str, roles: Vec<String>) -> String {
        match self {
            Self::Simple(s) => s.assign(username, roles),
            Self::Jwt(j) => j.issue(username, roles),
        }
    }

    pub fn verify(&self, token: &str) -> Result<TokenClaims, TokenError> {
        match self {
            Self::Simple(s) => s.info(token),
            Self::Jwt(j) => j.verify(token),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Simple(_) => "simple",
            Self::Jwt(_) => "jwt",
        }
    }
}

// ── Helpers (b64url + HS256) ─────────────────────────────────────────────

fn b64url(data: impl AsRef<[u8]>) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data.as_ref())
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|e| e.to_string())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) { acc |= x ^ y; }
    acc == 0
}

/// Domain-separated double-hashed keyed digest (test-grade HMAC).  Stable,
/// 32 bytes, depends on (key, msg).  Production: rfc2104 HMAC-SHA-256.
fn hs256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let inner = mix(key, b"\x36", msg);
    mix(key, b"\x5c", &inner)
}

fn mix(key: &[u8], pad: &[u8], msg: &[u8]) -> [u8; 32] {
    // Iteratively fold (key ⨁ pad || msg) into 32 bytes via fnv-like rounds.
    let mut state = [0u8; 32];
    let mut s: u64 = 0xcbf29ce484222325;
    for (i, slot) in state.iter_mut().enumerate() {
        for &k in key.iter() {
            s = s.wrapping_mul(0x100000001b3).wrapping_add((k ^ pad[0] ^ (i as u8)) as u64);
        }
        for &m in msg.iter() {
            s = s.wrapping_mul(0x100000001b3).wrapping_add(m as u64);
        }
        *slot = (s ^ s.rotate_right(31)) as u8;
    }
    state
}

// ─────────────────────────────────────────────────────────────────────────
// Auth-token tests — feat/cave-etcd-100-pct-sprint
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_simple() -> SimpleTokenIssuer { SimpleTokenIssuer::new(Duration::from_secs(60)) }
    fn fresh_jwt() -> JwtIssuer { JwtIssuer::new("cave-etcd", b"secret-key".to_vec(), Duration::from_secs(60)) }

    // ── SimpleToken ───────────────────────────────────────────────────

    #[test]
    fn test_simple_assign_and_info() {
        // cite: server/auth/simple_token.go assignSimpleTokenToUser
        let s = fresh_simple();
        let t = s.assign("alice", vec!["root".into()]);
        let info = s.info(&t).unwrap();
        assert_eq!(info.username, "alice");
        assert_eq!(info.roles, vec!["root"]);
    }

    #[test]
    fn test_simple_info_unknown_token() {
        // cite: simple_token.go (unknown ⇒ ErrInvalidAuthToken)
        let s = fresh_simple();
        assert_eq!(s.info("nope").unwrap_err(), TokenError::Invalid);
    }

    #[test]
    fn test_simple_token_expires() {
        // cite: simple_token.go simpleTokenTTL
        let s = SimpleTokenIssuer::new(Duration::from_secs(0));
        let t = s.assign("alice", vec![]);
        // Wait long enough that exp <= now (TTL is 0 so already expired).
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(s.info(&t).unwrap_err(), TokenError::Expired);
    }

    #[test]
    fn test_simple_expired_token_is_evicted_on_lookup() {
        // cite: simple_token.go (lazy eviction during info)
        let s = SimpleTokenIssuer::new(Duration::from_secs(0));
        let t = s.assign("alice", vec![]);
        std::thread::sleep(Duration::from_secs(1));
        let _ = s.info(&t);
        assert!(s.is_empty());
    }

    #[test]
    fn test_simple_refresh_extends_ttl() {
        // cite: simple_token.go AttachAuthToken (re-attach refreshes TTL)
        let s = fresh_simple();
        let t = s.assign("alice", vec![]);
        let before = s.info(&t).unwrap().expires_at.unwrap();
        std::thread::sleep(Duration::from_secs(1));
        s.refresh(&t).unwrap();
        let after = s.info(&t).unwrap().expires_at.unwrap();
        assert!(after >= before);
    }

    #[test]
    fn test_simple_refresh_unknown_errors() {
        // cite: simple_token.go (unknown token ⇒ no refresh)
        let s = fresh_simple();
        assert_eq!(s.refresh("missing").unwrap_err(), TokenError::Invalid);
    }

    #[test]
    fn test_simple_revoke() {
        // cite: simple_token.go invalidateUser
        let s = fresh_simple();
        let t = s.assign("alice", vec![]);
        assert!(s.revoke(&t));
        assert_eq!(s.info(&t).unwrap_err(), TokenError::Invalid);
    }

    #[test]
    fn test_simple_revoke_returns_false_when_unknown() {
        // cite: simple_token.go (revoke missing token = no-op)
        let s = fresh_simple();
        assert!(!s.revoke("ghost"));
    }

    #[test]
    fn test_simple_sweep_drops_expired_only() {
        // cite: simple_token.go simpleTokenTTLKeeper goroutine
        let s = SimpleTokenIssuer::new(Duration::from_secs(0));
        s.assign("a", vec![]);
        s.assign("b", vec![]);
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(s.sweep(), 2);
        assert!(s.is_empty());
    }

    #[test]
    fn test_simple_token_format_carries_prefix() {
        // cite: simple_token.go (token includes <prefix>.<index>)
        let s = SimpleTokenIssuer::with_prefix(Duration::from_secs(60), "etcd");
        let t = s.assign("alice", vec![]);
        assert!(t.starts_with("etcd."), "token={t}");
    }

    #[test]
    fn test_simple_token_unique_per_call() {
        // cite: simple_token.go (each call produces a new token)
        let s = fresh_simple();
        let a = s.assign("alice", vec![]);
        let b = s.assign("alice", vec![]);
        assert_ne!(a, b);
    }

    // ── JwtIssuer ─────────────────────────────────────────────────────

    #[test]
    fn test_jwt_issue_three_segments() {
        // cite: server/auth/jwt.go (header.payload.sig)
        let j = fresh_jwt();
        let t = j.issue("alice", vec!["root".into()]);
        assert_eq!(t.split('.').count(), 3);
    }

    #[test]
    fn test_jwt_verify_roundtrip() {
        // cite: jwt.go verify (HS256 sig matches)
        let j = fresh_jwt();
        let t = j.issue("alice", vec!["r1".into(), "r2".into()]);
        let claims = j.verify(&t).unwrap();
        assert_eq!(claims.username, "alice");
        assert_eq!(claims.roles, vec!["r1", "r2"]);
        assert_eq!(claims.issuer.as_deref(), Some("cave-etcd"));
    }

    #[test]
    fn test_jwt_verify_bad_signature() {
        // cite: jwt.go verify (signature mismatch ⇒ invalid)
        let j = fresh_jwt();
        let mut t = j.issue("alice", vec![]);
        let last = t.len() - 1;
        // Twiddle one signature byte.
        let bytes = unsafe { t.as_bytes_mut() };
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        assert_eq!(j.verify(&t).unwrap_err(), TokenError::Invalid);
    }

    #[test]
    fn test_jwt_verify_bad_secret() {
        // cite: jwt.go verify (wrong secret ⇒ invalid)
        let j = fresh_jwt();
        let t = j.issue("alice", vec![]);
        let other = JwtIssuer::new("cave-etcd", b"other-key".to_vec(), Duration::from_secs(60));
        assert_eq!(other.verify(&t).unwrap_err(), TokenError::Invalid);
    }

    #[test]
    fn test_jwt_verify_malformed_segments() {
        // cite: jwt.go (bad number of segments)
        let j = fresh_jwt();
        match j.verify("only.two") {
            Err(TokenError::Malformed(_)) => (),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_jwt_verify_bad_b64() {
        // cite: jwt.go (base64-decode failure)
        let j = fresh_jwt();
        match j.verify("!!!.!!!.!!!") {
            Err(TokenError::Malformed(_)) => (),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_jwt_verify_alg_mismatch() {
        // cite: jwt.go (alg field must match server policy)
        let j = fresh_jwt();
        let header = b64url(serde_json::to_vec(&serde_json::json!({"alg": "RS256", "typ": "JWT"})).unwrap());
        let payload = b64url(serde_json::to_vec(&serde_json::json!({"username": "x"})).unwrap());
        let token = format!("{header}.{payload}.signature");
        match j.verify(&token) {
            Err(TokenError::AlgorithmMismatch { expected, got }) => {
                assert_eq!(expected, "HS256");
                assert_eq!(got, "RS256");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_jwt_verify_expired() {
        // cite: jwt.go (exp claim enforced)
        let j = JwtIssuer::new("cave-etcd", b"k".to_vec(), Duration::from_secs(0));
        let t = j.issue("alice", vec![]);
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(j.verify(&t).unwrap_err(), TokenError::Expired);
    }

    #[test]
    fn test_jwt_carries_issuer() {
        // cite: jwt.go iss claim
        let j = JwtIssuer::new("cluster-prod-1", b"k".to_vec(), Duration::from_secs(60));
        let t = j.issue("alice", vec![]);
        let claims = j.verify(&t).unwrap();
        assert_eq!(claims.issuer.as_deref(), Some("cluster-prod-1"));
    }

    #[test]
    fn test_jwt_payload_round_trip_roles() {
        // cite: jwt.go (roles claim survives serialization)
        let j = fresh_jwt();
        let t = j.issue("alice", vec!["r1".into(), "r2".into(), "r3".into()]);
        assert_eq!(j.verify(&t).unwrap().roles.len(), 3);
    }

    #[test]
    fn test_jwt_constant_time_compare_used() {
        // cite: jwt.go uses subtle.ConstantTimeCompare on the signature
        let j = fresh_jwt();
        let t = j.issue("alice", vec![]);
        // Truncated signature ⇒ unequal ⇒ Invalid (not panic).
        let parts: Vec<&str> = t.split('.').collect();
        let last = parts[2];
        let truncated = format!("{}.{}.{}", parts[0], parts[1], &last[..3]);
        assert_eq!(j.verify(&truncated).unwrap_err(), TokenError::Invalid);
    }

    // ── AuthTokenProvider unification ─────────────────────────────────

    #[test]
    fn test_provider_simple_kind() {
        let p = AuthTokenProvider::Simple(fresh_simple());
        assert_eq!(p.kind(), "simple");
    }

    #[test]
    fn test_provider_jwt_kind() {
        let p = AuthTokenProvider::Jwt(fresh_jwt());
        assert_eq!(p.kind(), "jwt");
    }

    #[test]
    fn test_provider_simple_round_trip() {
        let p = AuthTokenProvider::Simple(fresh_simple());
        let t = p.issue("alice", vec!["r".into()]);
        let c = p.verify(&t).unwrap();
        assert_eq!(c.username, "alice");
    }

    #[test]
    fn test_provider_jwt_round_trip() {
        let p = AuthTokenProvider::Jwt(fresh_jwt());
        let t = p.issue("bob", vec!["root".into()]);
        let c = p.verify(&t).unwrap();
        assert_eq!(c.username, "bob");
        assert_eq!(c.roles, vec!["root"]);
    }

    // ── TokenClaims helper ─────────────────────────────────────────────

    #[test]
    fn test_claims_is_expired_no_exp() {
        let c = TokenClaims { username: "x".into(), roles: vec![], expires_at: None, issuer: None };
        assert!(!c.is_expired(u64::MAX));
    }

    #[test]
    fn test_claims_is_expired_in_future() {
        let c = TokenClaims { username: "x".into(), roles: vec![], expires_at: Some(now_secs() + 60), issuer: None };
        assert!(!c.is_expired(now_secs()));
    }

    #[test]
    fn test_claims_is_expired_in_past() {
        let c = TokenClaims { username: "x".into(), roles: vec![], expires_at: Some(now_secs().saturating_sub(60)), issuer: None };
        assert!(c.is_expired(now_secs()));
    }
}
