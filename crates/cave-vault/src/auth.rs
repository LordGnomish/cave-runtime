<<<<<<< HEAD
//! Authentication methods — token, AppRole, Kubernetes, OIDC.
//!
//! All methods return a `AuthResult` containing a client token that can be
//! used for subsequent vault API calls.

use crate::models::LeaseInfo;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Invalid or expired token")]
    InvalidToken,
    #[error("Token has expired")]
    TokenExpired,
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Role not found: {0}")]
    RoleNotFound(String),
    #[error("Permission denied")]
    PermissionDenied,
}

/// In-memory token record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub token_id: String,
    pub policies: Vec<String>,
    pub lease: LeaseInfo,
    pub renewable: bool,
    pub display_name: String,
    pub metadata: HashMap<String, String>,
    pub created_at: chrono::DateTime<Utc>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

/// AppRole definition
=======
//! Auth methods — token, userpass, AppRole, OIDC (stub).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::VaultError;
use crate::models::{AuthResult, LeaseInfo};

// ── Token ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub token_id: String,
    pub accessor: String,
    pub display_name: String,
    pub policies: Vec<String>,
    pub meta: HashMap<String, String>,
    pub renewable: bool,
    pub orphan: bool,
    pub created_time: DateTime<Utc>,
    pub expire_time: Option<DateTime<Utc>>,
    pub ttl: u64,
}

impl TokenInfo {
    pub fn is_valid(&self) -> bool {
        match self.expire_time {
            None    => true,
            Some(t) => Utc::now() < t,
        }
    }
}

// ── Userpass ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserpassUser {
    pub username: String,
    /// bcrypt hash, but we store a salted SHA-256 hex string for simplicity
    /// (production: use argon2 or bcrypt).
    pub password_hash: String,
    pub policies: Vec<String>,
    pub token_ttl: u64,
}

// ── AppRole ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRoleSecretId {
    pub secret_id: String,
    pub accessor: String,
    pub meta: HashMap<String, String>,
    pub expire_time: Option<DateTime<Utc>>,
}

impl AppRoleSecretId {
    pub fn is_valid(&self) -> bool {
        match self.expire_time {
            None    => true,
            Some(t) => Utc::now() < t,
        }
    }
}

>>>>>>> claude/frosty-wiles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRole {
    pub role_id: String,
    pub secret_ids: HashMap<String, AppRoleSecretId>,
    pub policies: Vec<String>,
    pub token_ttl: u64,
    pub token_max_ttl: u64,
    pub secret_id_ttl: u64,
<<<<<<< HEAD
    /// When true, a valid secret_id must be presented at login
    pub bind_secret_id: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRoleSecretId {
    pub secret_id: String,
    pub secret_id_accessor: String,
    pub metadata: HashMap<String, String>,
    pub created_at: chrono::DateTime<Utc>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

/// Result returned to the caller on successful authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub client_token: String,
    pub lease_duration: u64,
    pub renewable: bool,
    pub policies: Vec<String>,
    pub token_type: String,
    pub metadata: HashMap<String, String>,
}

/// Validate an existing token and return its info.
pub fn token_auth(
    tokens: &HashMap<String, TokenInfo>,
    token: &str,
) -> Result<AuthResult, AuthError> {
    let info = tokens.get(token).ok_or(AuthError::InvalidToken)?;
    if let Some(exp) = info.expires_at {
        if Utc::now() > exp {
            return Err(AuthError::TokenExpired);
        }
    }
    Ok(AuthResult {
        client_token: info.token_id.clone(),
        lease_duration: info.lease.lease_duration,
        renewable: info.renewable,
        policies: info.policies.clone(),
        token_type: "service".to_string(),
        metadata: info.metadata.clone(),
    })
}

/// Authenticate via AppRole (role_id + secret_id).
pub fn approle_auth(
    approles: &HashMap<String, AppRole>,
    tokens: &mut HashMap<String, TokenInfo>,
    role_id: &str,
    secret_id: &str,
) -> Result<AuthResult, AuthError> {
    let role = approles
        .values()
        .find(|r| r.role_id == role_id)
        .ok_or(AuthError::InvalidCredentials)?;

    if role.bind_secret_id {
        let valid = role
            .secret_ids
            .values()
            .any(|s| {
                s.secret_id == secret_id
                    && s.expires_at.map(|e| Utc::now() < e).unwrap_or(true)
            });
        if !valid {
            return Err(AuthError::InvalidCredentials);
        }
    }

    let info = mint_token(tokens, &role.policies, role.token_ttl, "approle");
    Ok(AuthResult {
        client_token: info.token_id,
        lease_duration: role.token_ttl,
        renewable: true,
        policies: role.policies.clone(),
        token_type: "service".to_string(),
        metadata: HashMap::new(),
    })
}

/// Authenticate via Kubernetes service account JWT.
pub fn kubernetes_auth(
    tokens: &mut HashMap<String, TokenInfo>,
    jwt: &str,
    role: &str,
) -> Result<AuthResult, AuthError> {
    // Validate minimal JWT structure (header.payload.signature)
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(AuthError::InvalidCredentials);
    }
    let policies = vec!["default".to_string(), format!("k8s-{role}")];
    let info = mint_token(tokens, &policies, 3600, &format!("k8s/{role}"));
    Ok(AuthResult {
        client_token: info.token_id,
        lease_duration: 3600,
        renewable: true,
        policies,
        token_type: "service".to_string(),
        metadata: {
            let mut m = HashMap::new();
            m.insert("role".to_string(), role.to_string());
            m
        },
    })
}

/// Authenticate via OIDC auth code exchange.
pub fn oidc_auth(
    tokens: &mut HashMap<String, TokenInfo>,
    code: &str,
    role: &str,
) -> Result<AuthResult, AuthError> {
    // Production: exchange code with IdP; here we validate non-empty code
    if code.is_empty() {
        return Err(AuthError::InvalidCredentials);
    }
    let policies = vec!["default".to_string(), format!("oidc-{role}")];
    let info = mint_token(tokens, &policies, 3600, &format!("oidc/{role}"));
    Ok(AuthResult {
        client_token: info.token_id,
        lease_duration: 3600,
        renewable: true,
        policies,
        token_type: "service".to_string(),
        metadata: HashMap::new(),
    })
}

/// Generate a new token and insert it into the token store.
pub fn mint_token(
    tokens: &mut HashMap<String, TokenInfo>,
    policies: &[String],
    ttl_secs: u64,
    display_name: &str,
) -> TokenInfo {
    let now = Utc::now();
    let token_id = Uuid::new_v4().to_string();
    let expires_at = now + chrono::Duration::seconds(ttl_secs as i64);

    let info = TokenInfo {
        token_id: token_id.clone(),
        policies: policies.to_vec(),
        lease: LeaseInfo {
            lease_id: format!("auth/token/{token_id}"),
            renewable: true,
            lease_duration: ttl_secs,
            created_at: now,
            expires_at,
        },
        renewable: true,
        display_name: display_name.to_string(),
        metadata: HashMap::new(),
        created_at: now,
        expires_at: Some(expires_at),
    };
    tokens.insert(token_id, info.clone());
    info
=======
    pub bind_secret_id: bool,
}

// ── Auth engine ───────────────────────────────────────────────────────────────

pub struct AuthEngine {
    pub tokens:    HashMap<String, TokenInfo>,
    pub userpass:  HashMap<String, UserpassUser>,
    pub approles:  HashMap<String, AppRole>,
    root_token:    String,
}

impl AuthEngine {
    pub fn new() -> Self {
        // Create a root token
        let root_token = format!("root-{}", Uuid::new_v4());
        let root_info = TokenInfo {
            token_id: root_token.clone(),
            accessor: Uuid::new_v4().to_string(),
            display_name: "root".into(),
            policies: vec!["root".into()],
            meta: HashMap::new(),
            renewable: false,
            orphan: true,
            created_time: Utc::now(),
            expire_time: None,
            ttl: 0,
        };
        let mut tokens = HashMap::new();
        tokens.insert(root_token.clone(), root_info);
        Self {
            tokens,
            userpass: HashMap::new(),
            approles: HashMap::new(),
            root_token,
        }
    }

    pub fn root_token(&self) -> &str {
        &self.root_token
    }

    // ── Token auth ────────────────────────────────────────────────────────────

    pub fn lookup_token(&self, token: &str) -> Result<&TokenInfo, VaultError> {
        let info = self
            .tokens
            .get(token)
            .ok_or(VaultError::InvalidToken)?;
        if !info.is_valid() {
            return Err(VaultError::LeaseExpired);
        }
        Ok(info)
    }

    pub fn mint_token(
        &mut self,
        display_name: &str,
        policies: Vec<String>,
        ttl_secs: u64,
        renewable: bool,
        meta: HashMap<String, String>,
    ) -> TokenInfo {
        let token_id = format!("s.{}", Uuid::new_v4().to_string().replace('-', ""));
        let expire_time = if ttl_secs > 0 {
            Some(Utc::now() + chrono::Duration::seconds(ttl_secs as i64))
        } else {
            None
        };
        let info = TokenInfo {
            token_id: token_id.clone(),
            accessor: Uuid::new_v4().to_string(),
            display_name: display_name.to_string(),
            policies,
            meta,
            renewable,
            orphan: false,
            created_time: Utc::now(),
            expire_time,
            ttl: ttl_secs,
        };
        self.tokens.insert(token_id, info.clone());
        info
    }

    pub fn revoke_token(&mut self, token: &str) -> Result<(), VaultError> {
        self.tokens
            .remove(token)
            .ok_or(VaultError::InvalidToken)
            .map(|_| ())
    }

    pub fn renew_token(&mut self, token: &str, increment: u64) -> Result<&TokenInfo, VaultError> {
        let info = self.tokens.get_mut(token).ok_or(VaultError::InvalidToken)?;
        if !info.renewable {
            return Err(VaultError::InvalidRequest("token is not renewable".into()));
        }
        let secs = if increment > 0 { increment } else { info.ttl };
        info.expire_time = Some(Utc::now() + chrono::Duration::seconds(secs as i64));
        Ok(self.tokens.get(token).unwrap())
    }

    // ── Userpass auth ─────────────────────────────────────────────────────────

    pub fn userpass_create(
        &mut self,
        username: &str,
        password: &str,
        policies: Vec<String>,
        token_ttl: u64,
    ) {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        self.userpass.insert(
            username.to_string(),
            UserpassUser {
                username: username.to_string(),
                password_hash: hash,
                policies,
                token_ttl,
            },
        );
    }

    pub fn userpass_login(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<AuthResult, VaultError> {
        use sha2::{Digest, Sha256};
        let user = self
            .userpass
            .get(username)
            .ok_or_else(|| VaultError::PermissionDenied("invalid credentials".into()))?
            .clone();

        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        if hash != user.password_hash {
            return Err(VaultError::PermissionDenied("invalid credentials".into()));
        }

        let token = self.mint_token(
            &format!("userpass-{username}"),
            user.policies.clone(),
            user.token_ttl,
            true,
            {
                let mut m = HashMap::new();
                m.insert("username".into(), username.to_string());
                m
            },
        );

        Ok(AuthResult {
            client_token: token.token_id.clone(),
            accessor: token.accessor.clone(),
            policies: token.policies.clone(),
            lease_duration: user.token_ttl,
            renewable: true,
            token_type: "service".into(),
            metadata: token.meta.clone(),
        })
    }

    // ── AppRole auth ──────────────────────────────────────────────────────────

    pub fn approle_create(
        &mut self,
        role_name: &str,
        policies: Vec<String>,
        token_ttl: u64,
        token_max_ttl: u64,
        secret_id_ttl: u64,
        bind_secret_id: bool,
    ) -> String {
        let role_id = Uuid::new_v4().to_string();
        self.approles.insert(
            role_name.to_string(),
            AppRole {
                role_id: role_id.clone(),
                secret_ids: HashMap::new(),
                policies,
                token_ttl,
                token_max_ttl,
                secret_id_ttl,
                bind_secret_id,
            },
        );
        role_id
    }

    pub fn approle_generate_secret_id(
        &mut self,
        role_name: &str,
        meta: HashMap<String, String>,
    ) -> Result<String, VaultError> {
        let role = self
            .approles
            .get_mut(role_name)
            .ok_or_else(|| VaultError::NotFound(format!("approle '{role_name}'")))?;

        let secret_id = Uuid::new_v4().to_string();
        let expire_time = if role.secret_id_ttl > 0 {
            Some(Utc::now() + chrono::Duration::seconds(role.secret_id_ttl as i64))
        } else {
            None
        };
        role.secret_ids.insert(
            secret_id.clone(),
            AppRoleSecretId {
                secret_id: secret_id.clone(),
                accessor: Uuid::new_v4().to_string(),
                meta,
                expire_time,
            },
        );
        Ok(secret_id)
    }

    pub fn approle_login(
        &mut self,
        role_id: &str,
        secret_id: &str,
    ) -> Result<AuthResult, VaultError> {
        // Find role by role_id
        let role_name = self
            .approles
            .iter()
            .find(|(_, r)| r.role_id == role_id)
            .map(|(name, _)| name.clone())
            .ok_or_else(|| VaultError::PermissionDenied("invalid role_id".into()))?;

        let role = self.approles.get(&role_name).unwrap().clone();

        if role.bind_secret_id {
            let sid = role
                .secret_ids
                .get(secret_id)
                .ok_or_else(|| VaultError::PermissionDenied("invalid secret_id".into()))?;
            if !sid.is_valid() {
                return Err(VaultError::LeaseExpired);
            }
        }

        let token = self.mint_token(
            &format!("approle-{role_name}"),
            role.policies.clone(),
            role.token_ttl,
            true,
            {
                let mut m = HashMap::new();
                m.insert("role_name".into(), role_name.clone());
                m
            },
        );

        Ok(AuthResult {
            client_token: token.token_id.clone(),
            accessor: token.accessor.clone(),
            policies: token.policies.clone(),
            lease_duration: role.token_ttl,
            renewable: true,
            token_type: "service".into(),
            metadata: token.meta.clone(),
        })
    }

    // ── OIDC auth (stub) ──────────────────────────────────────────────────────

    /// Stub OIDC login. In production this validates the JWT against the OIDC provider.
    pub fn oidc_login(
        &mut self,
        _code: &str,
        _state: &str,
        policies: Vec<String>,
    ) -> Result<AuthResult, VaultError> {
        let token = self.mint_token("oidc-user", policies, 3600, true, HashMap::new());
        Ok(AuthResult {
            client_token: token.token_id.clone(),
            accessor: token.accessor.clone(),
            policies: token.policies.clone(),
            lease_duration: 3600,
            renewable: true,
            token_type: "service".into(),
            metadata: token.meta.clone(),
        })
    }

    pub fn prune_expired(&mut self) {
        self.tokens.retain(|_, t| t.is_valid());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_token_creation_validation() {
        let mut engine = AuthEngine::new();
        let token = engine.mint_token("test", vec!["default".into()], 3600, true, HashMap::new());
        let looked_up = engine.lookup_token(&token.token_id).unwrap();
        assert_eq!(looked_up.display_name, "test");
    }

    #[test]
    fn test_auth_root_token_valid() {
        let engine = AuthEngine::new();
        let root = engine.root_token().to_string();
        let info = engine.lookup_token(&root).unwrap();
        assert!(info.policies.contains(&"root".to_string()));
    }

    #[test]
    fn test_auth_userpass_login() {
        let mut engine = AuthEngine::new();
        engine.userpass_create("alice", "hunter2", vec!["default".into()], 3600);
        let result = engine.userpass_login("alice", "hunter2").unwrap();
        assert!(!result.client_token.is_empty());
    }

    #[test]
    fn test_auth_userpass_wrong_password() {
        let mut engine = AuthEngine::new();
        engine.userpass_create("bob", "correct", vec![], 3600);
        let result = engine.userpass_login("bob", "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn test_auth_approle_login() {
        let mut engine = AuthEngine::new();
        let role_id = engine.approle_create(
            "my-app",
            vec!["default".into()],
            3600,
            86400,
            600,
            true,
        );
        let secret_id = engine
            .approle_generate_secret_id("my-app", HashMap::new())
            .unwrap();
        let result = engine.approle_login(&role_id, &secret_id).unwrap();
        assert!(!result.client_token.is_empty());
    }

    #[test]
    fn test_auth_token_revoke() {
        let mut engine = AuthEngine::new();
        let token = engine.mint_token("tmp", vec![], 3600, false, HashMap::new());
        engine.revoke_token(&token.token_id).unwrap();
        assert!(engine.lookup_token(&token.token_id).is_err());
    }
>>>>>>> claude/frosty-wiles
}
