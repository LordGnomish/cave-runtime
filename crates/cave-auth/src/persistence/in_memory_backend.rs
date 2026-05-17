// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 model/map/src/main/java/org/keycloak/models/map/storage/chm/ConcurrentHashMapStorage.java
//
// RED-phase skeleton — trait method bodies all return a deterministic
// `PersistenceError::Backend("not implemented in RED phase")`, which
// causes every `#[tokio::test]` in this file to fail until the GREEN
// commit lands the real ConcurrentHashMap-backed implementation.

//! `InMemoryBackend` — in-memory implementation of [`PersistenceBackend`] (RED).

use async_trait::async_trait;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use super::backend::{PersistenceBackend, PersistenceError, Result};
use super::entities::{
    AuthFlowEntity, ClientEntity, GroupEntity, IdentityProviderEntity, RealmEntity, RoleEntity,
    UserEntity,
};
use super::txn::Transaction;

#[derive(Default, Clone)]
pub struct InMemoryBackend {
    _inner: Arc<RwLock<()>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct InMemoryTxn;

#[async_trait]
impl Transaction for InMemoryTxn {
    async fn commit(self: Box<Self>) -> Result<()> {
        Err(PersistenceError::Backend(
            "not implemented in RED phase".into(),
        ))
    }
    async fn rollback(self: Box<Self>) -> Result<()> {
        Err(PersistenceError::Backend(
            "not implemented in RED phase".into(),
        ))
    }
    fn is_in_memory(&self) -> bool {
        false // RED stub claims false to trip the test
    }
}

fn red<T>() -> Result<T> {
    Err(PersistenceError::Backend(
        "not implemented in RED phase".into(),
    ))
}

#[async_trait]
impl PersistenceBackend for InMemoryBackend {
    async fn list_realms(&self) -> Result<Vec<RealmEntity>> { red() }
    async fn get_realm_by_id(&self, _id: Uuid) -> Result<Option<RealmEntity>> { red() }
    async fn get_realm_by_name(&self, _name: &str) -> Result<Option<RealmEntity>> { red() }
    async fn create_realm(&self, _r: RealmEntity) -> Result<RealmEntity> { red() }
    async fn update_realm(&self, _r: RealmEntity) -> Result<RealmEntity> { red() }
    async fn delete_realm(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_realms(&self) -> Result<usize> { red() }

    async fn list_users_in_realm(&self, _realm_id: Uuid) -> Result<Vec<UserEntity>> { red() }
    async fn get_user_by_id(&self, _id: Uuid) -> Result<Option<UserEntity>> { red() }
    async fn get_user_by_name(&self, _realm_id: Uuid, _username: &str) -> Result<Option<UserEntity>> { red() }
    async fn create_user(&self, _u: UserEntity) -> Result<UserEntity> { red() }
    async fn update_user(&self, _u: UserEntity) -> Result<UserEntity> { red() }
    async fn delete_user(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_users(&self, _realm_id: Uuid) -> Result<usize> { red() }

    async fn list_clients_in_realm(&self, _realm_id: Uuid) -> Result<Vec<ClientEntity>> { red() }
    async fn get_client_by_id(&self, _id: Uuid) -> Result<Option<ClientEntity>> { red() }
    async fn get_client_by_name(&self, _realm_id: Uuid, _client_id: &str) -> Result<Option<ClientEntity>> { red() }
    async fn create_client(&self, _c: ClientEntity) -> Result<ClientEntity> { red() }
    async fn update_client(&self, _c: ClientEntity) -> Result<ClientEntity> { red() }
    async fn delete_client(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_clients(&self, _realm_id: Uuid) -> Result<usize> { red() }

    async fn list_roles_in_realm(&self, _realm_id: Uuid) -> Result<Vec<RoleEntity>> { red() }
    async fn get_role_by_id(&self, _id: Uuid) -> Result<Option<RoleEntity>> { red() }
    async fn get_role_by_name(&self, _realm_id: Uuid, _name: &str) -> Result<Option<RoleEntity>> { red() }
    async fn create_role(&self, _r: RoleEntity) -> Result<RoleEntity> { red() }
    async fn update_role(&self, _r: RoleEntity) -> Result<RoleEntity> { red() }
    async fn delete_role(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_roles(&self, _realm_id: Uuid) -> Result<usize> { red() }

    async fn list_groups_in_realm(&self, _realm_id: Uuid) -> Result<Vec<GroupEntity>> { red() }
    async fn get_group_by_id(&self, _id: Uuid) -> Result<Option<GroupEntity>> { red() }
    async fn get_group_by_name(&self, _realm_id: Uuid, _name: &str) -> Result<Option<GroupEntity>> { red() }
    async fn create_group(&self, _g: GroupEntity) -> Result<GroupEntity> { red() }
    async fn update_group(&self, _g: GroupEntity) -> Result<GroupEntity> { red() }
    async fn delete_group(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_groups(&self, _realm_id: Uuid) -> Result<usize> { red() }

    async fn list_idps_in_realm(&self, _realm_id: Uuid) -> Result<Vec<IdentityProviderEntity>> { red() }
    async fn get_idp_by_id(&self, _id: Uuid) -> Result<Option<IdentityProviderEntity>> { red() }
    async fn get_idp_by_name(&self, _realm_id: Uuid, _alias: &str) -> Result<Option<IdentityProviderEntity>> { red() }
    async fn create_idp(&self, _i: IdentityProviderEntity) -> Result<IdentityProviderEntity> { red() }
    async fn update_idp(&self, _i: IdentityProviderEntity) -> Result<IdentityProviderEntity> { red() }
    async fn delete_idp(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_idps(&self, _realm_id: Uuid) -> Result<usize> { red() }

    async fn list_flows_in_realm(&self, _realm_id: Uuid) -> Result<Vec<AuthFlowEntity>> { red() }
    async fn get_flow_by_id(&self, _id: Uuid) -> Result<Option<AuthFlowEntity>> { red() }
    async fn get_flow_by_name(&self, _realm_id: Uuid, _alias: &str) -> Result<Option<AuthFlowEntity>> { red() }
    async fn create_flow(&self, _f: AuthFlowEntity) -> Result<AuthFlowEntity> { red() }
    async fn update_flow(&self, _f: AuthFlowEntity) -> Result<AuthFlowEntity> { red() }
    async fn delete_flow(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_flows(&self, _realm_id: Uuid) -> Result<usize> { red() }

    async fn begin_txn(&self) -> Result<Box<dyn Transaction>> { red() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::entities::*;

    #[tokio::test]
    async fn create_and_get_realm_roundtrip() {
        let b = InMemoryBackend::new();
        let r = RealmEntity::new("master");
        let created = b.create_realm(r.clone()).await.unwrap();
        let got = b.get_realm_by_id(created.id).await.unwrap();
        assert_eq!(got.as_ref().map(|x| x.name.as_str()), Some("master"));
        assert_eq!(b.get_realm_by_name("master").await.unwrap(), Some(r));
    }

    #[tokio::test]
    async fn realm_name_unique_conflict() {
        let b = InMemoryBackend::new();
        b.create_realm(RealmEntity::new("master")).await.unwrap();
        let err = b.create_realm(RealmEntity::new("master")).await;
        assert!(matches!(err, Err(PersistenceError::Conflict { .. })));
    }

    #[tokio::test]
    async fn delete_is_soft_and_count_excludes() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        assert_eq!(b.count_realms().await.unwrap(), 1);
        b.delete_realm(r.id).await.unwrap();
        assert_eq!(b.count_realms().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn user_unique_within_realm() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        b.create_user(UserEntity::new(r.id, "alice")).await.unwrap();
        let err = b.create_user(UserEntity::new(r.id, "alice")).await;
        assert!(matches!(err, Err(PersistenceError::Conflict { .. })));
    }

    #[tokio::test]
    async fn txn_commit_keeps_changes() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let txn = b.begin_txn().await.unwrap();
        b.create_user(UserEntity::new(r.id, "alice")).await.unwrap();
        txn.commit().await.unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn txn_rollback_restores_snapshot() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let txn = b.begin_txn().await.unwrap();
        b.create_user(UserEntity::new(r.id, "transient"))
            .await
            .unwrap();
        txn.rollback().await.unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn txn_is_in_memory_flag() {
        let b = InMemoryBackend::new();
        let txn = b.begin_txn().await.unwrap();
        assert!(txn.is_in_memory());
        txn.rollback().await.unwrap();
    }
}
