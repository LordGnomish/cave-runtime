// SPDX-License-Identifier: AGPL-3.0-or-later
//! Personal Access Tokens (PAT) — create, list, validate, revoke.
//!
//! PATs are long-lived tokens scoped to specific capabilities.
//! The token is shown only once at creation; only its hash is stored.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use rand::RngCore;
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

const PAT_PREFIX: &str = "cave_pat_";
const TOKEN_BYTES: usize = 32;

/// A Personal Access Token record (stored server-side; plain token shown only once).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalAccessToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    /// SHA256 hex digest of the raw token.
    pub token_hash: String,
    /// Scopes this PAT is authorized for, e.g., ["flags:read", "secrets:read"].
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

impl PersonalAccessToken {
    pub fn is_valid(&self) -> bool {
        if self.revoked {
            return false;
        }
        if let Some(exp) = self.expires_at {
            if Utc::now() > exp {
                return false;
            }
        }
        true
    }

    pub fn has_scope(&self, required: &str) -> bool {
        self.scopes.iter().any(|s| {
            s == required
                || s == "*"
                || s.ends_with(":*")
                    && required.starts_with(s.trim_end_matches('*').trim_end_matches(':'))
        })
    }
}

/// Request to create a PAT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePatRequest {
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Response includes the plain token — shown once, never stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePatResponse {
    pub pat: PersonalAccessToken,
    /// The actual token string (shown only at creation).
    pub token: String,
}

/// Hash a PAT raw token using SHA256.
fn hash_token(raw: &str) -> String {
    let h = digest(&SHA256, raw.as_bytes());
    hex::encode(h.as_ref())
}

/// Generate a new random PAT string.
fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{}{}", PAT_PREFIX, URL_SAFE_NO_PAD.encode(bytes))
}

/// PAT service — manages creation, validation, revocation.
#[derive(Clone)]
pub struct PatService {
    /// Keyed by PAT id.
    tokens: Arc<RwLock<HashMap<Uuid, PersonalAccessToken>>>,
}

impl PatService {
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new PAT for a user.
    pub async fn create(
        &self,
        user_id: Uuid,
        tenant_id: String,
        req: CreatePatRequest,
    ) -> CreatePatResponse {
        let raw_token = generate_token();
        let token_hash = hash_token(&raw_token);

        let pat = PersonalAccessToken {
            id: Uuid::new_v4(),
            user_id,
            tenant_id,
            name: req.name,
            token_hash,
            scopes: req.scopes,
            expires_at: req.expires_at,
            created_at: Utc::now(),
            last_used_at: None,
            revoked: false,
        };

        self.tokens.write().await.insert(pat.id, pat.clone());
        CreatePatResponse {
            pat,
            token: raw_token,
        }
    }

    /// List all PATs for a user in a tenant (excluding revoked).
    pub async fn list(&self, user_id: Uuid, tenant_id: &str) -> Vec<PersonalAccessToken> {
        self.tokens
            .read()
            .await
            .values()
            .filter(|p| p.user_id == user_id && p.tenant_id == tenant_id && !p.revoked)
            .cloned()
            .collect()
    }

    /// Revoke a PAT by ID (only the owner can revoke).
    pub async fn revoke(&self, id: Uuid, user_id: Uuid) -> Result<(), String> {
        let mut tokens = self.tokens.write().await;
        let pat = tokens
            .get_mut(&id)
            .ok_or_else(|| format!("PAT {id} not found"))?;
        if pat.user_id != user_id {
            return Err("Not authorized to revoke this PAT".to_string());
        }
        pat.revoked = true;
        Ok(())
    }

    /// Validate a raw PAT string — returns the PAT record if valid.
    pub async fn validate(&self, raw_token: &str, required_scope: Option<&str>) -> Result<PersonalAccessToken, String> {
        let token_hash = hash_token(raw_token);
        let mut tokens = self.tokens.write().await;

        let pat = tokens
            .values_mut()
            .find(|p| p.token_hash == token_hash)
            .ok_or("Invalid PAT")?;

        if !pat.is_valid() {
            return Err("PAT is revoked or expired".to_string());
        }

        if let Some(scope) = required_scope {
            if !pat.has_scope(scope) {
                return Err(format!("PAT lacks required scope: {scope}"));
            }
        }

        pat.last_used_at = Some(Utc::now());
        Ok(pat.clone())
    }
}

impl Default for PatService {
    fn default() -> Self {
        Self::new()
    }
}

// hex encoding helper (avoid adding hex crate - implement inline)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pat_create_returns_token() {
        let svc = PatService::new();
        let user_id = Uuid::new_v4();
        let resp = svc
            .create(
                user_id,
                "acme".to_string(),
                CreatePatRequest {
                    name: "CI token".to_string(),
                    scopes: vec!["flags:read".to_string()],
                    expires_at: None,
                },
            )
            .await;
        assert!(resp.token.starts_with("cave_pat_"));
        assert!(!resp.pat.revoked);
    }

    #[tokio::test]
    async fn pat_validate_succeeds() {
        let svc = PatService::new();
        let user_id = Uuid::new_v4();
        let resp = svc
            .create(
                user_id,
                "acme".to_string(),
                CreatePatRequest {
                    name: "test".to_string(),
                    scopes: vec!["secrets:read".to_string()],
                    expires_at: None,
                },
            )
            .await;

        let validated = svc.validate(&resp.token, Some("secrets:read")).await;
        assert!(validated.is_ok());
    }

    #[tokio::test]
    async fn pat_validate_wrong_token_fails() {
        let svc = PatService::new();
        let result = svc.validate("cave_pat_not_a_real_token", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pat_revoke_invalidates() {
        let svc = PatService::new();
        let user_id = Uuid::new_v4();
        let resp = svc
            .create(
                user_id,
                "acme".to_string(),
                CreatePatRequest {
                    name: "to-revoke".to_string(),
                    scopes: vec!["*".to_string()],
                    expires_at: None,
                },
            )
            .await;

        svc.revoke(resp.pat.id, user_id).await.unwrap();
        let result = svc.validate(&resp.token, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pat_scope_check() {
        let svc = PatService::new();
        let user_id = Uuid::new_v4();
        let resp = svc
            .create(
                user_id,
                "acme".to_string(),
                CreatePatRequest {
                    name: "scoped".to_string(),
                    scopes: vec!["flags:read".to_string()],
                    expires_at: None,
                },
            )
            .await;

        // Has flags:read
        assert!(svc.validate(&resp.token, Some("flags:read")).await.is_ok());
        // Doesn't have flags:write
        assert!(svc.validate(&resp.token, Some("flags:write")).await.is_err());
    }

    #[tokio::test]
    async fn pat_list_excludes_revoked() {
        let svc = PatService::new();
        let user_id = Uuid::new_v4();

        let r1 = svc
            .create(user_id, "acme".to_string(), CreatePatRequest {
                name: "active".to_string(),
                scopes: vec![],
                expires_at: None,
            })
            .await;
        let r2 = svc
            .create(user_id, "acme".to_string(), CreatePatRequest {
                name: "revoked".to_string(),
                scopes: vec![],
                expires_at: None,
            })
            .await;

        svc.revoke(r2.pat.id, user_id).await.unwrap();

        let list = svc.list(user_id, "acme").await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "active");
    }
}
