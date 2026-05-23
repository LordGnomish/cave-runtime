// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Realm controller — orchestrates CRUD over the store and surfaces
//! `ReconcileEvent` records for the audit pipeline.

use crate::error::Result;
use crate::events::{AuditEvent, EventKind, EventSink};
use crate::models::Realm;
use crate::store::KeycloakStore;

pub struct RealmController<'a> {
    pub store: &'a KeycloakStore,
    pub events: &'a EventSink,
}

impl<'a> RealmController<'a> {
    pub fn create(&self, tenant_id: &str, r: Realm) -> Result<Realm> {
        let id = r.id.clone();
        self.store.put_realm(r.clone())?;
        self.events.append(AuditEvent::new(tenant_id, &id, EventKind::RealmCreated));
        Ok(r)
    }

    pub fn delete(&self, tenant_id: &str, id: &str) -> Result<()> {
        self.store.delete_realm(tenant_id, id)?;
        self.events.append(AuditEvent::new(tenant_id, id, EventKind::RealmDeleted));
        Ok(())
    }

    pub fn list(&self, tenant_id: &str) -> Vec<Realm> {
        self.store.list_realms(tenant_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_emits_realm_created_event() {
        let store = KeycloakStore::new();
        let events = EventSink::default();
        let ctl = RealmController { store: &store, events: &events };
        let r = Realm::new("r1", "t1", "R1");
        ctl.create("t1", r).unwrap();
        let drained = events.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].kind, EventKind::RealmCreated);
        assert_eq!(drained[0].subject, "r1");
    }

    #[test]
    fn delete_emits_realm_deleted_event() {
        let store = KeycloakStore::new();
        let events = EventSink::default();
        let ctl = RealmController { store: &store, events: &events };
        ctl.create("t1", Realm::new("r1", "t1", "R1")).unwrap();
        let _ = events.drain();
        ctl.delete("t1", "r1").unwrap();
        let drained = events.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].kind, EventKind::RealmDeleted);
    }

    #[test]
    fn list_is_tenant_scoped() {
        let store = KeycloakStore::new();
        let events = EventSink::default();
        let ctl = RealmController { store: &store, events: &events };
        ctl.create("t1", Realm::new("r1", "t1", "R1")).unwrap();
        ctl.create("t2", Realm::new("r2", "t2", "R2")).unwrap();
        assert_eq!(ctl.list("t1").len(), 1);
        assert_eq!(ctl.list("t2").len(), 1);
    }
}
