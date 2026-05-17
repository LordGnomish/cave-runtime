// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/connections/jpa/JpaConnectionProviderFactory.java
//
// RED-phase skeleton — every trait body returns
// `PersistenceError::Backend("not implemented in RED phase")`. The
// associated `#[tokio::test]`s fail until the GREEN commit lands the
// rusqlite-backed real implementation.

//! `RdbmsBackend` — rusqlite-backed implementation of [`PersistenceBackend`] (RED).

use async_trait::async_trait;
use std::path::Path;
use uuid::Uuid;

use super::backend::{PersistenceBackend, PersistenceError, Result};
use super::entities::{
    AuthFlowEntity, ClientEntity, GroupEntity, IdentityProviderEntity, RealmEntity, RoleEntity,
    UserEntity,
};
use super::migration::AppliedMigration;
use super::txn::Transaction;

#[derive(Clone, Default)]
pub struct RdbmsBackend;

impl RdbmsBackend {
    pub fn open<P: AsRef<Path>>(_path: P) -> Result<Self> {
        Err(PersistenceError::Backend(
            "not implemented in RED phase".into(),
        ))
    }
    pub fn in_memory() -> Result<Self> {
        Err(PersistenceError::Backend(
            "not implemented in RED phase".into(),
        ))
    }
    pub fn applied_migrations(&self) -> Result<Vec<AppliedMigration>> {
        Err(PersistenceError::Backend(
            "not implemented in RED phase".into(),
        ))
    }
}

fn red<T>() -> Result<T> {
    Err(PersistenceError::Backend(
        "not implemented in RED phase".into(),
    ))
}

#[async_trait]
impl PersistenceBackend for RdbmsBackend {
    async fn list_realms(&self) -> Result<Vec<RealmEntity>> { red() }
    async fn get_realm_by_id(&self, _id: Uuid) -> Result<Option<RealmEntity>> { red() }
    async fn get_realm_by_name(&self, _name: &str) -> Result<Option<RealmEntity>> { red() }
    async fn create_realm(&self, _r: RealmEntity) -> Result<RealmEntity> { red() }
    async fn update_realm(&self, _r: RealmEntity) -> Result<RealmEntity> { red() }
    async fn delete_realm(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_realms(&self) -> Result<usize> { red() }

    async fn list_users_in_realm(&self, _r: Uuid) -> Result<Vec<UserEntity>> { red() }
    async fn get_user_by_id(&self, _id: Uuid) -> Result<Option<UserEntity>> { red() }
    async fn get_user_by_name(&self, _r: Uuid, _u: &str) -> Result<Option<UserEntity>> { red() }
    async fn create_user(&self, _u: UserEntity) -> Result<UserEntity> { red() }
    async fn update_user(&self, _u: UserEntity) -> Result<UserEntity> { red() }
    async fn delete_user(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_users(&self, _r: Uuid) -> Result<usize> { red() }

    async fn list_clients_in_realm(&self, _r: Uuid) -> Result<Vec<ClientEntity>> { red() }
    async fn get_client_by_id(&self, _id: Uuid) -> Result<Option<ClientEntity>> { red() }
    async fn get_client_by_name(&self, _r: Uuid, _cid: &str) -> Result<Option<ClientEntity>> { red() }
    async fn create_client(&self, _c: ClientEntity) -> Result<ClientEntity> { red() }
    async fn update_client(&self, _c: ClientEntity) -> Result<ClientEntity> { red() }
    async fn delete_client(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_clients(&self, _r: Uuid) -> Result<usize> { red() }

    async fn list_roles_in_realm(&self, _r: Uuid) -> Result<Vec<RoleEntity>> { red() }
    async fn get_role_by_id(&self, _id: Uuid) -> Result<Option<RoleEntity>> { red() }
    async fn get_role_by_name(&self, _r: Uuid, _n: &str) -> Result<Option<RoleEntity>> { red() }
    async fn create_role(&self, _r: RoleEntity) -> Result<RoleEntity> { red() }
    async fn update_role(&self, _r: RoleEntity) -> Result<RoleEntity> { red() }
    async fn delete_role(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_roles(&self, _r: Uuid) -> Result<usize> { red() }

    async fn list_groups_in_realm(&self, _r: Uuid) -> Result<Vec<GroupEntity>> { red() }
    async fn get_group_by_id(&self, _id: Uuid) -> Result<Option<GroupEntity>> { red() }
    async fn get_group_by_name(&self, _r: Uuid, _n: &str) -> Result<Option<GroupEntity>> { red() }
    async fn create_group(&self, _g: GroupEntity) -> Result<GroupEntity> { red() }
    async fn update_group(&self, _g: GroupEntity) -> Result<GroupEntity> { red() }
    async fn delete_group(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_groups(&self, _r: Uuid) -> Result<usize> { red() }

    async fn list_idps_in_realm(&self, _r: Uuid) -> Result<Vec<IdentityProviderEntity>> { red() }
    async fn get_idp_by_id(&self, _id: Uuid) -> Result<Option<IdentityProviderEntity>> { red() }
    async fn get_idp_by_name(&self, _r: Uuid, _a: &str) -> Result<Option<IdentityProviderEntity>> { red() }
    async fn create_idp(&self, _i: IdentityProviderEntity) -> Result<IdentityProviderEntity> { red() }
    async fn update_idp(&self, _i: IdentityProviderEntity) -> Result<IdentityProviderEntity> { red() }
    async fn delete_idp(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_idps(&self, _r: Uuid) -> Result<usize> { red() }

    async fn list_flows_in_realm(&self, _r: Uuid) -> Result<Vec<AuthFlowEntity>> { red() }
    async fn get_flow_by_id(&self, _id: Uuid) -> Result<Option<AuthFlowEntity>> { red() }
    async fn get_flow_by_name(&self, _r: Uuid, _a: &str) -> Result<Option<AuthFlowEntity>> { red() }
    async fn create_flow(&self, _f: AuthFlowEntity) -> Result<AuthFlowEntity> { red() }
    async fn update_flow(&self, _f: AuthFlowEntity) -> Result<AuthFlowEntity> { red() }
    async fn delete_flow(&self, _id: Uuid) -> Result<()> { red() }
    async fn count_flows(&self, _r: Uuid) -> Result<usize> { red() }

    async fn begin_txn(&self) -> Result<Box<dyn Transaction>> { red() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_applied_exactly_once() {
        let b = RdbmsBackend::in_memory().expect("open in-memory");
        let applied = b.applied_migrations().unwrap();
        let versions: Vec<_> = applied.iter().map(|m| m.version.as_str()).collect();
        assert_eq!(versions, vec!["V001", "V002", "V003", "V004"]);
    }

    #[tokio::test]
    async fn create_and_get_realm_roundtrip_rdbms() {
        let b = RdbmsBackend::in_memory().expect("open in-memory");
        let r = b
            .create_realm(RealmEntity::new("master"))
            .await
            .unwrap();
        let got = b.get_realm_by_id(r.id).await.unwrap().unwrap();
        assert_eq!(got.name, "master");
    }

    #[tokio::test]
    async fn realm_name_unique_conflict_rdbms() {
        let b = RdbmsBackend::in_memory().expect("open in-memory");
        b.create_realm(RealmEntity::new("dup")).await.unwrap();
        let err = b.create_realm(RealmEntity::new("dup")).await;
        assert!(matches!(err, Err(PersistenceError::Conflict { .. })));
    }

    #[tokio::test]
    async fn soft_delete_excludes_realm_rdbms() {
        let b = RdbmsBackend::in_memory().expect("open in-memory");
        let r = b
            .create_realm(RealmEntity::new("ephemeral"))
            .await
            .unwrap();
        b.delete_realm(r.id).await.unwrap();
        assert!(b.get_realm_by_id(r.id).await.unwrap().is_none());
        assert_eq!(b.count_realms().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn rdbms_txn_rollback_restores() {
        let b = RdbmsBackend::in_memory().expect("open in-memory");
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let txn = b.begin_txn().await.unwrap();
        b.create_user(UserEntity::new(r.id, "transient"))
            .await
            .unwrap();
        txn.rollback().await.unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 0);
    }
}
