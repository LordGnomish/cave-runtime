// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sealed Secrets controller key-rotation state.
//!
//! Upstream: `pkg/controller/keys.go` (sealed-secrets v0.37.0). The full
//! reconciler is scope-cut to `cave-policy-controller (Phase 2)`; here we
//! model the in-memory keystore shape + the "current vs deprecated" key
//! selection logic that determines which RSA key is used for unseal.

use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

/// One sealing keypair entry.
///
/// `public_key_pem` is what gets handed to kubeseal CLI consumers.
/// `private_key_pem` is held server-side only.
#[derive(Debug, Clone)]
pub struct KeyEntry {
    pub id: String,
    pub public_key_pem: String,
    pub private_key_pem: String,
    pub created_at: DateTime<Utc>,
    pub deprecated: bool,
}

/// Controller keystore — current + deprecated keys (deprecated keys are still
/// retained for unseal of older SealedSecrets, but never used for new seals).
#[derive(Debug, Default)]
pub struct KeyStore {
    entries: BTreeMap<String, KeyEntry>,
    current_id: Option<String>,
}

impl KeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new key. If `current` is true, makes it the active key + marks
    /// the previously-active key deprecated.
    pub fn insert(&mut self, entry: KeyEntry, current: bool) {
        let id = entry.id.clone();
        self.entries.insert(id.clone(), entry);
        if current {
            if let Some(prev) = self.current_id.take() {
                if let Some(p) = self.entries.get_mut(&prev) {
                    p.deprecated = true;
                }
            }
            self.current_id = Some(id);
        }
    }

    pub fn current(&self) -> Option<&KeyEntry> {
        self.current_id.as_ref().and_then(|id| self.entries.get(id))
    }

    pub fn get(&self, id: &str) -> Option<&KeyEntry> {
        self.entries.get(id)
    }

    /// Return all candidate keys to try on decrypt, newest first.
    pub fn decryption_candidates(&self) -> Vec<&KeyEntry> {
        let mut v: Vec<&KeyEntry> = self.entries.values().collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    /// Mark an explicit ID deprecated (test-only entry-point).
    pub fn deprecate(&mut self, id: &str) -> bool {
        if let Some(e) = self.entries.get_mut(id) {
            e.deprecated = true;
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, ts_secs: i64) -> KeyEntry {
        KeyEntry {
            id: id.into(),
            public_key_pem: format!("PEM_PUB_{id}"),
            private_key_pem: format!("PEM_PRIV_{id}"),
            created_at: DateTime::<Utc>::from_timestamp(ts_secs, 0).unwrap(),
            deprecated: false,
        }
    }

    #[test]
    fn insert_first_current() {
        let mut s = KeyStore::new();
        s.insert(mk("k1", 100), true);
        assert_eq!(s.current().unwrap().id, "k1");
        assert!(!s.current().unwrap().deprecated);
    }

    #[test]
    fn rotate_deprecates_previous() {
        let mut s = KeyStore::new();
        s.insert(mk("k1", 100), true);
        s.insert(mk("k2", 200), true);
        assert_eq!(s.current().unwrap().id, "k2");
        assert!(s.get("k1").unwrap().deprecated);
    }

    #[test]
    fn decryption_candidates_newest_first() {
        let mut s = KeyStore::new();
        s.insert(mk("k1", 100), true);
        s.insert(mk("k2", 200), true);
        s.insert(mk("k3", 150), false);
        let cands = s.decryption_candidates();
        let ids: Vec<&str> = cands.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["k2", "k3", "k1"]);
    }

    #[test]
    fn deprecate_marks_existing() {
        let mut s = KeyStore::new();
        s.insert(mk("k1", 100), false);
        assert!(s.deprecate("k1"));
        assert!(s.get("k1").unwrap().deprecated);
        assert!(!s.deprecate("kx"));
    }

    #[test]
    fn empty_store_has_no_current() {
        let s = KeyStore::new();
        assert!(s.is_empty());
        assert!(s.current().is_none());
    }
}
