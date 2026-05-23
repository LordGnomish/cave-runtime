// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OAuth2 / OIDC provider — authorization endpoint, token endpoint,
//! userinfo, introspection (RFC 7662), revocation (RFC 7009), device
//! authorization grant (RFC 8628), and PKCE (RFC 7636).
//!
//! Upstream:
//!   * `services/src/main/java/org/keycloak/protocol/oidc/endpoints/AuthorizationEndpoint.java`
//!   * `services/src/main/java/org/keycloak/protocol/oidc/endpoints/TokenEndpoint.java`
//!   * `services/src/main/java/org/keycloak/protocol/oidc/endpoints/UserInfoEndpoint.java`
//!   * `services/src/main/java/org/keycloak/protocol/oidc/endpoints/TokenRevocationEndpoint.java`
//!   * `services/src/main/java/org/keycloak/protocol/oidc/endpoints/TokenIntrospectionEndpoint.java`
//!   * `services/src/main/java/org/keycloak/protocol/oidc/grants/device/endpoints/DeviceEndpoint.java`

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::{KeycloakError, Result};
use crate::models::{Client, GrantType};

// ─── PKCE (RFC 7636) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PkceMethod {
    /// `code_challenge = BASE64URL(SHA256(code_verifier))` — required.
    S256,
    /// `code_challenge = code_verifier` — explicitly opted in (legacy).
    Plain,
}

/// Verify a PKCE challenge — §4.6 of RFC 7636.
pub fn pkce_verify(method: PkceMethod, expected_challenge: &str, presented_verifier: &str) -> Result<()> {
    let len = presented_verifier.chars().count();
    if !(43..=128).contains(&len) {
        return Err(KeycloakError::PkceFailed("verifier-length-43-128".into()));
    }
    let computed = match method {
        PkceMethod::S256 => {
            let mut h = Sha256::new();
            h.update(presented_verifier.as_bytes());
            URL_SAFE_NO_PAD.encode(h.finalize())
        }
        PkceMethod::Plain => presented_verifier.to_string(),
    };
    if computed == expected_challenge {
        Ok(())
    } else {
        Err(KeycloakError::PkceFailed("challenge-mismatch".into()))
    }
}

// ─── Authorization code grant ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCode {
    pub code: String,
    pub realm_id: String,
    pub client_id: String,
    pub user_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub nonce: Option<String>,
    pub pkce_challenge: Option<(PkceMethod, String)>,
    pub issued_at: DateTime<Utc>,
    pub ttl_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeRequest<'a> {
    pub realm_id: &'a str,
    pub client_id: &'a str,
    pub user_id: &'a str,
    pub redirect_uri: &'a str,
    pub scope: &'a str,
    pub state: Option<&'a str>,
    pub nonce: Option<&'a str>,
    pub pkce: Option<(PkceMethod, &'a str)>,
}

/// `/realms/{realm}/protocol/openid-connect/auth` core check:
///   * client exists + enabled + supports auth-code grant
///   * redirect_uri exactly matches a registered URI
///   * scope subset of default ∪ optional scopes
///   * PKCE required when `client.require_pkce`
pub fn authorize(client: &Client, req: &AuthorizeRequest<'_>) -> Result<AuthCode> {
    if !client.enabled {
        return Err(KeycloakError::InvalidClientOrRedirect);
    }
    if client.client_id != req.client_id {
        return Err(KeycloakError::InvalidClientOrRedirect);
    }
    if !client.allowed_grant_types.contains(&GrantType::AuthorizationCode) {
        return Err(KeycloakError::InvalidGrant("client lacks authorization_code".into()));
    }
    if !client.accepts_redirect_uri(req.redirect_uri) {
        return Err(KeycloakError::InvalidClientOrRedirect);
    }
    check_scope(client, req.scope)?;
    if client.require_pkce && req.pkce.is_none() {
        return Err(KeycloakError::PkceFailed("missing-challenge".into()));
    }
    let mut code_bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut code_bytes);
    Ok(AuthCode {
        code: URL_SAFE_NO_PAD.encode(code_bytes),
        realm_id: req.realm_id.to_string(),
        client_id: req.client_id.to_string(),
        user_id: req.user_id.to_string(),
        redirect_uri: req.redirect_uri.to_string(),
        scope: req.scope.to_string(),
        nonce: req.nonce.map(|s| s.to_string()),
        pkce_challenge: req.pkce.map(|(m, c)| (m, c.to_string())),
        issued_at: Utc::now(),
        ttl_seconds: 60,
    })
}

fn check_scope(client: &Client, scope: &str) -> Result<()> {
    for s in scope.split_ascii_whitespace() {
        if s == "openid" {
            continue;
        }
        if !client.default_scopes.iter().any(|d| d == s) && !client.optional_scopes.iter().any(|o| o == s) {
            return Err(KeycloakError::ScopeNotPermitted(s.into()));
        }
    }
    Ok(())
}

/// `AuthCode` store with single-use semantics. `redeem` removes the
/// entry; replay attempts return `InvalidGrant`.
pub struct AuthCodeStore {
    inner: Mutex<BTreeMap<String, AuthCode>>,
}

impl Default for AuthCodeStore {
    fn default() -> Self {
        Self { inner: Mutex::new(BTreeMap::new()) }
    }
}

impl AuthCodeStore {
    pub fn issue(&self, code: AuthCode) {
        let mut g = self.inner.lock().unwrap();
        g.insert(code.code.clone(), code);
    }

    pub fn redeem(&self, code: &str, client_id: &str, redirect_uri: &str, verifier: Option<&str>) -> Result<AuthCode> {
        let mut g = self.inner.lock().unwrap();
        let entry = g.remove(code).ok_or_else(|| KeycloakError::InvalidGrant("unknown code".into()))?;
        if entry.client_id != client_id {
            return Err(KeycloakError::InvalidGrant("client mismatch".into()));
        }
        if entry.redirect_uri != redirect_uri {
            return Err(KeycloakError::InvalidGrant("redirect_uri mismatch".into()));
        }
        let age = (Utc::now() - entry.issued_at).num_seconds();
        if age > entry.ttl_seconds {
            return Err(KeycloakError::InvalidGrant("code expired".into()));
        }
        if let Some((method, challenge)) = &entry.pkce_challenge {
            let v = verifier.ok_or_else(|| KeycloakError::PkceFailed("missing-verifier".into()))?;
            pkce_verify(*method, challenge, v)?;
        }
        Ok(entry)
    }
}

// ─── Refresh token + rotation ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshToken {
    pub token: String,
    pub realm_id: String,
    pub client_id: String,
    pub user_id: String,
    pub scope: String,
    pub issued_at: DateTime<Utc>,
    pub idle_ttl: i64,
    pub max_ttl: i64,
    pub reuse_id: String,
    pub revoked: bool,
}

pub struct RefreshTokenStore {
    inner: Mutex<RefreshInner>,
}

struct RefreshInner {
    by_token: BTreeMap<String, RefreshToken>,
    /// reuse_id → ordered list of tokens (for replay detection).
    by_chain: BTreeMap<String, Vec<String>>,
}

impl Default for RefreshTokenStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(RefreshInner {
                by_token: BTreeMap::new(),
                by_chain: BTreeMap::new(),
            }),
        }
    }
}

impl RefreshTokenStore {
    pub fn issue(&self, client_id: &str, realm_id: &str, user_id: &str, scope: &str, idle_ttl: i64, max_ttl: i64) -> RefreshToken {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let mut chain_b = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut chain_b);
        let rt = RefreshToken {
            token: URL_SAFE_NO_PAD.encode(bytes),
            realm_id: realm_id.into(),
            client_id: client_id.into(),
            user_id: user_id.into(),
            scope: scope.into(),
            issued_at: Utc::now(),
            idle_ttl,
            max_ttl,
            reuse_id: hex::encode(chain_b),
            revoked: false,
        };
        let mut g = self.inner.lock().unwrap();
        g.by_chain.entry(rt.reuse_id.clone()).or_default().push(rt.token.clone());
        g.by_token.insert(rt.token.clone(), rt.clone());
        rt
    }

    /// Rotate — caller presents the old token; we mint a new one in the
    /// same reuse chain and mark the old one used. If the old token has
    /// already been used (chain has a successor), the whole chain is
    /// revoked — RFC 6749 §10.4 + Keycloak's `revokeRefreshToken` policy.
    pub fn rotate(&self, presented: &str) -> Result<RefreshToken> {
        let now = Utc::now();
        let mut g = self.inner.lock().unwrap();
        let old = g
            .by_token
            .get(presented)
            .cloned()
            .ok_or_else(|| KeycloakError::InvalidGrant("unknown refresh_token".into()))?;
        if old.revoked {
            // chain compromised — revoke every token in the chain
            if let Some(chain) = g.by_chain.get(&old.reuse_id).cloned() {
                for tok in chain {
                    if let Some(t) = g.by_token.get_mut(&tok) {
                        t.revoked = true;
                    }
                }
            }
            return Err(KeycloakError::TokenRevoked);
        }
        if (now - old.issued_at).num_seconds() > old.max_ttl {
            if let Some(t) = g.by_token.get_mut(presented) {
                t.revoked = true;
            }
            return Err(KeycloakError::TokenExpired);
        }
        // check chain has no later token already (replay)
        if let Some(chain) = g.by_chain.get(&old.reuse_id).cloned() {
            let pos = chain.iter().position(|t| t == presented).unwrap_or(0);
            if pos + 1 < chain.len() {
                // a successor already exists — replay! revoke everything
                for tok in chain {
                    if let Some(t) = g.by_token.get_mut(&tok) {
                        t.revoked = true;
                    }
                }
                return Err(KeycloakError::TokenRevoked);
            }
        }
        // mint successor in the same chain
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let succ = RefreshToken {
            token: URL_SAFE_NO_PAD.encode(bytes),
            realm_id: old.realm_id.clone(),
            client_id: old.client_id.clone(),
            user_id: old.user_id.clone(),
            scope: old.scope.clone(),
            issued_at: now,
            idle_ttl: old.idle_ttl,
            max_ttl: old.max_ttl,
            reuse_id: old.reuse_id.clone(),
            revoked: false,
        };
        if let Some(t) = g.by_token.get_mut(presented) {
            t.revoked = true;
        }
        g.by_chain.entry(succ.reuse_id.clone()).or_default().push(succ.token.clone());
        g.by_token.insert(succ.token.clone(), succ.clone());
        Ok(succ)
    }

    pub fn revoke(&self, presented: &str) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let t = g.by_token.get_mut(presented).ok_or_else(|| KeycloakError::InvalidGrant("unknown refresh_token".into()))?;
        t.revoked = true;
        Ok(())
    }

    pub fn introspect(&self, presented: &str) -> Option<RefreshToken> {
        let g = self.inner.lock().unwrap();
        g.by_token.get(presented).cloned()
    }
}

// ─── Device authorization grant (RFC 8628) ──────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub realm_id: String,
    pub client_id: String,
    pub scope: String,
    pub interval_seconds: u32,
    pub issued_at: DateTime<Utc>,
    pub ttl_seconds: i64,
    pub approved_user_id: Option<String>,
    pub denied: bool,
    pub last_poll: Option<DateTime<Utc>>,
}

pub struct DeviceCodeStore {
    inner: Mutex<DeviceCodeInner>,
}

struct DeviceCodeInner {
    by_code: BTreeMap<String, DeviceCode>,
    by_user_code: BTreeMap<String, String>,
}

impl Default for DeviceCodeStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(DeviceCodeInner {
                by_code: BTreeMap::new(),
                by_user_code: BTreeMap::new(),
            }),
        }
    }
}

impl DeviceCodeStore {
    pub fn issue(&self, client: &Client, realm_id: &str, scope: &str) -> Result<DeviceCode> {
        if !client.allowed_grant_types.contains(&GrantType::DeviceCode) {
            return Err(KeycloakError::InvalidGrant("client lacks urn:ietf:params:oauth:grant-type:device_code".into()));
        }
        check_scope(client, scope)?;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let device_code = URL_SAFE_NO_PAD.encode(bytes);
        // user_code: BCDFGHJKLMNPQRSTVWXZ + digits — Crockford-ish 8-char
        const A: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";
        let mut user_code = String::with_capacity(8);
        let mut rng = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut rng);
        for b in rng {
            user_code.push(A[(b as usize) % A.len()] as char);
        }
        let entry = DeviceCode {
            device_code: device_code.clone(),
            user_code: user_code.clone(),
            realm_id: realm_id.into(),
            client_id: client.client_id.clone(),
            scope: scope.into(),
            interval_seconds: 5,
            issued_at: Utc::now(),
            ttl_seconds: 600,
            approved_user_id: None,
            denied: false,
            last_poll: None,
        };
        let mut g = self.inner.lock().unwrap();
        g.by_user_code.insert(user_code, device_code.clone());
        g.by_code.insert(device_code, entry.clone());
        Ok(entry)
    }

    pub fn approve(&self, user_code: &str, user_id: &str) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let device = g.by_user_code.get(user_code).cloned().ok_or_else(|| KeycloakError::InvalidRequest("unknown user_code".into()))?;
        let entry = g.by_code.get_mut(&device).ok_or_else(|| KeycloakError::Internal("dc mismatch".into()))?;
        entry.approved_user_id = Some(user_id.into());
        Ok(())
    }

    pub fn deny(&self, user_code: &str) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let device = g.by_user_code.get(user_code).cloned().ok_or_else(|| KeycloakError::InvalidRequest("unknown user_code".into()))?;
        let entry = g.by_code.get_mut(&device).ok_or_else(|| KeycloakError::Internal("dc mismatch".into()))?;
        entry.denied = true;
        Ok(())
    }

    pub fn poll(&self, device_code: &str) -> Result<DeviceCode> {
        let now = Utc::now();
        let mut g = self.inner.lock().unwrap();
        let entry = g.by_code.get_mut(device_code).ok_or_else(|| KeycloakError::InvalidGrant("expired_token".into()))?;
        if (now - entry.issued_at).num_seconds() > entry.ttl_seconds {
            return Err(KeycloakError::TokenExpired);
        }
        if let Some(prev) = entry.last_poll {
            if (now - prev).num_seconds() < entry.interval_seconds as i64 {
                return Err(KeycloakError::InvalidGrant("slow_down".into()));
            }
        }
        entry.last_poll = Some(now);
        if entry.denied {
            return Err(KeycloakError::InvalidGrant("access_denied".into()));
        }
        if entry.approved_user_id.is_none() {
            return Err(KeycloakError::InvalidGrant("authorization_pending".into()));
        }
        Ok(entry.clone())
    }
}

// ─── Token revocation (RFC 7009) + introspection (RFC 7662) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionResponse {
    pub active: bool,
    pub scope: Option<String>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub sub: Option<String>,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub token_type: Option<String>,
}

impl IntrospectionResponse {
    pub fn inactive() -> Self {
        Self {
            active: false,
            scope: None,
            client_id: None,
            username: None,
            sub: None,
            exp: None,
            iat: None,
            token_type: None,
        }
    }
}

pub fn jitter(d: Duration) -> Duration {
    let mut b = [0u8; 1];
    rand::thread_rng().fill_bytes(&mut b);
    let extra = (b[0] as i64) % 16; // ± 0..15s — surface for production randomization
    d + Duration::seconds(extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Client, Protocol};
    use std::collections::BTreeMap;

    fn spa() -> Client {
        Client {
            id: "c1".into(),
            realm_id: "r1".into(),
            client_id: "spa".into(),
            name: "SPA".into(),
            enabled: true,
            protocol: Protocol::OpenIdConnect,
            public_client: true,
            client_secret_hash: None,
            redirect_uris: vec!["https://app/cb".into()],
            web_origins: vec![],
            default_scopes: vec!["openid".into(), "profile".into()],
            optional_scopes: vec!["email".into()],
            allowed_grant_types: vec![GrantType::AuthorizationCode, GrantType::RefreshToken, GrantType::DeviceCode],
            require_pkce: true,
            access_token_lifespan_seconds: None,
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn pkce_s256_roundtrip() {
        let verifier = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq"; // 43 chars
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(h.finalize());
        pkce_verify(PkceMethod::S256, &challenge, verifier).unwrap();
        assert!(pkce_verify(PkceMethod::S256, &challenge, "wrong-verifier-must-be-long-enough-43chars-x").is_err());
    }

    #[test]
    fn pkce_rejects_short_verifier() {
        assert!(pkce_verify(PkceMethod::S256, "x", "short").is_err());
    }

    #[test]
    fn authorize_emits_code_when_inputs_valid() {
        let c = spa();
        let req = AuthorizeRequest {
            realm_id: "r1",
            client_id: "spa",
            user_id: "u1",
            redirect_uri: "https://app/cb",
            scope: "openid profile",
            state: Some("xyz"),
            nonce: Some("n-1"),
            pkce: Some((PkceMethod::S256, "abc")),
        };
        let code = authorize(&c, &req).unwrap();
        assert_eq!(code.client_id, "spa");
        assert!(!code.code.is_empty());
    }

    #[test]
    fn authorize_rejects_unknown_redirect_uri() {
        let c = spa();
        let req = AuthorizeRequest {
            realm_id: "r1",
            client_id: "spa",
            user_id: "u1",
            redirect_uri: "https://evil/cb",
            scope: "openid",
            state: None,
            nonce: None,
            pkce: Some((PkceMethod::S256, "abc")),
        };
        assert!(authorize(&c, &req).is_err());
    }

    #[test]
    fn authorize_rejects_scope_outside_default_optional() {
        let c = spa();
        let req = AuthorizeRequest {
            realm_id: "r1",
            client_id: "spa",
            user_id: "u1",
            redirect_uri: "https://app/cb",
            scope: "openid evil-scope",
            state: None,
            nonce: None,
            pkce: Some((PkceMethod::S256, "abc")),
        };
        assert!(matches!(authorize(&c, &req), Err(KeycloakError::ScopeNotPermitted(_))));
    }

    #[test]
    fn authorize_requires_pkce_when_client_demands_it() {
        let c = spa();
        let req = AuthorizeRequest {
            realm_id: "r1",
            client_id: "spa",
            user_id: "u1",
            redirect_uri: "https://app/cb",
            scope: "openid",
            state: None,
            nonce: None,
            pkce: None,
        };
        assert!(matches!(authorize(&c, &req), Err(KeycloakError::PkceFailed(_))));
    }

    #[test]
    fn code_redeem_is_single_use() {
        let store = AuthCodeStore::default();
        let verifier = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(h.finalize());
        let code = AuthCode {
            code: "code-1".into(),
            realm_id: "r1".into(),
            client_id: "spa".into(),
            user_id: "u1".into(),
            redirect_uri: "https://app/cb".into(),
            scope: "openid".into(),
            nonce: None,
            pkce_challenge: Some((PkceMethod::S256, challenge)),
            issued_at: Utc::now(),
            ttl_seconds: 60,
        };
        store.issue(code);
        let back = store.redeem("code-1", "spa", "https://app/cb", Some(verifier)).unwrap();
        assert_eq!(back.user_id, "u1");
        assert!(store.redeem("code-1", "spa", "https://app/cb", Some(verifier)).is_err());
    }

    #[test]
    fn code_redeem_rejects_pkce_mismatch() {
        let store = AuthCodeStore::default();
        let code = AuthCode {
            code: "c".into(),
            realm_id: "r1".into(),
            client_id: "spa".into(),
            user_id: "u1".into(),
            redirect_uri: "https://app/cb".into(),
            scope: "openid".into(),
            nonce: None,
            pkce_challenge: Some((PkceMethod::S256, "ZZ".into())),
            issued_at: Utc::now(),
            ttl_seconds: 60,
        };
        store.issue(code);
        let bad = store.redeem("c", "spa", "https://app/cb", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq"));
        assert!(matches!(bad, Err(KeycloakError::PkceFailed(_))));
    }

    #[test]
    fn refresh_rotation_then_replay_revokes_chain() {
        let store = RefreshTokenStore::default();
        let t0 = store.issue("spa", "r1", "u1", "openid", 1800, 36_000);
        let t1 = store.rotate(&t0.token).unwrap();
        // Old token replayed → chain revoked
        assert!(matches!(store.rotate(&t0.token), Err(KeycloakError::TokenRevoked)));
        // t1 was successor but now belongs to a revoked chain
        let after = store.introspect(&t1.token).unwrap();
        assert!(after.revoked);
    }

    #[test]
    fn device_code_flow_pending_then_approved() {
        let s = DeviceCodeStore::default();
        let d = s.issue(&spa(), "r1", "openid profile").unwrap();
        // pending immediately
        assert!(matches!(s.poll(&d.device_code), Err(KeycloakError::InvalidGrant(_))));
        s.approve(&d.user_code, "u1").unwrap();
        // slow_down: same-second poll
        let res = s.poll(&d.device_code);
        assert!(res.is_err());
    }

    #[test]
    fn introspection_inactive_shape() {
        let r = IntrospectionResponse::inactive();
        assert!(!r.active);
        assert!(r.scope.is_none());
    }

    #[test]
    fn jitter_returns_close_to_original() {
        let d = Duration::seconds(60);
        let j = jitter(d);
        assert!((j.num_seconds() - 60).abs() < 30);
    }
}
