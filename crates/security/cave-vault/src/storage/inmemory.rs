// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Process-local backend — `HashMap` behind a single `RwLock`.
//! The default for tests / dev / single-process demo, equivalent
//! to OpenBao's `physical/inmem`. No persistence, no validation
//! beyond the trait contract (path-traversal can't escape a
//! `HashMap`).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::{Backend, StorageError, collect_one_level_children};

/// HashMap-backed `Backend`. `Arc<RwLock<…>>` so the same instance
/// can be `clone()`-ed across handlers; `&self` on the trait still
/// works because the lock provides interior mutability.
#[derive(Default, Clone)]
pub struct InMemoryBackend {
    inner: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl InMemoryBackend {
    /// New empty backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Live key count — handy for tests and metrics, not part of
    /// the upstream Vault contract.
    pub fn len(&self) -> usize {
        self.inner.read().expect("poisoned").len()
    }

    /// `true` if no keys stored.
    pub fn is_empty(&self) -> bool {
        self.inner.read().expect("poisoned").is_empty()
    }
}

impl Backend for InMemoryBackend {
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let g = self.inner.read().expect("poisoned");
        Ok(g.get(path).cloned())
    }

    fn put(&self, path: &str, value: Vec<u8>) -> Result<(), StorageError> {
        let mut g = self.inner.write().expect("poisoned");
        g.insert(path.to_string(), value);
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<(), StorageError> {
        let mut g = self.inner.write().expect("poisoned");
        g.remove(path);
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let g = self.inner.read().expect("poisoned");
        let keys: Vec<&str> = g.keys().map(|s| s.as_str()).collect();
        Ok(collect_one_level_children(prefix, keys.into_iter()))
    }

    fn exists(&self, path: &str) -> Result<bool, StorageError> {
        let g = self.inner.read().expect("poisoned");
        Ok(g.contains_key(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn put_then_get_round_trips() {
        let b = InMemoryBackend::new();
        b.put("kv/a", b"hello".to_vec()).unwrap();
        assert_eq!(b.get("kv/a").unwrap(), Some(b"hello".to_vec()));
    }

    #[test]
    fn get_missing_returns_none_not_err() {
        let b = InMemoryBackend::new();
        assert_eq!(b.get("absent").unwrap(), None);
    }

    #[test]
    fn put_overwrites_previous_value() {
        let b = InMemoryBackend::new();
        b.put("k", b"v1".to_vec()).unwrap();
        b.put("k", b"v2".to_vec()).unwrap();
        assert_eq!(b.get("k").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn delete_removes_and_is_idempotent() {
        let b = InMemoryBackend::new();
        b.put("k", b"v".to_vec()).unwrap();
        b.delete("k").unwrap();
        assert_eq!(b.get("k").unwrap(), None);
        // Second delete of same missing key must not error.
        b.delete("k").unwrap();
        b.delete("never-existed").unwrap();
    }

    #[test]
    fn exists_reflects_storage() {
        let b = InMemoryBackend::new();
        assert!(!b.exists("k").unwrap());
        b.put("k", b"v".to_vec()).unwrap();
        assert!(b.exists("k").unwrap());
        b.delete("k").unwrap();
        assert!(!b.exists("k").unwrap());
    }

    #[test]
    fn list_returns_one_level_children() {
        let b = InMemoryBackend::new();
        b.put("kv/a", b"1".to_vec()).unwrap();
        b.put("kv/b/x", b"2".to_vec()).unwrap();
        b.put("kv/b/y", b"3".to_vec()).unwrap();
        b.put("other/z", b"4".to_vec()).unwrap();
        let mut got = b.list("kv").unwrap();
        got.sort();
        assert_eq!(got, vec!["a", "b/"]);
    }

    #[test]
    fn list_empty_prefix_returns_top_level() {
        let b = InMemoryBackend::new();
        b.put("a", b"1".to_vec()).unwrap();
        b.put("b/x", b"2".to_vec()).unwrap();
        let mut got = b.list("").unwrap();
        got.sort();
        assert_eq!(got, vec!["a", "b/"]);
    }

    #[test]
    fn clone_shares_underlying_store() {
        let a = InMemoryBackend::new();
        let b = a.clone();
        a.put("k", b"v".to_vec()).unwrap();
        assert_eq!(b.get("k").unwrap(), Some(b"v".to_vec()));
        assert_eq!(b.len(), 1);
        assert!(!a.is_empty());
    }

    #[test]
    fn concurrent_writes_dont_panic_and_all_land() {
        let b = InMemoryBackend::new();
        let mut handles = Vec::new();
        for i in 0..16 {
            let bc = b.clone();
            handles.push(thread::spawn(move || {
                bc.put(&format!("k/{i}"), vec![i as u8]).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(b.len(), 16);
        for i in 0..16 {
            assert_eq!(b.get(&format!("k/{i}")).unwrap(), Some(vec![i as u8]));
        }
    }
}
