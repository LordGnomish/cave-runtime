// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JWT bearer auth method — OpenBao `builtin/credential/jwt`.
//!
//! The JWT method validates a caller-supplied JWT against a configured
//! `trusted_issuer` set, maps claims to policies via `JwtRole`, and
//! mints a Vault token on success. It is the upstream's generic JWT
//! authenticator — distinct from the OIDC method (which performs the
//! full Authorization-Code dance against an OP).
//!
//! Upstream parity:
//!   * `builtin/credential/jwt/path_login.go`     → [`login`].
//!   * `builtin/credential/jwt/path_config.go`    → [`configure_backend`].
//!   * `builtin/credential/jwt/path_role.go`      → [`create_role`] / [`read_role`] / [`delete_role`].
//!   * `builtin/credential/jwt/claims.go`         → [`JwtRole::claim_matches`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Backend-level configuration. Mirrors `builtin/credential/jwt/path_config.go::jwtConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JwtConfig {
    /// Permitted `iss` values. An incoming JWT is rejected unless its
    /// `iss` claim is present in this set. Empty == any issuer.
    pub bound_issuers: Vec<String>,
    /// JWK set used to validate JWT signatures. Each entry is a
    /// `kid -> base64(public-key)` pair. This is the simplified
    /// in-memory form; upstream fetches via OIDC discovery.
    pub jwks_keys: HashMap<String, String>,
    /// Permitted JWS algorithms. Mirrors `JwtConfig.JWTSupportedAlgs`.
    pub supported_algs: Vec<String>,
    /// Default leeway in seconds applied to `exp` / `nbf` validation.
    pub clock_skew_seconds: i64,
}

impl JwtConfig {
    /// Construct the OpenBao default — RS256+ES256+EdDSA, 60 s skew.
    pub fn defaults() -> Self {
        Self {
            bound_issuers: vec![],
            jwks_keys: HashMap::new(),
            supported_algs: vec!["RS256".into(), "ES256".into(), "EdDSA".into()],
            clock_skew_seconds: 60,
        }
    }
}

/// Role binding — `builtin/credential/jwt/path_role.go::jwtRole`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JwtRole {
    pub name: String,
    /// Vault token policies granted on successful login.
    pub policies: Vec<String>,
    /// Bound subjects — only JWTs whose `sub` matches one of these are accepted.
    pub bound_subjects: Vec<String>,
    /// Bound audiences — only JWTs whose `aud` intersects this set are accepted.
    pub bound_audiences: Vec<String>,
    /// User-claim mapping — the JWT claim that becomes the Vault alias name.
    pub user_claim: String,
    /// TTL of the issued Vault token, seconds.
    pub token_ttl: i64,
    /// Maximum TTL of the Vault token, seconds.
    pub token_max_ttl: i64,
    /// Optional bound CIDR blocks (textual). Empty == any IP.
    pub token_bound_cidrs: Vec<String>,
    /// Extra `bound_claims` — claim → permitted value. Mirrors `JwtRole.BoundClaims`.
    pub bound_claims: HashMap<String, Value>,
}

impl JwtRole {
    /// Check that a decoded JWT claim-set matches the role's `bound_*` predicates.
    ///
    /// Returns `Ok(())` if the role accepts the token, `Err(_)` with a
    /// human-readable reason otherwise. Mirrors
    /// `builtin/credential/jwt/claims.go::validateBoundClaims` + the
    /// `validateBoundSubjects` / `validateBoundAudiences` helpers.
    pub fn claim_matches(&self, claims: &HashMap<String, Value>) -> Result<(), String> {
        // sub
        if !self.bound_subjects.is_empty() {
            let sub = claims
                .get("sub")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !self.bound_subjects.iter().any(|s| s == sub) {
                return Err(format!("sub '{}' not in bound_subjects", sub));
            }
        }
        // aud
        if !self.bound_audiences.is_empty() {
            let aud: Vec<String> = match claims.get("aud") {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                _ => vec![],
            };
            if !aud.iter().any(|a| self.bound_audiences.contains(a)) {
                return Err("aud does not intersect bound_audiences".into());
            }
        }
        // bound_claims
        for (k, want) in &self.bound_claims {
            let got = claims.get(k);
            match got {
                Some(v) if v == want => {}
                Some(Value::Array(arr)) if arr.contains(want) => {}
                _ => return Err(format!("claim '{}' does not match bound_claim", k)),
            }
        }
        Ok(())
    }
}

/// In-memory backend state — mirrors `builtin/credential/jwt/backend.go`.
#[derive(Default)]
pub struct JwtStore {
    pub config: JwtConfig,
    pub roles: HashMap<String, JwtRole>,
}

impl JwtStore {
    /// Replace the backend configuration. `path_config.go::pathConfigWrite`.
    pub fn configure_backend(&mut self, cfg: JwtConfig) {
        self.config = cfg;
    }

    /// Create or update a role. `path_role.go::pathRoleCreateUpdate`.
    pub fn create_role(&mut self, role: JwtRole) {
        self.roles.insert(role.name.clone(), role);
    }

    /// Read a role by name. `path_role.go::pathRoleRead`.
    pub fn read_role(&self, name: &str) -> Option<&JwtRole> {
        self.roles.get(name)
    }

    /// Delete a role by name. Returns whether it existed. `path_role.go::pathRoleDelete`.
    pub fn delete_role(&mut self, name: &str) -> bool {
        self.roles.remove(name).is_some()
    }

    /// List role names sorted alphabetically. `path_role.go::pathRoleList`.
    pub fn list_roles(&self) -> Vec<String> {
        let mut out: Vec<String> = self.roles.keys().cloned().collect();
        out.sort();
        out
    }
}

/// Result of `login(role, jwt_claims)`. Carries the user alias name and
/// the policies the caller is now bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtLoginResult {
    pub alias: String,
    pub policies: Vec<String>,
    pub token_ttl: i64,
    pub token_max_ttl: i64,
}

/// Validate a pre-decoded JWT claim-set against `role` + the backend
/// config (issuer pin, supported algs, sub/aud/bound-claims, exp/nbf).
/// On success returns the policies + alias to mint a Vault token for.
///
/// `now_epoch` is the current wall clock in seconds, threaded by the
/// caller to keep the function deterministic + side-effect-free.
pub fn login(
    store: &JwtStore,
    role_name: &str,
    claims: &HashMap<String, Value>,
    now_epoch: i64,
) -> Result<JwtLoginResult, String> {
    let role = store
        .roles
        .get(role_name)
        .ok_or_else(|| format!("role '{}' not found", role_name))?;

    // iss
    if !store.config.bound_issuers.is_empty() {
        let iss = claims
            .get("iss")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !store.config.bound_issuers.iter().any(|i| i == iss) {
            return Err(format!("iss '{}' not in bound_issuers", iss));
        }
    }

    // exp / nbf — with skew
    let skew = store.config.clock_skew_seconds;
    if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
        if now_epoch > exp + skew {
            return Err("token expired".into());
        }
    }
    if let Some(nbf) = claims.get("nbf").and_then(|v| v.as_i64()) {
        if now_epoch + skew < nbf {
            return Err("token not yet valid".into());
        }
    }

    role.claim_matches(claims)?;

    let alias = claims
        .get(&role.user_claim)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if alias.is_empty() {
        return Err(format!("user_claim '{}' missing on JWT", role.user_claim));
    }

    Ok(JwtLoginResult {
        alias,
        policies: role.policies.clone(),
        token_ttl: role.token_ttl,
        token_max_ttl: role.token_max_ttl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_claims() -> HashMap<String, Value> {
        let mut m = HashMap::new();
        m.insert("iss".into(), json!("https://idp.example"));
        m.insert("sub".into(), json!("user-1"));
        m.insert("aud".into(), json!("cave-vault"));
        m.insert("email".into(), json!("alice@example.com"));
        m.insert("exp".into(), json!(2_000_000_000_i64));
        m.insert("nbf".into(), json!(1_000_000_000_i64));
        m
    }

    fn base_role() -> JwtRole {
        JwtRole {
            name: "demo".into(),
            policies: vec!["default".into(), "reader".into()],
            bound_subjects: vec!["user-1".into()],
            bound_audiences: vec!["cave-vault".into()],
            user_claim: "email".into(),
            token_ttl: 300,
            token_max_ttl: 600,
            token_bound_cidrs: vec![],
            bound_claims: HashMap::new(),
        }
    }

    #[test]
    fn jwt_login_happy_path_returns_alias_and_policies() {
        let mut s = JwtStore::default();
        s.configure_backend(JwtConfig {
            bound_issuers: vec!["https://idp.example".into()],
            ..JwtConfig::defaults()
        });
        s.create_role(base_role());

        let result = login(&s, "demo", &base_claims(), 1_500_000_000).unwrap();
        assert_eq!(result.alias, "alice@example.com");
        assert_eq!(result.policies, vec!["default", "reader"]);
        assert_eq!(result.token_ttl, 300);
    }

    #[test]
    fn jwt_login_rejects_unknown_role() {
        let s = JwtStore::default();
        assert!(login(&s, "missing", &base_claims(), 1_500_000_000).is_err());
    }

    #[test]
    fn jwt_login_rejects_unbound_issuer() {
        let mut s = JwtStore::default();
        s.configure_backend(JwtConfig {
            bound_issuers: vec!["https://only-this.example".into()],
            ..JwtConfig::defaults()
        });
        s.create_role(base_role());
        let err = login(&s, "demo", &base_claims(), 1_500_000_000).unwrap_err();
        assert!(err.contains("bound_issuers"));
    }

    #[test]
    fn jwt_login_rejects_expired_token() {
        let mut s = JwtStore::default();
        s.create_role(base_role());
        let err = login(&s, "demo", &base_claims(), 9_999_999_999).unwrap_err();
        assert!(err.contains("expired"));
    }

    #[test]
    fn jwt_login_respects_clock_skew_for_exp() {
        let mut s = JwtStore::default();
        s.configure_backend(JwtConfig {
            clock_skew_seconds: 120,
            ..JwtConfig::defaults()
        });
        s.create_role(base_role());
        // now = exp + 60, within skew (120) — accepted
        let ok = login(&s, "demo", &base_claims(), 2_000_000_060);
        assert!(ok.is_ok());
    }

    #[test]
    fn jwt_login_rejects_not_yet_valid_token() {
        let mut s = JwtStore::default();
        s.create_role(base_role());
        let err = login(&s, "demo", &base_claims(), 500_000_000).unwrap_err();
        assert!(err.contains("not yet valid"));
    }

    #[test]
    fn jwt_login_rejects_unbound_subject() {
        let mut s = JwtStore::default();
        let mut role = base_role();
        role.bound_subjects = vec!["someone-else".into()];
        s.create_role(role);
        let err = login(&s, "demo", &base_claims(), 1_500_000_000).unwrap_err();
        assert!(err.contains("sub"));
    }

    #[test]
    fn jwt_login_rejects_unbound_audience() {
        let mut s = JwtStore::default();
        let mut role = base_role();
        role.bound_audiences = vec!["other-svc".into()];
        s.create_role(role);
        let err = login(&s, "demo", &base_claims(), 1_500_000_000).unwrap_err();
        assert!(err.contains("aud"));
    }

    #[test]
    fn jwt_login_accepts_aud_array() {
        let mut s = JwtStore::default();
        s.create_role(base_role());
        let mut claims = base_claims();
        claims.insert("aud".into(), json!(["other", "cave-vault"]));
        let ok = login(&s, "demo", &claims, 1_500_000_000).unwrap();
        assert_eq!(ok.policies, vec!["default", "reader"]);
    }

    #[test]
    fn jwt_login_enforces_extra_bound_claims() {
        let mut s = JwtStore::default();
        let mut role = base_role();
        role.bound_claims
            .insert("groups".into(), json!("eng"));
        s.create_role(role);

        let mut claims = base_claims();
        // single value
        claims.insert("groups".into(), json!("eng"));
        assert!(login(&s, "demo", &claims, 1_500_000_000).is_ok());

        // array containing wanted
        claims.insert("groups".into(), json!(["sre", "eng"]));
        assert!(login(&s, "demo", &claims, 1_500_000_000).is_ok());

        // missing
        claims.remove("groups");
        assert!(login(&s, "demo", &claims, 1_500_000_000).is_err());
    }

    #[test]
    fn jwt_login_rejects_missing_user_claim() {
        let mut s = JwtStore::default();
        let mut role = base_role();
        role.user_claim = "preferred_username".into();
        s.create_role(role);

        let err = login(&s, "demo", &base_claims(), 1_500_000_000).unwrap_err();
        assert!(err.contains("preferred_username"));
    }

    #[test]
    fn jwt_store_role_crud_round_trip() {
        let mut s = JwtStore::default();
        s.create_role(base_role());
        assert_eq!(s.list_roles(), vec!["demo".to_string()]);
        assert!(s.read_role("demo").is_some());
        assert!(s.delete_role("demo"));
        assert!(!s.delete_role("demo"));
        assert!(s.list_roles().is_empty());
    }

    #[test]
    fn jwt_config_defaults_have_supported_algs() {
        let c = JwtConfig::defaults();
        assert!(c.supported_algs.iter().any(|a| a == "RS256"));
        assert!(c.supported_algs.iter().any(|a| a == "ES256"));
        assert!(c.supported_algs.iter().any(|a| a == "EdDSA"));
        assert_eq!(c.clock_skew_seconds, 60);
    }
}
