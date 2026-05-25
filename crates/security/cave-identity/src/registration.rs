// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). RegistrationEntry CRUD shape
// + selector matching line-ported from pkg/server/datastore/sqlstore +
// pkg/server/api/entry/v1.
//
//! Registration-entry CRUD + selector matching.

use crate::error::{IdentityError, Result};
use crate::models::{RegistrationEntry, Selector, SpiffeId};
use crate::spiffe_id::parse_spiffe_id;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

/// In-memory registration store — a placeholder for the sqlite-backed
/// [`crate::store::SqliteEntryStore`].
#[derive(Default)]
pub struct InMemoryEntryStore {
    inner: Arc<DashMap<String, RegistrationEntry>>,
}

impl InMemoryEntryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new entry — fails if `entry.id` already present, validates
    /// spiffe_id + parent_id + every selector.
    pub fn create(&self, mut entry: RegistrationEntry) -> Result<RegistrationEntry> {
        if entry.id.is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        parse_spiffe_id(entry.spiffe_id.as_str())?;
        parse_spiffe_id(entry.parent_id.as_str())?;
        for sel in &entry.selectors {
            validate_selector(sel)?;
        }
        if self.inner.contains_key(&entry.id) {
            return Err(IdentityError::EntryExists(entry.id));
        }
        self.inner.insert(entry.id.clone(), entry.clone());
        Ok(entry)
    }

    /// Fetch by id.
    pub fn get(&self, id: &str) -> Result<RegistrationEntry> {
        self.inner
            .get(id)
            .map(|e| e.clone())
            .ok_or_else(|| IdentityError::EntryNotFound(id.to_string()))
    }

    /// Update — replaces by id. Bumps `revision_number`.
    pub fn update(&self, mut entry: RegistrationEntry) -> Result<RegistrationEntry> {
        let prev = self.get(&entry.id)?;
        entry.revision_number = prev.revision_number + 1;
        self.inner.insert(entry.id.clone(), entry.clone());
        Ok(entry)
    }

    /// Delete — returns the removed entry or [`IdentityError::EntryNotFound`].
    pub fn delete(&self, id: &str) -> Result<RegistrationEntry> {
        self.inner
            .remove(id)
            .map(|(_, e)| e)
            .ok_or_else(|| IdentityError::EntryNotFound(id.to_string()))
    }

    /// List all entries.
    pub fn list(&self) -> Vec<RegistrationEntry> {
        self.inner.iter().map(|e| e.value().clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// List entries by parent SPIFFE ID.
    pub fn list_by_parent(&self, parent: &SpiffeId) -> Vec<RegistrationEntry> {
        self.inner
            .iter()
            .filter(|e| &e.value().parent_id == parent)
            .map(|e| e.value().clone())
            .collect()
    }

    /// List entries matching every supplied selector (set-subset match —
    /// equivalent to `pkg/server/datastore.ListRegistrationEntriesRequest.BySelectors:MATCH_SUBSET`).
    pub fn list_by_selectors(&self, selectors: &[Selector]) -> Vec<RegistrationEntry> {
        self.inner
            .iter()
            .filter(|e| selectors_match(selectors, &e.value().selectors))
            .map(|e| e.value().clone())
            .collect()
    }
}

/// SPIRE selector validation — `kind` non-empty and one of the known kinds.
pub fn validate_selector(sel: &Selector) -> Result<()> {
    if sel.kind.is_empty() {
        return Err(IdentityError::Internal("selector kind empty".into()));
    }
    if sel.value.is_empty() {
        return Err(IdentityError::Internal("selector value empty".into()));
    }
    match sel.kind.as_str() {
        "k8s" | "unix" | "docker" | "x509_pop" => Ok(()),
        other => Err(IdentityError::Internal(format!(
            "unknown selector kind: {}",
            other
        ))),
    }
}

/// Returns true when every selector in `required` appears in `available`.
///
/// Matches SPIRE `selectorSet.IncludesSet` semantics for `MATCH_SUBSET`.
pub fn selectors_match(required: &[Selector], available: &[Selector]) -> bool {
    required.iter().all(|r| {
        available
            .iter()
            .any(|a| a.kind == r.kind && a.value == r.value)
    })
}

/// Strict equality — `MATCH_EXACT` mode (same set, no extras).
pub fn selectors_equal(a: &[Selector], b: &[Selector]) -> bool {
    a.len() == b.len() && selectors_match(a, b) && selectors_match(b, a)
}

/// `MATCH_SUPERSET` — required ⊇ available.
pub fn selectors_superset(required: &[Selector], available: &[Selector]) -> bool {
    available.iter().all(|a| {
        required
            .iter()
            .any(|r| r.kind == a.kind && r.value == a.value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(spiffe: &str, parent: &str) -> RegistrationEntry {
        RegistrationEntry {
            spiffe_id: SpiffeId::new(spiffe),
            parent_id: SpiffeId::new(parent),
            ..Default::default()
        }
    }

    #[test]
    fn create_assigns_id() {
        let s = InMemoryEntryStore::new();
        let e = s
            .create(entry(
                "spiffe://example.org/svc",
                "spiffe://example.org/spire/agent/k8s_psat/node1",
            ))
            .unwrap();
        assert!(!e.id.is_empty());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn create_rejects_duplicate_id() {
        let s = InMemoryEntryStore::new();
        let mut e = entry(
            "spiffe://example.org/svc",
            "spiffe://example.org/spire/agent/k8s_psat/node1",
        );
        e.id = "fixed".to_string();
        s.create(e.clone()).unwrap();
        assert!(matches!(
            s.create(e),
            Err(IdentityError::EntryExists(_))
        ));
    }

    #[test]
    fn create_rejects_bad_spiffe_id() {
        let s = InMemoryEntryStore::new();
        let e = entry("not-a-spiffe-id", "spiffe://example.org/agent");
        assert!(matches!(s.create(e), Err(IdentityError::InvalidSpiffeId(_))));
    }

    #[test]
    fn create_rejects_bad_selector() {
        let s = InMemoryEntryStore::new();
        let mut e = entry(
            "spiffe://example.org/svc",
            "spiffe://example.org/spire/agent/k8s_psat/n",
        );
        e.selectors = vec![Selector::new("unknown-kind", "v")];
        assert!(matches!(s.create(e), Err(IdentityError::Internal(_))));
    }

    #[test]
    fn update_bumps_revision() {
        let s = InMemoryEntryStore::new();
        let mut e = s
            .create(entry(
                "spiffe://example.org/svc",
                "spiffe://example.org/spire/agent/k8s_psat/n",
            ))
            .unwrap();
        e.ttl_seconds = 7200;
        let u = s.update(e.clone()).unwrap();
        assert_eq!(u.ttl_seconds, 7200);
        assert_eq!(u.revision_number, 1);
    }

    #[test]
    fn delete_removes() {
        let s = InMemoryEntryStore::new();
        let e = s
            .create(entry(
                "spiffe://example.org/svc",
                "spiffe://example.org/spire/agent/k8s_psat/n",
            ))
            .unwrap();
        s.delete(&e.id).unwrap();
        assert!(s.get(&e.id).is_err());
        assert!(s.is_empty());
    }

    #[test]
    fn list_by_parent() {
        let s = InMemoryEntryStore::new();
        let p = "spiffe://example.org/spire/agent/k8s_psat/n";
        s.create(entry("spiffe://example.org/a", p)).unwrap();
        s.create(entry("spiffe://example.org/b", p)).unwrap();
        s.create(entry(
            "spiffe://example.org/c",
            "spiffe://example.org/spire/agent/k8s_psat/m",
        ))
        .unwrap();
        let by_n = s.list_by_parent(&SpiffeId::new(p));
        assert_eq!(by_n.len(), 2);
    }

    #[test]
    fn list_by_selectors_subset() {
        let s = InMemoryEntryStore::new();
        let mut e = entry(
            "spiffe://example.org/a",
            "spiffe://example.org/spire/agent/k8s_psat/n",
        );
        e.selectors = vec![
            Selector::new("k8s", "ns:default"),
            Selector::new("k8s", "sa:foo"),
        ];
        s.create(e).unwrap();
        let want = vec![Selector::new("k8s", "ns:default")];
        assert_eq!(s.list_by_selectors(&want).len(), 1);
        let want2 = vec![Selector::new("k8s", "ns:other")];
        assert_eq!(s.list_by_selectors(&want2).len(), 0);
    }

    #[test]
    fn selectors_helpers() {
        let a = vec![
            Selector::new("k8s", "ns:default"),
            Selector::new("k8s", "sa:foo"),
        ];
        let b = vec![Selector::new("k8s", "ns:default")];
        assert!(selectors_match(&b, &a));
        assert!(!selectors_equal(&a, &b));
        assert!(selectors_superset(&a, &b));
        assert!(!selectors_superset(&b, &a));
    }
}
