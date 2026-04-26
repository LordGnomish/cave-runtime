//! API key management — create, validate, revoke consumer keys.

use crate::error::{GatewayError, GatewayResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub key: String,
    pub name: String,
    pub consumer: String,
    pub scopes: Vec<Scope>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    ChatCompletions,
    ModelsList,
    Admin,
    All,
}

impl Scope {
    pub fn allows(&self, required: &Scope) -> bool {
        matches!(self, Scope::All) || self == required
    }
}

impl ApiKey {
    pub fn is_valid(&self) -> bool {
        if self.revoked {
            return false;
        }
        if let Some(expires_at) = self.expires_at {
            if chrono::Utc::now().timestamp() > expires_at {
                return false;
            }
        }
        true
    }

    pub fn has_scope(&self, required: &Scope) -> bool {
        self.scopes.iter().any(|s| s.allows(required))
    }
}

pub struct ApiKeyStore {
    // key string → ApiKey
    by_key: DashMap<String, ApiKey>,
    // id → key string (for lookup by id)
    by_id: DashMap<String, String>,
}

impl ApiKeyStore {
    pub fn new() -> Self {
        Self { by_key: DashMap::new(), by_id: DashMap::new() }
    }

    /// Create a new API key. Returns the created key.
    pub fn create(&self, name: &str, consumer: &str, scopes: Vec<Scope>, ttl_days: Option<u32>) -> ApiKey {
        let id = Uuid::new_v4().to_string();
        let key = format!("gw-{}", Uuid::new_v4().to_string().replace('-', ""));
        let now = chrono::Utc::now().timestamp();
        let expires_at = ttl_days.map(|days| now + (days as i64 * 86400));

        let api_key = ApiKey {
            id: id.clone(),
            key: key.clone(),
            name: name.to_string(),
            consumer: consumer.to_string(),
            scopes,
            created_at: now,
            expires_at,
            last_used_at: None,
            revoked: false,
        };

        self.by_key.insert(key.clone(), api_key.clone());
        self.by_id.insert(id, key);

        api_key
    }

    /// Validate an API key string and check required scope.
    pub fn validate(&self, key_str: &str, required_scope: &Scope) -> GatewayResult<ApiKey> {
        let mut entry = self.by_key.get_mut(key_str)
            .ok_or_else(|| GatewayError::Unauthorized("invalid API key".into()))?;

        if !entry.is_valid() {
            return Err(GatewayError::Unauthorized("API key expired or revoked".into()));
        }
        if !entry.has_scope(required_scope) {
            return Err(GatewayError::Unauthorized("insufficient scope".into()));
        }

        entry.last_used_at = Some(chrono::Utc::now().timestamp());
        Ok(entry.clone())
    }

    pub fn get_by_id(&self, id: &str) -> Option<ApiKey> {
        let key_str = self.by_id.get(id)?;
        self.by_key.get(key_str.as_str()).map(|e| e.clone())
    }

    pub fn revoke(&self, id: &str) -> GatewayResult<()> {
        let key_str = self.by_id.get(id)
            .ok_or_else(|| GatewayError::NotFound(format!("API key {id}")))?
            .clone();

        let mut entry = self.by_key.get_mut(key_str.as_str())
            .ok_or_else(|| GatewayError::NotFound(format!("API key {id}")))?;
        entry.revoked = true;
        Ok(())
    }

    pub fn list(&self) -> Vec<ApiKey> {
        self.by_key.iter().map(|e| {
            // Don't expose the raw key in list responses — mask it
            let mut k = e.value().clone();
            let masked_len = k.key.len().saturating_sub(4);
            k.key = format!("{}...{}", &k.key[..4], &k.key[masked_len..]);
            k
        }).collect()
    }

    pub fn list_for_consumer(&self, consumer: &str) -> Vec<ApiKey> {
        self.by_key.iter()
            .filter(|e| e.value().consumer == consumer)
            .map(|e| e.value().clone())
            .collect()
    }
}

impl Default for ApiKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_validate_key() {
        let store = ApiKeyStore::new();
        let key = store.create("test-key", "alice", vec![Scope::ChatCompletions], None);
        assert!(store.validate(&key.key, &Scope::ChatCompletions).is_ok());
    }

    #[test]
    fn wrong_scope_rejected() {
        let store = ApiKeyStore::new();
        let key = store.create("limited-key", "bob", vec![Scope::ModelsList], None);
        let result = store.validate(&key.key, &Scope::ChatCompletions);
        assert!(result.is_err());
    }

    #[test]
    fn all_scope_allows_any() {
        let store = ApiKeyStore::new();
        let key = store.create("admin-key", "admin", vec![Scope::All], None);
        assert!(store.validate(&key.key, &Scope::ChatCompletions).is_ok());
        assert!(store.validate(&key.key, &Scope::Admin).is_ok());
    }

    #[test]
    fn revoked_key_rejected() {
        let store = ApiKeyStore::new();
        let key = store.create("revoke-me", "dave", vec![Scope::All], None);
        store.revoke(&key.id).unwrap();
        let result = store.validate(&key.key, &Scope::ChatCompletions);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_key_rejected() {
        let store = ApiKeyStore::new();
        let result = store.validate("gw-notakey", &Scope::ChatCompletions);
        assert!(result.is_err());
    }
}
