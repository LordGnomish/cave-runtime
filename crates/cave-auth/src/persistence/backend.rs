// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 model/jpa/src/main/java/org/keycloak/models/jpa/JpaRealmProvider.java
//
// Port of Keycloak's `*Provider` interfaces (RealmProvider, UserProvider,
// ClientProvider, RoleProvider, GroupProvider, IdentityProviderProvider,
// AuthenticationManagementProvider) collapsed into a single async trait.

//! `PersistenceBackend` — async CRUD over seven Keycloak entities.

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use super::entities::{
    AuthFlowEntity, ClientEntity, GroupEntity, IdentityProviderEntity, RealmEntity, RoleEntity,
    UserEntity,
};
use super::txn::Transaction;

/// Backend-level error surface — covers SQL, in-memory snapshot, txn,
/// and "not found" / "conflict" semantics so the trait is exhaustive.
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("entity not found: {entity}/{id}")]
    NotFound { entity: &'static str, id: String },

    #[error("conflict (unique violation) on {entity}: {detail}")]
    Conflict {
        entity: &'static str,
        detail: String,
    },

    #[error("backend i/o: {0}")]
    Backend(String),

    #[error("transaction error: {0}")]
    Transaction(String),

    #[error("schema migration error: {0}")]
    Migration(String),
}

impl PersistenceError {
    pub fn not_found(entity: &'static str, id: impl ToString) -> Self {
        Self::NotFound {
            entity,
            id: id.to_string(),
        }
    }
    pub fn conflict(entity: &'static str, detail: impl Into<String>) -> Self {
        Self::Conflict {
            entity,
            detail: detail.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, PersistenceError>;

/// Async CRUD facade for every Keycloak JPA entity.
///
/// Soft delete: every `delete_*` method sets `audit.deleted_at` and
/// every `list/get/count` filter tombstoned rows by default.
///
/// Multi-tenancy: any entity with a `realm_id` FK exposes a
/// `list_*_in_realm` helper. Realm itself is keyed by name.
#[async_trait]
pub trait PersistenceBackend: Send + Sync {
    // ── Realm ────────────────────────────────────────────────────────────
    async fn list_realms(&self) -> Result<Vec<RealmEntity>>;
    async fn get_realm_by_id(&self, id: Uuid) -> Result<Option<RealmEntity>>;
    async fn get_realm_by_name(&self, name: &str) -> Result<Option<RealmEntity>>;
    async fn create_realm(&self, r: RealmEntity) -> Result<RealmEntity>;
    async fn update_realm(&self, r: RealmEntity) -> Result<RealmEntity>;
    async fn delete_realm(&self, id: Uuid) -> Result<()>;
    async fn count_realms(&self) -> Result<usize>;

    // ── User ─────────────────────────────────────────────────────────────
    async fn list_users_in_realm(&self, realm_id: Uuid) -> Result<Vec<UserEntity>>;
    async fn get_user_by_id(&self, id: Uuid) -> Result<Option<UserEntity>>;
    async fn get_user_by_name(&self, realm_id: Uuid, username: &str) -> Result<Option<UserEntity>>;
    async fn create_user(&self, u: UserEntity) -> Result<UserEntity>;
    async fn update_user(&self, u: UserEntity) -> Result<UserEntity>;
    async fn delete_user(&self, id: Uuid) -> Result<()>;
    async fn count_users(&self, realm_id: Uuid) -> Result<usize>;

    // ── Client ───────────────────────────────────────────────────────────
    async fn list_clients_in_realm(&self, realm_id: Uuid) -> Result<Vec<ClientEntity>>;
    async fn get_client_by_id(&self, id: Uuid) -> Result<Option<ClientEntity>>;
    async fn get_client_by_name(
        &self,
        realm_id: Uuid,
        client_id: &str,
    ) -> Result<Option<ClientEntity>>;
    async fn create_client(&self, c: ClientEntity) -> Result<ClientEntity>;
    async fn update_client(&self, c: ClientEntity) -> Result<ClientEntity>;
    async fn delete_client(&self, id: Uuid) -> Result<()>;
    async fn count_clients(&self, realm_id: Uuid) -> Result<usize>;

    // ── Role ─────────────────────────────────────────────────────────────
    async fn list_roles_in_realm(&self, realm_id: Uuid) -> Result<Vec<RoleEntity>>;
    async fn get_role_by_id(&self, id: Uuid) -> Result<Option<RoleEntity>>;
    async fn get_role_by_name(&self, realm_id: Uuid, name: &str) -> Result<Option<RoleEntity>>;
    async fn create_role(&self, r: RoleEntity) -> Result<RoleEntity>;
    async fn update_role(&self, r: RoleEntity) -> Result<RoleEntity>;
    async fn delete_role(&self, id: Uuid) -> Result<()>;
    async fn count_roles(&self, realm_id: Uuid) -> Result<usize>;

    // ── Group ────────────────────────────────────────────────────────────
    async fn list_groups_in_realm(&self, realm_id: Uuid) -> Result<Vec<GroupEntity>>;
    async fn get_group_by_id(&self, id: Uuid) -> Result<Option<GroupEntity>>;
    async fn get_group_by_name(&self, realm_id: Uuid, name: &str) -> Result<Option<GroupEntity>>;
    async fn create_group(&self, g: GroupEntity) -> Result<GroupEntity>;
    async fn update_group(&self, g: GroupEntity) -> Result<GroupEntity>;
    async fn delete_group(&self, id: Uuid) -> Result<()>;
    async fn count_groups(&self, realm_id: Uuid) -> Result<usize>;

    // ── IdentityProvider ─────────────────────────────────────────────────
    async fn list_idps_in_realm(&self, realm_id: Uuid) -> Result<Vec<IdentityProviderEntity>>;
    async fn get_idp_by_id(&self, id: Uuid) -> Result<Option<IdentityProviderEntity>>;
    async fn get_idp_by_name(
        &self,
        realm_id: Uuid,
        alias: &str,
    ) -> Result<Option<IdentityProviderEntity>>;
    async fn create_idp(&self, i: IdentityProviderEntity) -> Result<IdentityProviderEntity>;
    async fn update_idp(&self, i: IdentityProviderEntity) -> Result<IdentityProviderEntity>;
    async fn delete_idp(&self, id: Uuid) -> Result<()>;
    async fn count_idps(&self, realm_id: Uuid) -> Result<usize>;

    // ── AuthenticationFlow ───────────────────────────────────────────────
    async fn list_flows_in_realm(&self, realm_id: Uuid) -> Result<Vec<AuthFlowEntity>>;
    async fn get_flow_by_id(&self, id: Uuid) -> Result<Option<AuthFlowEntity>>;
    async fn get_flow_by_name(&self, realm_id: Uuid, alias: &str)
    -> Result<Option<AuthFlowEntity>>;
    async fn create_flow(&self, f: AuthFlowEntity) -> Result<AuthFlowEntity>;
    async fn update_flow(&self, f: AuthFlowEntity) -> Result<AuthFlowEntity>;
    async fn delete_flow(&self, id: Uuid) -> Result<()>;
    async fn count_flows(&self, realm_id: Uuid) -> Result<usize>;

    // ── Transaction ──────────────────────────────────────────────────────
    async fn begin_txn(&self) -> Result<Box<dyn Transaction>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_not_found_helper_carries_entity_and_id() {
        let id = Uuid::new_v4();
        let e = PersistenceError::not_found("realm", id);
        let msg = format!("{e}");
        assert!(msg.contains("realm"));
        assert!(msg.contains(&id.to_string()));
    }

    #[test]
    fn error_conflict_helper() {
        let e = PersistenceError::conflict("user", "username `alice` already exists");
        let msg = format!("{e}");
        assert!(msg.contains("user"));
        assert!(msg.contains("alice"));
    }

    #[test]
    fn error_variants_implement_std_error() {
        fn _accepts_std_error(_: &dyn std::error::Error) {}
        let e = PersistenceError::Backend("oom".to_string());
        _accepts_std_error(&e);
    }
}
