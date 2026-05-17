// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 model/map/src/main/java/org/keycloak/models/map/storage/chm/ConcurrentHashMapStorage.java
//
// Port of the upstream `map`-storage backend (the in-memory store used
// for tests and local dev). We mirror the JPA semantics — soft delete,
// unique constraints, transactional snapshot — so unit tests can pass
// against either backend interchangeably.

//! `InMemoryBackend` — in-memory implementation of [`PersistenceBackend`].

use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use super::backend::{PersistenceBackend, PersistenceError, Result};
use super::entities::{
    AuthFlowEntity, ClientEntity, GroupEntity, IdentityProviderEntity, RealmEntity, RoleEntity,
    UserEntity,
};
use super::txn::Transaction;

#[derive(Default, Clone)]
struct Tables {
    realms: HashMap<Uuid, RealmEntity>,
    users: HashMap<Uuid, UserEntity>,
    clients: HashMap<Uuid, ClientEntity>,
    roles: HashMap<Uuid, RoleEntity>,
    groups: HashMap<Uuid, GroupEntity>,
    idps: HashMap<Uuid, IdentityProviderEntity>,
    flows: HashMap<Uuid, AuthFlowEntity>,
}

/// Concurrency-safe `Arc<RwLock<HashMap<…>>>` over every entity type.
#[derive(Default, Clone)]
pub struct InMemoryBackend {
    inner: Arc<RwLock<Tables>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn snapshot(&self) -> Tables {
        self.inner.read().expect("rwlock poisoned").clone()
    }

    fn restore(&self, snap: Tables) {
        *self.inner.write().expect("rwlock poisoned") = snap;
    }
}

// ── In-memory copy-on-write transaction ──────────────────────────────────────

pub struct InMemoryTxn {
    backend: InMemoryBackend,
    snapshot: Tables,
    /// On commit we keep current state; on rollback we restore the snapshot.
    /// Set to false on `commit()` so `drop` (rollback default) is a no-op.
    rollback_on_drop: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl Transaction for InMemoryTxn {
    async fn commit(self: Box<Self>) -> Result<()> {
        self.rollback_on_drop
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    async fn rollback(self: Box<Self>) -> Result<()> {
        // Restore the snapshot taken at begin_txn().
        self.backend.restore(self.snapshot.clone());
        self.rollback_on_drop
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn is_in_memory(&self) -> bool {
        true
    }
}

impl Drop for InMemoryTxn {
    fn drop(&mut self) {
        if self
            .rollback_on_drop
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.backend.restore(self.snapshot.clone());
        }
    }
}

#[async_trait]
impl PersistenceBackend for InMemoryBackend {
    // ── Realm ────────────────────────────────────────────────────────────
    async fn list_realms(&self) -> Result<Vec<RealmEntity>> {
        Ok(self
            .snapshot()
            .realms
            .values()
            .filter(|r| !r.audit.is_deleted())
            .cloned()
            .collect())
    }
    async fn get_realm_by_id(&self, id: Uuid) -> Result<Option<RealmEntity>> {
        Ok(self
            .snapshot()
            .realms
            .get(&id)
            .filter(|r| !r.audit.is_deleted())
            .cloned())
    }
    async fn get_realm_by_name(&self, name: &str) -> Result<Option<RealmEntity>> {
        Ok(self
            .snapshot()
            .realms
            .values()
            .find(|r| r.name == name && !r.audit.is_deleted())
            .cloned())
    }
    async fn create_realm(&self, r: RealmEntity) -> Result<RealmEntity> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if guard
            .realms
            .values()
            .any(|x| x.name == r.name && !x.audit.is_deleted())
        {
            return Err(PersistenceError::conflict(
                "realm",
                format!("realm name `{}` already exists", r.name),
            ));
        }
        guard.realms.insert(r.id, r.clone());
        Ok(r)
    }
    async fn update_realm(&self, mut r: RealmEntity) -> Result<RealmEntity> {
        r.audit.updated_at = Utc::now();
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if !guard.realms.contains_key(&r.id) {
            return Err(PersistenceError::not_found("realm", r.id));
        }
        guard.realms.insert(r.id, r.clone());
        Ok(r)
    }
    async fn delete_realm(&self, id: Uuid) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let row = guard
            .realms
            .get_mut(&id)
            .ok_or_else(|| PersistenceError::not_found("realm", id))?;
        row.audit.deleted_at = Some(Utc::now());
        row.audit.updated_at = Utc::now();
        Ok(())
    }
    async fn count_realms(&self) -> Result<usize> {
        Ok(self
            .snapshot()
            .realms
            .values()
            .filter(|r| !r.audit.is_deleted())
            .count())
    }

    // ── User ─────────────────────────────────────────────────────────────
    async fn list_users_in_realm(&self, realm_id: Uuid) -> Result<Vec<UserEntity>> {
        Ok(self
            .snapshot()
            .users
            .values()
            .filter(|u| u.realm_id == realm_id && !u.audit.is_deleted())
            .cloned()
            .collect())
    }
    async fn get_user_by_id(&self, id: Uuid) -> Result<Option<UserEntity>> {
        Ok(self
            .snapshot()
            .users
            .get(&id)
            .filter(|u| !u.audit.is_deleted())
            .cloned())
    }
    async fn get_user_by_name(&self, realm_id: Uuid, username: &str) -> Result<Option<UserEntity>> {
        Ok(self
            .snapshot()
            .users
            .values()
            .find(|u| u.realm_id == realm_id && u.username == username && !u.audit.is_deleted())
            .cloned())
    }
    async fn create_user(&self, u: UserEntity) -> Result<UserEntity> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if guard.users.values().any(|x| {
            x.realm_id == u.realm_id && x.username == u.username && !x.audit.is_deleted()
        }) {
            return Err(PersistenceError::conflict(
                "user",
                format!("username `{}` already exists in realm", u.username),
            ));
        }
        guard.users.insert(u.id, u.clone());
        Ok(u)
    }
    async fn update_user(&self, mut u: UserEntity) -> Result<UserEntity> {
        u.audit.updated_at = Utc::now();
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if !guard.users.contains_key(&u.id) {
            return Err(PersistenceError::not_found("user", u.id));
        }
        guard.users.insert(u.id, u.clone());
        Ok(u)
    }
    async fn delete_user(&self, id: Uuid) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let row = guard
            .users
            .get_mut(&id)
            .ok_or_else(|| PersistenceError::not_found("user", id))?;
        row.audit.deleted_at = Some(Utc::now());
        Ok(())
    }
    async fn count_users(&self, realm_id: Uuid) -> Result<usize> {
        Ok(self
            .snapshot()
            .users
            .values()
            .filter(|u| u.realm_id == realm_id && !u.audit.is_deleted())
            .count())
    }

    // ── Client ───────────────────────────────────────────────────────────
    async fn list_clients_in_realm(&self, realm_id: Uuid) -> Result<Vec<ClientEntity>> {
        Ok(self
            .snapshot()
            .clients
            .values()
            .filter(|c| c.realm_id == realm_id && !c.audit.is_deleted())
            .cloned()
            .collect())
    }
    async fn get_client_by_id(&self, id: Uuid) -> Result<Option<ClientEntity>> {
        Ok(self
            .snapshot()
            .clients
            .get(&id)
            .filter(|c| !c.audit.is_deleted())
            .cloned())
    }
    async fn get_client_by_name(
        &self,
        realm_id: Uuid,
        client_id: &str,
    ) -> Result<Option<ClientEntity>> {
        Ok(self
            .snapshot()
            .clients
            .values()
            .find(|c| c.realm_id == realm_id && c.client_id == client_id && !c.audit.is_deleted())
            .cloned())
    }
    async fn create_client(&self, c: ClientEntity) -> Result<ClientEntity> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if guard.clients.values().any(|x| {
            x.realm_id == c.realm_id && x.client_id == c.client_id && !x.audit.is_deleted()
        }) {
            return Err(PersistenceError::conflict(
                "client",
                format!("client_id `{}` already exists in realm", c.client_id),
            ));
        }
        guard.clients.insert(c.id, c.clone());
        Ok(c)
    }
    async fn update_client(&self, mut c: ClientEntity) -> Result<ClientEntity> {
        c.audit.updated_at = Utc::now();
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if !guard.clients.contains_key(&c.id) {
            return Err(PersistenceError::not_found("client", c.id));
        }
        guard.clients.insert(c.id, c.clone());
        Ok(c)
    }
    async fn delete_client(&self, id: Uuid) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let row = guard
            .clients
            .get_mut(&id)
            .ok_or_else(|| PersistenceError::not_found("client", id))?;
        row.audit.deleted_at = Some(Utc::now());
        Ok(())
    }
    async fn count_clients(&self, realm_id: Uuid) -> Result<usize> {
        Ok(self
            .snapshot()
            .clients
            .values()
            .filter(|c| c.realm_id == realm_id && !c.audit.is_deleted())
            .count())
    }

    // ── Role ─────────────────────────────────────────────────────────────
    async fn list_roles_in_realm(&self, realm_id: Uuid) -> Result<Vec<RoleEntity>> {
        Ok(self
            .snapshot()
            .roles
            .values()
            .filter(|r| r.realm_id == realm_id && !r.audit.is_deleted())
            .cloned()
            .collect())
    }
    async fn get_role_by_id(&self, id: Uuid) -> Result<Option<RoleEntity>> {
        Ok(self
            .snapshot()
            .roles
            .get(&id)
            .filter(|r| !r.audit.is_deleted())
            .cloned())
    }
    async fn get_role_by_name(&self, realm_id: Uuid, name: &str) -> Result<Option<RoleEntity>> {
        Ok(self
            .snapshot()
            .roles
            .values()
            .find(|r| r.realm_id == realm_id && r.name == name && !r.audit.is_deleted())
            .cloned())
    }
    async fn create_role(&self, r: RoleEntity) -> Result<RoleEntity> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if guard.roles.values().any(|x| {
            x.realm_id == r.realm_id
                && x.client_id == r.client_id
                && x.name == r.name
                && !x.audit.is_deleted()
        }) {
            return Err(PersistenceError::conflict(
                "role",
                format!("role `{}` already exists in realm/client scope", r.name),
            ));
        }
        guard.roles.insert(r.id, r.clone());
        Ok(r)
    }
    async fn update_role(&self, mut r: RoleEntity) -> Result<RoleEntity> {
        r.audit.updated_at = Utc::now();
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if !guard.roles.contains_key(&r.id) {
            return Err(PersistenceError::not_found("role", r.id));
        }
        guard.roles.insert(r.id, r.clone());
        Ok(r)
    }
    async fn delete_role(&self, id: Uuid) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let row = guard
            .roles
            .get_mut(&id)
            .ok_or_else(|| PersistenceError::not_found("role", id))?;
        row.audit.deleted_at = Some(Utc::now());
        Ok(())
    }
    async fn count_roles(&self, realm_id: Uuid) -> Result<usize> {
        Ok(self
            .snapshot()
            .roles
            .values()
            .filter(|r| r.realm_id == realm_id && !r.audit.is_deleted())
            .count())
    }

    // ── Group ────────────────────────────────────────────────────────────
    async fn list_groups_in_realm(&self, realm_id: Uuid) -> Result<Vec<GroupEntity>> {
        Ok(self
            .snapshot()
            .groups
            .values()
            .filter(|g| g.realm_id == realm_id && !g.audit.is_deleted())
            .cloned()
            .collect())
    }
    async fn get_group_by_id(&self, id: Uuid) -> Result<Option<GroupEntity>> {
        Ok(self
            .snapshot()
            .groups
            .get(&id)
            .filter(|g| !g.audit.is_deleted())
            .cloned())
    }
    async fn get_group_by_name(&self, realm_id: Uuid, name: &str) -> Result<Option<GroupEntity>> {
        Ok(self
            .snapshot()
            .groups
            .values()
            .find(|g| g.realm_id == realm_id && g.name == name && !g.audit.is_deleted())
            .cloned())
    }
    async fn create_group(&self, g: GroupEntity) -> Result<GroupEntity> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        guard.groups.insert(g.id, g.clone());
        Ok(g)
    }
    async fn update_group(&self, mut g: GroupEntity) -> Result<GroupEntity> {
        g.audit.updated_at = Utc::now();
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if !guard.groups.contains_key(&g.id) {
            return Err(PersistenceError::not_found("group", g.id));
        }
        guard.groups.insert(g.id, g.clone());
        Ok(g)
    }
    async fn delete_group(&self, id: Uuid) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let row = guard
            .groups
            .get_mut(&id)
            .ok_or_else(|| PersistenceError::not_found("group", id))?;
        row.audit.deleted_at = Some(Utc::now());
        Ok(())
    }
    async fn count_groups(&self, realm_id: Uuid) -> Result<usize> {
        Ok(self
            .snapshot()
            .groups
            .values()
            .filter(|g| g.realm_id == realm_id && !g.audit.is_deleted())
            .count())
    }

    // ── IdentityProvider ─────────────────────────────────────────────────
    async fn list_idps_in_realm(&self, realm_id: Uuid) -> Result<Vec<IdentityProviderEntity>> {
        Ok(self
            .snapshot()
            .idps
            .values()
            .filter(|i| i.realm_id == realm_id && !i.audit.is_deleted())
            .cloned()
            .collect())
    }
    async fn get_idp_by_id(&self, id: Uuid) -> Result<Option<IdentityProviderEntity>> {
        Ok(self
            .snapshot()
            .idps
            .get(&id)
            .filter(|i| !i.audit.is_deleted())
            .cloned())
    }
    async fn get_idp_by_name(
        &self,
        realm_id: Uuid,
        alias: &str,
    ) -> Result<Option<IdentityProviderEntity>> {
        Ok(self
            .snapshot()
            .idps
            .values()
            .find(|i| i.realm_id == realm_id && i.alias == alias && !i.audit.is_deleted())
            .cloned())
    }
    async fn create_idp(&self, i: IdentityProviderEntity) -> Result<IdentityProviderEntity> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if guard
            .idps
            .values()
            .any(|x| x.realm_id == i.realm_id && x.alias == i.alias && !x.audit.is_deleted())
        {
            return Err(PersistenceError::conflict(
                "identity_provider",
                format!("alias `{}` already exists in realm", i.alias),
            ));
        }
        guard.idps.insert(i.id, i.clone());
        Ok(i)
    }
    async fn update_idp(
        &self,
        mut i: IdentityProviderEntity,
    ) -> Result<IdentityProviderEntity> {
        i.audit.updated_at = Utc::now();
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if !guard.idps.contains_key(&i.id) {
            return Err(PersistenceError::not_found("identity_provider", i.id));
        }
        guard.idps.insert(i.id, i.clone());
        Ok(i)
    }
    async fn delete_idp(&self, id: Uuid) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let row = guard
            .idps
            .get_mut(&id)
            .ok_or_else(|| PersistenceError::not_found("identity_provider", id))?;
        row.audit.deleted_at = Some(Utc::now());
        Ok(())
    }
    async fn count_idps(&self, realm_id: Uuid) -> Result<usize> {
        Ok(self
            .snapshot()
            .idps
            .values()
            .filter(|i| i.realm_id == realm_id && !i.audit.is_deleted())
            .count())
    }

    // ── AuthenticationFlow ───────────────────────────────────────────────
    async fn list_flows_in_realm(&self, realm_id: Uuid) -> Result<Vec<AuthFlowEntity>> {
        Ok(self
            .snapshot()
            .flows
            .values()
            .filter(|f| f.realm_id == realm_id && !f.audit.is_deleted())
            .cloned()
            .collect())
    }
    async fn get_flow_by_id(&self, id: Uuid) -> Result<Option<AuthFlowEntity>> {
        Ok(self
            .snapshot()
            .flows
            .get(&id)
            .filter(|f| !f.audit.is_deleted())
            .cloned())
    }
    async fn get_flow_by_name(
        &self,
        realm_id: Uuid,
        alias: &str,
    ) -> Result<Option<AuthFlowEntity>> {
        Ok(self
            .snapshot()
            .flows
            .values()
            .find(|f| f.realm_id == realm_id && f.alias == alias && !f.audit.is_deleted())
            .cloned())
    }
    async fn create_flow(&self, f: AuthFlowEntity) -> Result<AuthFlowEntity> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if guard
            .flows
            .values()
            .any(|x| x.realm_id == f.realm_id && x.alias == f.alias && !x.audit.is_deleted())
        {
            return Err(PersistenceError::conflict(
                "auth_flow",
                format!("flow alias `{}` already exists in realm", f.alias),
            ));
        }
        guard.flows.insert(f.id, f.clone());
        Ok(f)
    }
    async fn update_flow(&self, mut f: AuthFlowEntity) -> Result<AuthFlowEntity> {
        f.audit.updated_at = Utc::now();
        let mut guard = self.inner.write().expect("rwlock poisoned");
        if !guard.flows.contains_key(&f.id) {
            return Err(PersistenceError::not_found("auth_flow", f.id));
        }
        guard.flows.insert(f.id, f.clone());
        Ok(f)
    }
    async fn delete_flow(&self, id: Uuid) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let row = guard
            .flows
            .get_mut(&id)
            .ok_or_else(|| PersistenceError::not_found("auth_flow", id))?;
        row.audit.deleted_at = Some(Utc::now());
        Ok(())
    }
    async fn count_flows(&self, realm_id: Uuid) -> Result<usize> {
        Ok(self
            .snapshot()
            .flows
            .values()
            .filter(|f| f.realm_id == realm_id && !f.audit.is_deleted())
            .count())
    }

    // ── Transaction ──────────────────────────────────────────────────────
    async fn begin_txn(&self) -> Result<Box<dyn Transaction>> {
        Ok(Box::new(InMemoryTxn {
            backend: self.clone(),
            snapshot: self.snapshot(),
            rollback_on_drop: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }))
    }
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
        assert!(b.get_realm_by_id(r.id).await.unwrap().is_none());
        assert!(b.get_realm_by_name("r").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_bumps_updated_at() {
        let b = InMemoryBackend::new();
        let mut r = b.create_realm(RealmEntity::new("rr")).await.unwrap();
        let original_updated = r.audit.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        r.display_name = Some("Renamed".into());
        let after = b.update_realm(r).await.unwrap();
        assert!(after.audit.updated_at > original_updated);
        assert_eq!(after.display_name.as_deref(), Some("Renamed"));
    }

    #[tokio::test]
    async fn user_realm_scoping() {
        let b = InMemoryBackend::new();
        let r1 = b.create_realm(RealmEntity::new("r1")).await.unwrap();
        let r2 = b.create_realm(RealmEntity::new("r2")).await.unwrap();
        b.create_user(UserEntity::new(r1.id, "alice")).await.unwrap();
        b.create_user(UserEntity::new(r2.id, "alice")).await.unwrap();
        assert_eq!(b.count_users(r1.id).await.unwrap(), 1);
        assert_eq!(b.count_users(r2.id).await.unwrap(), 1);
        assert!(b
            .get_user_by_name(r1.id, "alice")
            .await
            .unwrap()
            .is_some());
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
    async fn client_crud_full_cycle() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let c = b.create_client(ClientEntity::new(r.id, "frontend")).await.unwrap();
        assert_eq!(b.list_clients_in_realm(r.id).await.unwrap().len(), 1);
        assert_eq!(
            b.get_client_by_name(r.id, "frontend").await.unwrap().unwrap().id,
            c.id
        );
        b.delete_client(c.id).await.unwrap();
        assert_eq!(b.count_clients(r.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn role_realm_and_client_scoped() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let client = b
            .create_client(ClientEntity::new(r.id, "api"))
            .await
            .unwrap();
        let realm_role = RoleEntity::new(r.id, "admin");
        let mut client_role = RoleEntity::new(r.id, "viewer");
        client_role.client_id = Some(client.id);
        b.create_role(realm_role).await.unwrap();
        b.create_role(client_role).await.unwrap();
        assert_eq!(b.count_roles(r.id).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn idp_alias_conflict() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        b.create_idp(IdentityProviderEntity::new(r.id, "okta", "oidc"))
            .await
            .unwrap();
        let err = b
            .create_idp(IdentityProviderEntity::new(r.id, "okta", "saml"))
            .await;
        assert!(matches!(err, Err(PersistenceError::Conflict { .. })));
    }

    #[tokio::test]
    async fn auth_flow_alias_conflict() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        b.create_flow(AuthFlowEntity::new(r.id, "browser"))
            .await
            .unwrap();
        let err = b.create_flow(AuthFlowEntity::new(r.id, "browser")).await;
        assert!(matches!(err, Err(PersistenceError::Conflict { .. })));
    }

    #[tokio::test]
    async fn update_unknown_returns_not_found() {
        let b = InMemoryBackend::new();
        let phantom = RealmEntity::new("ghost");
        let err = b.update_realm(phantom).await;
        assert!(matches!(err, Err(PersistenceError::NotFound { .. })));
    }

    #[tokio::test]
    async fn group_supports_nesting_persistence() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let parent = b
            .create_group(GroupEntity::new(r.id, "eng"))
            .await
            .unwrap();
        let mut child = GroupEntity::new(r.id, "backend");
        child.parent_id = Some(parent.id);
        let child = b.create_group(child).await.unwrap();
        let got = b.get_group_by_id(child.id).await.unwrap().unwrap();
        assert_eq!(got.parent_id, Some(parent.id));
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
        b.create_user(UserEntity::new(r.id, "preexisting"))
            .await
            .unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 1);

        let txn = b.begin_txn().await.unwrap();
        b.create_user(UserEntity::new(r.id, "transient"))
            .await
            .unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 2);
        txn.rollback().await.unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 1);
        assert!(b
            .get_user_by_name(r.id, "preexisting")
            .await
            .unwrap()
            .is_some());
        assert!(b
            .get_user_by_name(r.id, "transient")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn txn_drop_without_commit_rolls_back() {
        let b = InMemoryBackend::new();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        {
            let _txn = b.begin_txn().await.unwrap();
            b.create_user(UserEntity::new(r.id, "ephemeral"))
                .await
                .unwrap();
            assert_eq!(b.count_users(r.id).await.unwrap(), 1);
            // _txn dropped here — should roll back.
        }
        assert_eq!(b.count_users(r.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn txn_is_in_memory_flag() {
        let b = InMemoryBackend::new();
        let txn = b.begin_txn().await.unwrap();
        assert!(txn.is_in_memory());
        txn.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn delete_unknown_returns_not_found() {
        let b = InMemoryBackend::new();
        let err = b.delete_realm(Uuid::new_v4()).await;
        assert!(matches!(err, Err(PersistenceError::NotFound { .. })));
    }
}
