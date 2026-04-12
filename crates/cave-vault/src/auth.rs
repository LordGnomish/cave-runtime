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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRole {
    pub role_id: String,
    pub secret_ids: HashMap<String, AppRoleSecretId>,
    pub policies: Vec<String>,
    pub token_ttl: u64,
    pub token_max_ttl: u64,
    pub secret_id_ttl: u64,
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
}
