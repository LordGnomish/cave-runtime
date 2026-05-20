// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lease lifecycle + GC interlock.

use super::resource::{Resource, ResourceKind};
use crate::content::store::LocalStore;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    #[error("lease {0} not found")]
    NotFound(String),
    #[error("lease {0} already exists")]
    AlreadyExists(String),
}

#[derive(Debug, Clone)]
pub struct Lease {
    pub id: String,
    pub created_at_unix: i64,
    pub ttl_seconds: Option<u64>,
    pub labels: HashMap<String, String>,
    pub resources: HashSet<Resource>,
}

impl Lease {
    pub fn is_expired(&self, now_unix: i64) -> bool {
        let Some(ttl) = self.ttl_seconds else {
            return false;
        };
        now_unix.saturating_sub(self.created_at_unix) as u64 >= ttl
    }
}

/// In-memory lease table.
///
/// Optionally wired to a [`LocalStore`] via [`LeaseManager::with_store`].
/// When wired, content-kind resources mark the store's in-use map on
/// add and clear it on delete, blocking [`LocalStore::delete`] from
/// reaping leased blobs.
pub struct LeaseManager {
    leases: Arc<RwLock<HashMap<String, Lease>>>,
    store: Option<Arc<LocalStore>>,
}

impl LeaseManager {
    pub fn new() -> Self {
        Self {
            leases: Arc::new(RwLock::new(HashMap::new())),
            store: None,
        }
    }

    /// Wire a content store so that content-kind resources block the
    /// store's `delete()` path while held.
    pub fn with_store(store: Arc<LocalStore>) -> Self {
        Self {
            leases: Arc::new(RwLock::new(HashMap::new())),
            store: Some(store),
        }
    }

    pub fn create(
        &self,
        id: impl Into<String>,
        ttl_seconds: Option<u64>,
        labels: HashMap<String, String>,
    ) -> Result<Lease, LeaseError> {
        let id = id.into();
        let mut leases = self.leases.write().unwrap();
        if leases.contains_key(&id) {
            return Err(LeaseError::AlreadyExists(id));
        }
        let lease = Lease {
            id: id.clone(),
            created_at_unix: now_unix(),
            ttl_seconds,
            labels,
            resources: HashSet::new(),
        };
        leases.insert(id, lease.clone());
        Ok(lease)
    }

    pub fn get(&self, id: &str) -> Result<Lease, LeaseError> {
        self.leases
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| LeaseError::NotFound(id.into()))
    }

    pub fn list(&self) -> Vec<Lease> {
        self.leases.read().unwrap().values().cloned().collect()
    }

    /// Add a resource to a lease. For content resources, also marks
    /// the digest in-use on the wired store (if any).
    pub fn add_resource(&self, lease_id: &str, resource: Resource) -> Result<(), LeaseError> {
        let mut leases = self.leases.write().unwrap();
        let lease = leases
            .get_mut(lease_id)
            .ok_or_else(|| LeaseError::NotFound(lease_id.into()))?;
        if resource.kind == ResourceKind::Content {
            if let (Some(store), Some(digest)) = (&self.store, resource.content_digest()) {
                store.mark_in_use(&digest, lease_id.to_string());
            }
        }
        lease.resources.insert(resource);
        Ok(())
    }

    /// Remove a single resource from a lease. Does not clear the
    /// store's in-use mapping; on a multi-resource lease that mapping
    /// reflects the lease itself, not the individual resource.
    pub fn remove_resource(&self, lease_id: &str, resource: &Resource) -> Result<(), LeaseError> {
        let mut leases = self.leases.write().unwrap();
        let lease = leases
            .get_mut(lease_id)
            .ok_or_else(|| LeaseError::NotFound(lease_id.into()))?;
        lease.resources.remove(resource);
        Ok(())
    }

    /// Delete a lease, releasing every in-use reference it held.
    pub fn delete(&self, lease_id: &str) -> Result<(), LeaseError> {
        let removed = self.leases.write().unwrap().remove(lease_id);
        if removed.is_none() {
            return Err(LeaseError::NotFound(lease_id.into()));
        }
        if let Some(store) = &self.store {
            store.release_lease(lease_id);
        }
        Ok(())
    }

    /// Reap leases whose TTL has elapsed. Returns the list of IDs
    /// that were deleted.
    pub fn reap_expired(&self) -> Vec<String> {
        let now = now_unix();
        let expired: Vec<String> = self
            .leases
            .read()
            .unwrap()
            .values()
            .filter(|l| l.is_expired(now))
            .map(|l| l.id.clone())
            .collect();
        for id in &expired {
            let _ = self.delete(id);
        }
        expired
    }

    /// Set of content digests currently held by any lease — useful
    /// for the GC walker.
    pub fn live_content(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for lease in self.leases.read().unwrap().values() {
            for r in &lease.resources {
                if r.kind == ResourceKind::Content {
                    out.insert(r.id.clone());
                }
            }
        }
        out
    }
}

impl Default for LeaseManager {
    fn default() -> Self {
        Self::new()
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::digest::{Digest, DigestAlgorithm};
    use crate::content::store::{ContentStore, LocalStore, StoreError};
    use std::io::Write;
    use tempfile::TempDir;

    fn put_blob(store: &LocalStore, bytes: &[u8]) -> Digest {
        let expected = Digest::compute(DigestAlgorithm::Sha256, bytes);
        let mut w = store
            .writer(format!("r-{}", &expected.hex()[..8]), expected.clone())
            .unwrap();
        w.write_all(bytes).unwrap();
        w.commit().unwrap();
        expected
    }

    #[test]
    fn create_and_get_round_trips() {
        let m = LeaseManager::new();
        let l = m.create("lease-a", None, HashMap::new()).unwrap();
        let got = m.get("lease-a").unwrap();
        assert_eq!(got.id, l.id);
    }

    #[test]
    fn create_duplicate_id_refused() {
        let m = LeaseManager::new();
        m.create("x", None, HashMap::new()).unwrap();
        let err = m.create("x", None, HashMap::new()).unwrap_err();
        assert!(matches!(err, LeaseError::AlreadyExists(_)));
    }

    #[test]
    fn get_unknown_lease_errors() {
        let m = LeaseManager::new();
        assert!(matches!(
            m.get("nope").unwrap_err(),
            LeaseError::NotFound(_)
        ));
    }

    #[test]
    fn add_then_remove_resource() {
        let m = LeaseManager::new();
        m.create("L", None, HashMap::new()).unwrap();
        let r = Resource::snapshot("snap-1");
        m.add_resource("L", r.clone()).unwrap();
        assert_eq!(m.get("L").unwrap().resources.len(), 1);
        m.remove_resource("L", &r).unwrap();
        assert_eq!(m.get("L").unwrap().resources.len(), 0);
    }

    #[test]
    fn delete_lease_clears_table() {
        let m = LeaseManager::new();
        m.create("d", None, HashMap::new()).unwrap();
        m.delete("d").unwrap();
        assert!(matches!(m.get("d").unwrap_err(), LeaseError::NotFound(_)));
    }

    #[test]
    fn lease_blocks_content_delete() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(LocalStore::open(dir.path()).unwrap());
        let m = LeaseManager::with_store(store.clone());
        let digest = put_blob(&store, b"protected");
        m.create("hold", None, HashMap::new()).unwrap();
        m.add_resource("hold", Resource::content(&digest)).unwrap();
        // Store should refuse delete while lease holds it.
        let err = store.delete(&digest).unwrap_err();
        assert!(matches!(err, StoreError::InUse(_)));
        // Delete the lease → blob is reapable.
        m.delete("hold").unwrap();
        store.delete(&digest).unwrap();
    }

    #[test]
    fn live_content_lists_held_digests() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(LocalStore::open(dir.path()).unwrap());
        let m = LeaseManager::with_store(store.clone());
        let d1 = put_blob(&store, b"one");
        let d2 = put_blob(&store, b"two");
        m.create("la", None, HashMap::new()).unwrap();
        m.add_resource("la", Resource::content(&d1)).unwrap();
        m.add_resource("la", Resource::content(&d2)).unwrap();
        let live = m.live_content();
        assert!(live.contains(&d1.to_string()));
        assert!(live.contains(&d2.to_string()));
    }

    #[test]
    fn reap_expired_removes_old_leases() {
        let m = LeaseManager::new();
        m.create("short", Some(0), HashMap::new()).unwrap();
        m.create("forever", None, HashMap::new()).unwrap();
        // Force `short` to look expired (created_at older than now-ttl)
        // by manipulating the table directly. ttl=0 means "expires at
        // creation"; reap_expired sees it as already past.
        let expired = m.reap_expired();
        assert_eq!(expired, vec!["short".to_string()]);
        assert!(m.get("forever").is_ok());
    }

    #[test]
    fn list_returns_every_lease() {
        let m = LeaseManager::new();
        m.create("a", None, HashMap::new()).unwrap();
        m.create("b", None, HashMap::new()).unwrap();
        m.create("c", None, HashMap::new()).unwrap();
        assert_eq!(m.list().len(), 3);
    }

    #[test]
    fn labels_persist_on_lease() {
        let m = LeaseManager::new();
        let mut labels = HashMap::new();
        labels.insert("kind".into(), "pull".into());
        m.create("labeled", None, labels.clone()).unwrap();
        assert_eq!(m.get("labeled").unwrap().labels, labels);
    }

    #[test]
    fn lease_without_store_skips_in_use_tracking() {
        let dir = TempDir::new().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        // LeaseManager *without* a wired store.
        let m = LeaseManager::new();
        let d = put_blob(&store, b"unguarded");
        m.create("nope", None, HashMap::new()).unwrap();
        m.add_resource("nope", Resource::content(&d)).unwrap();
        // Store has no in-use entry → delete proceeds.
        store.delete(&d).unwrap();
    }
}
