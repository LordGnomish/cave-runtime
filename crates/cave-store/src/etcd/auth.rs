//! etcd Auth API — user/role management and authentication.

use crate::error::{StoreError, StoreResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Permission {
    Read,
    Write,
    Readwrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionEntry {
    pub perm_type: Permission,
    pub key: Vec<u8>,
    pub range_end: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    pub key_permission: Vec<PermissionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub name: String,
    pub password_hash: String, // argon2/bcrypt hash in prod; SHA256 here
    pub roles: Vec<String>,
}

pub struct AuthStore {
    pub enabled: bool,
    pub users: HashMap<String, User>,
    pub roles: HashMap<String, Role>,
}

impl Default for AuthStore {
    fn default() -> Self {
        let mut roles = HashMap::new();
        // Root role has full access
        roles.insert(
            "root".to_string(),
            Role {
                name: "root".to_string(),
                key_permission: vec![PermissionEntry {
                    perm_type: Permission::Readwrite,
                    key: b"\x00".to_vec(),
                    range_end: b"\x00".to_vec(),
                }],
            },
        );
        Self {
            enabled: false,
            users: HashMap::new(),
            roles,
        }
    }
}

pub struct AuthManager {
    pub store: RwLock<AuthStore>,
}

impl Default for AuthManager {
    fn default() -> Self {
        Self {
            store: RwLock::new(AuthStore::default()),
        }
    }
}

impl AuthManager {
    pub async fn enable(&self) -> StoreResult<()> {
        let mut store = self.store.write().await;
        if store.enabled {
            return Err(StoreError::AuthAlreadyEnabled);
        }
        store.enabled = true;
        Ok(())
    }

    pub async fn disable(&self) -> StoreResult<()> {
        let mut store = self.store.write().await;
        if !store.enabled {
            return Err(StoreError::AuthNotEnabled);
        }
        store.enabled = false;
        Ok(())
    }

    pub async fn is_enabled(&self) -> bool {
        self.store.read().await.enabled
    }

    pub async fn user_add(&self, name: String, password: String) -> StoreResult<()> {
        let mut store = self.store.write().await;
        if store.users.contains_key(&name) {
            return Err(StoreError::UserAlreadyExists(name));
        }
        let hash = hash_password(&password);
        store.users.insert(
            name.clone(),
            User {
                name,
                password_hash: hash,
                roles: Vec::new(),
            },
        );
        Ok(())
    }

    pub async fn user_delete(&self, name: &str) -> StoreResult<()> {
        let mut store = self.store.write().await;
        store
            .users
            .remove(name)
            .ok_or_else(|| StoreError::UserNotFound(name.to_string()))?;
        Ok(())
    }

    pub async fn user_get(&self, name: &str) -> StoreResult<User> {
        let store = self.store.read().await;
        store
            .users
            .get(name)
            .cloned()
            .ok_or_else(|| StoreError::UserNotFound(name.to_string()))
    }

    pub async fn user_list(&self) -> Vec<String> {
        self.store.read().await.users.keys().cloned().collect()
    }

    pub async fn user_change_password(&self, name: &str, password: String) -> StoreResult<()> {
        let mut store = self.store.write().await;
        let user = store
            .users
            .get_mut(name)
            .ok_or_else(|| StoreError::UserNotFound(name.to_string()))?;
        user.password_hash = hash_password(&password);
        Ok(())
    }

    pub async fn user_grant_role(&self, user: &str, role: &str) -> StoreResult<()> {
        let mut store = self.store.write().await;
        if !store.roles.contains_key(role) {
            return Err(StoreError::RoleNotFound(role.to_string()));
        }
        let u = store
            .users
            .get_mut(user)
            .ok_or_else(|| StoreError::UserNotFound(user.to_string()))?;
        if !u.roles.contains(&role.to_string()) {
            u.roles.push(role.to_string());
        }
        Ok(())
    }

    pub async fn user_revoke_role(&self, user: &str, role: &str) -> StoreResult<()> {
        let mut store = self.store.write().await;
        let u = store
            .users
            .get_mut(user)
            .ok_or_else(|| StoreError::UserNotFound(user.to_string()))?;
        u.roles.retain(|r| r != role);
        Ok(())
    }

    pub async fn role_add(&self, name: String) -> StoreResult<()> {
        let mut store = self.store.write().await;
        if store.roles.contains_key(&name) {
            return Err(StoreError::RoleAlreadyExists(name));
        }
        store.roles.insert(
            name.clone(),
            Role {
                name,
                key_permission: Vec::new(),
            },
        );
        Ok(())
    }

    pub async fn role_delete(&self, name: &str) -> StoreResult<()> {
        let mut store = self.store.write().await;
        store
            .roles
            .remove(name)
            .ok_or_else(|| StoreError::RoleNotFound(name.to_string()))?;
        Ok(())
    }

    pub async fn role_get(&self, name: &str) -> StoreResult<Role> {
        let store = self.store.read().await;
        store
            .roles
            .get(name)
            .cloned()
            .ok_or_else(|| StoreError::RoleNotFound(name.to_string()))
    }

    pub async fn role_list(&self) -> Vec<String> {
        self.store.read().await.roles.keys().cloned().collect()
    }

    pub async fn role_grant_permission(
        &self,
        role: &str,
        perm: PermissionEntry,
    ) -> StoreResult<()> {
        let mut store = self.store.write().await;
        let r = store
            .roles
            .get_mut(role)
            .ok_or_else(|| StoreError::RoleNotFound(role.to_string()))?;
        r.key_permission.push(perm);
        Ok(())
    }

    pub async fn role_revoke_permission(
        &self,
        role: &str,
        key: &[u8],
        range_end: &[u8],
    ) -> StoreResult<()> {
        let mut store = self.store.write().await;
        let r = store
            .roles
            .get_mut(role)
            .ok_or_else(|| StoreError::RoleNotFound(role.to_string()))?;
        r.key_permission
            .retain(|p| p.key != key || p.range_end != range_end);
        Ok(())
    }

    pub async fn authenticate(&self, name: &str, password: &str) -> StoreResult<String> {
        let store = self.store.read().await;
        if !store.enabled {
            // Auth disabled: anyone can access
            return Ok("noauth".to_string());
        }
        let user = store
            .users
            .get(name)
            .ok_or_else(|| StoreError::UserNotFound(name.to_string()))?;
        if user.password_hash != hash_password(password) {
            return Err(StoreError::PermissionDenied);
        }
        // Return a simple token (in prod, use JWT)
        Ok(format!("token-{name}-{}", chrono::Utc::now().timestamp()))
    }
}

fn hash_password(password: &str) -> String {
    use ring::digest;
    let digest = digest::digest(&digest::SHA256, password.as_bytes());
    hex::encode(digest.as_ref())
}
