// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory probe store with RwLock-guarded HashMap.

use crate::models::UptimeProbe;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

/// Thread-safe in-memory store for `UptimeProbe` entries.
pub struct ProbeStore {
    inner: RwLock<HashMap<Uuid, UptimeProbe>>,
}

impl Default for ProbeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ProbeStore {
    /// Create an empty probe store.
    pub fn new() -> Self {
        ProbeStore {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Insert (or replace) a probe.
    pub fn insert(&self, probe: UptimeProbe) {
        self.inner.write().unwrap().insert(probe.id, probe);
    }

    /// Retrieve a probe by ID.
    pub fn get(&self, id: Uuid) -> Option<UptimeProbe> {
        self.inner.read().unwrap().get(&id).cloned()
    }

    /// List all probes.
    pub fn list(&self) -> Vec<UptimeProbe> {
        self.inner.read().unwrap().values().cloned().collect()
    }

    /// Update an existing probe. Returns `true` if found and updated.
    pub fn update(&self, probe: UptimeProbe) -> bool {
        let mut guard = self.inner.write().unwrap();
        if guard.contains_key(&probe.id) {
            guard.insert(probe.id, probe);
            true
        } else {
            false
        }
    }

    /// Delete a probe by ID. Returns `true` if it existed.
    pub fn delete(&self, id: Uuid) -> bool {
        self.inner.write().unwrap().remove(&id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProbeType, UptimeProbe};

    fn p(name: &str) -> UptimeProbe {
        UptimeProbe {
            id: Uuid::new_v4(),
            name: name.to_string(),
            target_url: "http://example.com".to_string(),
            probe_type: ProbeType::Http,
            interval_seconds: 30,
            timeout_ms: 3000,
            enabled: true,
        }
    }

    #[test]
    fn roundtrip_single() {
        let store = ProbeStore::new();
        let probe = p("rtt");
        let id = probe.id;
        store.insert(probe.clone());
        assert_eq!(store.get(id).unwrap().name, "rtt");
    }

    #[test]
    fn list_count() {
        let store = ProbeStore::new();
        for i in 0..5 {
            store.insert(p(&format!("p{i}")));
        }
        assert_eq!(store.list().len(), 5);
    }

    #[test]
    fn delete_removes_entry() {
        let store = ProbeStore::new();
        let probe = p("del");
        let id = probe.id;
        store.insert(probe);
        assert!(store.delete(id));
        assert!(store.get(id).is_none());
        assert!(!store.delete(id));
    }
}
