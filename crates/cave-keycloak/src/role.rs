// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Role + role mapping + composite role expansion.
//!
//! Upstream:
//!   * `services/src/main/java/org/keycloak/services/managers/RoleContainerResource.java`
//!   * `model/src/main/java/org/keycloak/models/RoleModel.java` (compositesStream)

use std::collections::BTreeSet;

use crate::error::Result;
use crate::events::{AuditEvent, EventKind, EventSink};
use crate::models::{Group, Role, User};
use crate::store::KeycloakStore;

pub struct RoleController<'a> {
    pub store: &'a KeycloakStore,
    pub events: &'a EventSink,
}

impl<'a> RoleController<'a> {
    pub fn create(&self, tenant_id: &str, r: Role) -> Result<Role> {
        let id = r.id.clone();
        self.store.put_role(tenant_id, r.clone())?;
        self.events.append(AuditEvent::new(tenant_id, &id, EventKind::RoleCreated));
        Ok(r)
    }

    pub fn assign_to_user(&self, tenant_id: &str, user_id: &str, role_id: &str) -> Result<()> {
        let role = self.store.get_role(tenant_id, role_id)?;
        let mut user = self.store.get_user(tenant_id, user_id)?;
        if let Some(cid) = role.client_id.clone() {
            if !user.client_role_ids.iter().any(|(c, r)| c == &cid && r == role_id) {
                user.client_role_ids.push((cid, role_id.into()));
            }
        } else if !user.realm_role_ids.iter().any(|r| r == role_id) {
            user.realm_role_ids.push(role_id.into());
        }
        self.store.update_user(tenant_id, user)
    }

    pub fn assign_to_group(&self, tenant_id: &str, group_id: &str, role_id: &str) -> Result<()> {
        let _ = self.store.get_role(tenant_id, role_id)?;
        let g: Group = self.store.get_group(tenant_id, group_id)?;
        let mut g = g;
        if !g.realm_role_ids.iter().any(|r| r == role_id) {
            g.realm_role_ids.push(role_id.into());
        }
        self.store.put_group(tenant_id, g)
    }

    /// Expand composite roles transitively — returns the closure of every
    /// role implied by the user's direct realm + client role assignments.
    pub fn effective_role_ids(&self, tenant_id: &str, user: &User) -> Result<BTreeSet<String>> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        let mut frontier: Vec<String> = user.realm_role_ids.clone();
        frontier.extend(user.client_role_ids.iter().map(|(_, r)| r.clone()));
        for g_id in &user.group_ids {
            if let Ok(g) = self.store.get_group(tenant_id, g_id) {
                frontier.extend(g.realm_role_ids);
            }
        }
        while let Some(rid) = frontier.pop() {
            if !out.insert(rid.clone()) {
                continue;
            }
            if let Ok(r) = self.store.get_role(tenant_id, &rid) {
                for c in r.composite_ids {
                    if !out.contains(&c) {
                        frontier.push(c);
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Client, GrantType, Protocol, Realm};
    use std::collections::BTreeMap;
    use chrono::Utc;

    fn setup() -> (KeycloakStore, EventSink) {
        let s = KeycloakStore::new();
        s.put_realm(Realm::new("r1", "t1", "R1")).unwrap();
        (s, EventSink::default())
    }

    fn role(id: &str, name: &str, composites: Vec<String>) -> Role {
        Role {
            id: id.into(),
            realm_id: "r1".into(),
            client_id: None,
            name: name.into(),
            description: None,
            composite_ids: composites,
        }
    }

    fn user(id: &str, name: &str, roles: Vec<String>) -> User {
        User {
            id: id.into(),
            realm_id: "r1".into(),
            username: name.into(),
            enabled: true,
            email: None,
            email_verified: false,
            first_name: None,
            last_name: None,
            federated_link: None,
            group_ids: vec![],
            realm_role_ids: roles,
            client_role_ids: vec![],
            attributes: BTreeMap::new(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn create_role_emits_event() {
        let (s, e) = setup();
        let ctl = RoleController { store: &s, events: &e };
        ctl.create("t1", role("rid-1", "admin", vec![])).unwrap();
        let drained = e.drain();
        assert!(drained.iter().any(|ev| ev.kind == EventKind::RoleCreated));
    }

    #[test]
    fn assign_realm_role_to_user() {
        let (s, e) = setup();
        let ctl = RoleController { store: &s, events: &e };
        ctl.create("t1", role("rid-1", "admin", vec![])).unwrap();
        s.put_user("t1", user("u1", "alice", vec![])).unwrap();
        ctl.assign_to_user("t1", "u1", "rid-1").unwrap();
        let u = s.get_user("t1", "u1").unwrap();
        assert!(u.realm_role_ids.contains(&"rid-1".into()));
    }

    #[test]
    fn assign_client_role_keyed_by_client() {
        let (s, e) = setup();
        let c = Client {
            id: "c1".into(),
            realm_id: "r1".into(),
            client_id: "app".into(),
            name: "App".into(),
            enabled: true,
            protocol: Protocol::OpenIdConnect,
            public_client: true,
            client_secret_hash: None,
            redirect_uris: vec!["https://a/cb".into()],
            web_origins: vec![],
            default_scopes: vec![],
            optional_scopes: vec![],
            allowed_grant_types: vec![GrantType::AuthorizationCode],
            require_pkce: true,
            access_token_lifespan_seconds: None,
            attributes: BTreeMap::new(),
        };
        s.put_client("t1", c).unwrap();
        let ctl = RoleController { store: &s, events: &e };
        let r = Role {
            id: "rid-c".into(),
            realm_id: "r1".into(),
            client_id: Some("c1".into()),
            name: "manager".into(),
            description: None,
            composite_ids: vec![],
        };
        ctl.create("t1", r).unwrap();
        s.put_user("t1", user("u1", "alice", vec![])).unwrap();
        ctl.assign_to_user("t1", "u1", "rid-c").unwrap();
        let u = s.get_user("t1", "u1").unwrap();
        assert_eq!(u.client_role_ids, vec![("c1".into(), "rid-c".into())]);
    }

    #[test]
    fn composite_role_expansion_closes_transitively() {
        let (s, e) = setup();
        let ctl = RoleController { store: &s, events: &e };
        ctl.create("t1", role("admin", "admin", vec!["editor".into()])).unwrap();
        ctl.create("t1", role("editor", "editor", vec!["viewer".into()])).unwrap();
        ctl.create("t1", role("viewer", "viewer", vec![])).unwrap();
        let u = user("u1", "alice", vec!["admin".into()]);
        s.put_user("t1", u.clone()).unwrap();
        let eff = ctl.effective_role_ids("t1", &u).unwrap();
        assert!(eff.contains("admin"));
        assert!(eff.contains("editor"));
        assert!(eff.contains("viewer"));
    }

    #[test]
    fn composite_cycle_terminates() {
        let (s, e) = setup();
        let ctl = RoleController { store: &s, events: &e };
        ctl.create("t1", role("a", "a", vec!["b".into()])).unwrap();
        ctl.create("t1", role("b", "b", vec!["a".into()])).unwrap();
        let u = user("u1", "alice", vec!["a".into()]);
        s.put_user("t1", u.clone()).unwrap();
        let eff = ctl.effective_role_ids("t1", &u).unwrap();
        assert!(eff.contains("a"));
        assert!(eff.contains("b"));
        assert_eq!(eff.len(), 2);
    }
}
