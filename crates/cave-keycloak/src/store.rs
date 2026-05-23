// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-tenant in-memory store for realms, users, groups, roles, clients,
//! credentials, and sessions. Every read passes through `check_tenant` so
//! cross-tenant access is rejected with structured error context.
//!
//! Upstream parity: `model/jpa/src/main/java/org/keycloak/models/jpa/*` —
//! the JPA-backed realm/user/role/client models. The cave port keeps the
//! same accessor surface but stores everything in-memory; cave-db plugs in
//! later by swapping the inner `Mutex<BTreeMap<…>>` with a SQL-backed
//! transactional store.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::{KeycloakError, Result};
use crate::models::{Client, Group, Realm, Role, User};

/// Multi-tenant store. All maps are keyed by `id` (UUID) but every read
/// goes through `check_tenant` to enforce the cave invariant: a request
/// from `tenant_x` cannot touch a record owned by `tenant_y`.
pub struct KeycloakStore {
    realms: Mutex<BTreeMap<String, Realm>>,
    users: Mutex<BTreeMap<String, User>>,
    groups: Mutex<BTreeMap<String, Group>>,
    roles: Mutex<BTreeMap<String, Role>>,
    clients: Mutex<BTreeMap<String, Client>>,
}

impl Default for KeycloakStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeycloakStore {
    pub fn new() -> Self {
        Self {
            realms: Mutex::new(BTreeMap::new()),
            users: Mutex::new(BTreeMap::new()),
            groups: Mutex::new(BTreeMap::new()),
            roles: Mutex::new(BTreeMap::new()),
            clients: Mutex::new(BTreeMap::new()),
        }
    }

    // ── Realms ──────────────────────────────────────────────────────────

    pub fn put_realm(&self, r: Realm) -> Result<()> {
        r.validate()?;
        let mut realms = self.realms.lock().unwrap();
        if realms.contains_key(&r.id) {
            return Err(KeycloakError::RealmExists(r.id));
        }
        realms.insert(r.id.clone(), r);
        Ok(())
    }

    pub fn get_realm(&self, tenant_id: &str, id: &str) -> Result<Realm> {
        let realms = self.realms.lock().unwrap();
        let r = realms.get(id).ok_or_else(|| KeycloakError::RealmNotFound(id.to_string()))?;
        check_tenant(&r.tenant_id, tenant_id)?;
        Ok(r.clone())
    }

    pub fn delete_realm(&self, tenant_id: &str, id: &str) -> Result<()> {
        let mut realms = self.realms.lock().unwrap();
        let r = realms.get(id).ok_or_else(|| KeycloakError::RealmNotFound(id.to_string()))?;
        check_tenant(&r.tenant_id, tenant_id)?;
        realms.remove(id);
        Ok(())
    }

    pub fn list_realms(&self, tenant_id: &str) -> Vec<Realm> {
        let realms = self.realms.lock().unwrap();
        realms.values().filter(|r| r.tenant_id == tenant_id).cloned().collect()
    }

    pub fn realm_count(&self) -> usize {
        self.realms.lock().unwrap().len()
    }

    // ── Users ───────────────────────────────────────────────────────────

    pub fn put_user(&self, tenant_id: &str, u: User) -> Result<()> {
        u.validate()?;
        let realm = self.get_realm(tenant_id, &u.realm_id)?;
        if !realm.duplicate_emails_allowed {
            if let Some(email) = &u.email {
                let users = self.users.lock().unwrap();
                for existing in users.values() {
                    if existing.realm_id == u.realm_id
                        && existing.id != u.id
                        && existing.email.as_deref() == Some(email)
                    {
                        return Err(KeycloakError::UserExists(format!("duplicate-email:{}", email)));
                    }
                }
            }
        }
        let mut users = self.users.lock().unwrap();
        if users.contains_key(&u.id) {
            return Err(KeycloakError::UserExists(u.id));
        }
        users.insert(u.id.clone(), u);
        Ok(())
    }

    pub fn get_user(&self, tenant_id: &str, id: &str) -> Result<User> {
        let users = self.users.lock().unwrap();
        let u = users.get(id).ok_or_else(|| KeycloakError::UserNotFound(id.to_string()))?;
        let realm = self.get_realm(tenant_id, &u.realm_id)?;
        check_tenant(&realm.tenant_id, tenant_id)?;
        Ok(u.clone())
    }

    pub fn update_user(&self, tenant_id: &str, u: User) -> Result<()> {
        u.validate()?;
        let _ = self.get_user(tenant_id, &u.id)?;
        let mut users = self.users.lock().unwrap();
        users.insert(u.id.clone(), u);
        Ok(())
    }

    pub fn delete_user(&self, tenant_id: &str, id: &str) -> Result<()> {
        let _ = self.get_user(tenant_id, id)?;
        let mut users = self.users.lock().unwrap();
        users.remove(id);
        Ok(())
    }

    pub fn find_user_by_username(&self, tenant_id: &str, realm_id: &str, username: &str) -> Result<User> {
        let _ = self.get_realm(tenant_id, realm_id)?;
        let users = self.users.lock().unwrap();
        users
            .values()
            .find(|u| u.realm_id == realm_id && u.username == username)
            .cloned()
            .ok_or_else(|| KeycloakError::UserNotFound(format!("username={}", username)))
    }

    pub fn user_count(&self) -> usize {
        self.users.lock().unwrap().len()
    }

    // ── Groups ──────────────────────────────────────────────────────────

    pub fn put_group(&self, tenant_id: &str, g: Group) -> Result<()> {
        let _ = self.get_realm(tenant_id, &g.realm_id)?;
        if let Some(parent) = &g.parent_id {
            let groups = self.groups.lock().unwrap();
            let p = groups.get(parent).ok_or_else(|| KeycloakError::GroupNotFound(parent.clone()))?;
            if p.realm_id != g.realm_id {
                return Err(KeycloakError::InvalidRequest("parent group in different realm".into()));
            }
        }
        let mut groups = self.groups.lock().unwrap();
        groups.insert(g.id.clone(), g);
        Ok(())
    }

    pub fn get_group(&self, tenant_id: &str, id: &str) -> Result<Group> {
        let groups = self.groups.lock().unwrap();
        let g = groups.get(id).ok_or_else(|| KeycloakError::GroupNotFound(id.to_string()))?;
        let _ = self.get_realm(tenant_id, &g.realm_id)?;
        Ok(g.clone())
    }

    pub fn list_groups(&self, tenant_id: &str, realm_id: &str) -> Result<Vec<Group>> {
        let _ = self.get_realm(tenant_id, realm_id)?;
        let groups = self.groups.lock().unwrap();
        Ok(groups.values().filter(|g| g.realm_id == realm_id).cloned().collect())
    }

    // ── Roles ───────────────────────────────────────────────────────────

    pub fn put_role(&self, tenant_id: &str, r: Role) -> Result<()> {
        let _ = self.get_realm(tenant_id, &r.realm_id)?;
        if let Some(cid) = &r.client_id {
            let _ = self.get_client(tenant_id, cid)?;
        }
        let mut roles = self.roles.lock().unwrap();
        roles.insert(r.id.clone(), r);
        Ok(())
    }

    pub fn get_role(&self, tenant_id: &str, id: &str) -> Result<Role> {
        let roles = self.roles.lock().unwrap();
        let r = roles.get(id).ok_or_else(|| KeycloakError::RoleNotFound(id.to_string()))?;
        let _ = self.get_realm(tenant_id, &r.realm_id)?;
        Ok(r.clone())
    }

    pub fn list_roles(&self, tenant_id: &str, realm_id: &str) -> Result<Vec<Role>> {
        let _ = self.get_realm(tenant_id, realm_id)?;
        let roles = self.roles.lock().unwrap();
        Ok(roles.values().filter(|r| r.realm_id == realm_id).cloned().collect())
    }

    // ── Clients ─────────────────────────────────────────────────────────

    pub fn put_client(&self, tenant_id: &str, c: Client) -> Result<()> {
        c.validate()?;
        let _ = self.get_realm(tenant_id, &c.realm_id)?;
        let mut clients = self.clients.lock().unwrap();
        clients.insert(c.id.clone(), c);
        Ok(())
    }

    pub fn get_client(&self, tenant_id: &str, id: &str) -> Result<Client> {
        let clients = self.clients.lock().unwrap();
        let c = clients.get(id).ok_or_else(|| KeycloakError::ClientNotFound(id.to_string()))?;
        let _ = self.get_realm(tenant_id, &c.realm_id)?;
        Ok(c.clone())
    }

    pub fn find_client_by_client_id(&self, tenant_id: &str, realm_id: &str, client_id: &str) -> Result<Client> {
        let _ = self.get_realm(tenant_id, realm_id)?;
        let clients = self.clients.lock().unwrap();
        clients
            .values()
            .find(|c| c.realm_id == realm_id && c.client_id == client_id)
            .cloned()
            .ok_or_else(|| KeycloakError::ClientNotFound(format!("client_id={}", client_id)))
    }

    pub fn list_clients(&self, tenant_id: &str, realm_id: &str) -> Result<Vec<Client>> {
        let _ = self.get_realm(tenant_id, realm_id)?;
        let clients = self.clients.lock().unwrap();
        Ok(clients.values().filter(|c| c.realm_id == realm_id).cloned().collect())
    }
}

/// Cross-tenant guard. Returns `CrossTenantDenied` with both ids surfaced
/// for cave-oncall correlation when the owner and requester differ.
pub fn check_tenant(owner: &str, request: &str) -> Result<()> {
    if owner == request {
        Ok(())
    } else {
        Err(KeycloakError::CrossTenantDenied {
            owner_tenant: owner.to_string(),
            request_tenant: request.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{GrantType, Protocol};

    fn realm(id: &str, t: &str) -> Realm {
        Realm::new(id, t, format!("Realm {}", id))
    }

    fn user(id: &str, realm: &str, name: &str) -> User {
        User {
            id: id.into(),
            realm_id: realm.into(),
            username: name.into(),
            enabled: true,
            email: None,
            email_verified: false,
            first_name: None,
            last_name: None,
            federated_link: None,
            group_ids: vec![],
            realm_role_ids: vec![],
            client_role_ids: vec![],
            attributes: BTreeMap::new(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn put_get_realm_roundtrip() {
        let s = KeycloakStore::new();
        s.put_realm(realm("r1", "t1")).unwrap();
        let back = s.get_realm("t1", "r1").unwrap();
        assert_eq!(back.id, "r1");
    }

    #[test]
    fn cross_tenant_realm_read_denied() {
        let s = KeycloakStore::new();
        s.put_realm(realm("r1", "t1")).unwrap();
        let err = s.get_realm("t2", "r1").unwrap_err();
        match err {
            KeycloakError::CrossTenantDenied { owner_tenant, request_tenant } => {
                assert_eq!(owner_tenant, "t1");
                assert_eq!(request_tenant, "t2");
            }
            _ => panic!("expected CrossTenantDenied, got {:?}", err),
        }
    }

    #[test]
    fn duplicate_realm_id_rejected() {
        let s = KeycloakStore::new();
        s.put_realm(realm("r1", "t1")).unwrap();
        let err = s.put_realm(realm("r1", "t1")).unwrap_err();
        assert!(matches!(err, KeycloakError::RealmExists(_)));
    }

    #[test]
    fn duplicate_email_rejected_when_disallowed() {
        let s = KeycloakStore::new();
        s.put_realm(realm("r1", "t1")).unwrap();
        let mut u1 = user("u1", "r1", "alice");
        u1.email = Some("a@x".into());
        let mut u2 = user("u2", "r1", "bob");
        u2.email = Some("a@x".into());
        s.put_user("t1", u1).unwrap();
        let err = s.put_user("t1", u2).unwrap_err();
        assert!(matches!(err, KeycloakError::UserExists(_)));
    }

    #[test]
    fn user_lookup_by_username_in_correct_realm() {
        let s = KeycloakStore::new();
        s.put_realm(realm("r1", "t1")).unwrap();
        s.put_realm(realm("r2", "t1")).unwrap();
        s.put_user("t1", user("u1", "r1", "alice")).unwrap();
        s.put_user("t1", user("u2", "r2", "alice")).unwrap();
        let in_r1 = s.find_user_by_username("t1", "r1", "alice").unwrap();
        assert_eq!(in_r1.id, "u1");
        let in_r2 = s.find_user_by_username("t1", "r2", "alice").unwrap();
        assert_eq!(in_r2.id, "u2");
    }

    #[test]
    fn group_parent_must_share_realm() {
        let s = KeycloakStore::new();
        s.put_realm(realm("r1", "t1")).unwrap();
        s.put_realm(realm("r2", "t1")).unwrap();
        let parent = Group {
            id: "g1".into(),
            realm_id: "r1".into(),
            name: "Parent".into(),
            parent_id: None,
            attributes: BTreeMap::new(),
            realm_role_ids: vec![],
        };
        s.put_group("t1", parent).unwrap();
        let child = Group {
            id: "g2".into(),
            realm_id: "r2".into(),
            name: "Child".into(),
            parent_id: Some("g1".into()),
            attributes: BTreeMap::new(),
            realm_role_ids: vec![],
        };
        assert!(s.put_group("t1", child).is_err());
    }

    #[test]
    fn client_lookup_by_client_id_is_realm_scoped() {
        let s = KeycloakStore::new();
        s.put_realm(realm("r1", "t1")).unwrap();
        s.put_realm(realm("r2", "t1")).unwrap();
        let c1 = Client {
            id: "c1".into(),
            realm_id: "r1".into(),
            client_id: "spa".into(),
            name: "SPA".into(),
            enabled: true,
            protocol: Protocol::OpenIdConnect,
            public_client: true,
            client_secret_hash: None,
            redirect_uris: vec!["https://x/cb".into()],
            web_origins: vec![],
            default_scopes: vec![],
            optional_scopes: vec![],
            allowed_grant_types: vec![GrantType::AuthorizationCode],
            require_pkce: true,
            access_token_lifespan_seconds: None,
            attributes: BTreeMap::new(),
        };
        let mut c2 = c1.clone();
        c2.id = "c2".into();
        c2.realm_id = "r2".into();
        s.put_client("t1", c1).unwrap();
        s.put_client("t1", c2).unwrap();
        let f1 = s.find_client_by_client_id("t1", "r1", "spa").unwrap();
        assert_eq!(f1.id, "c1");
        let f2 = s.find_client_by_client_id("t1", "r2", "spa").unwrap();
        assert_eq!(f2.id, "c2");
    }
}
