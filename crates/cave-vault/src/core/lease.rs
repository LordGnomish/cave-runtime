// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::error::{VaultError, VaultResult};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub id: String,
    pub path: String,
    pub renewable: bool,
    pub ttl: i64,
    pub max_ttl: i64,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub token: String,
    pub mount: String,
}

impl Lease {
    pub fn new(path: &str, token: &str, mount: &str, ttl_secs: i64, max_ttl_secs: i64, renewable: bool) -> Self {
        let now = Utc::now();
        let id = format!("{}/{}", path, Uuid::new_v4());
        Lease {
            id,
            path: path.to_string(),
            renewable,
            ttl: ttl_secs,
            max_ttl: max_ttl_secs,
            created_at: now,
            expires_at: now + Duration::seconds(ttl_secs),
            token: token.to_string(),
            mount: mount.to_string(),
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn remaining_secs(&self) -> i64 {
        (self.expires_at - Utc::now()).num_seconds().max(0)
    }
}

#[derive(Default)]
pub struct LeaseStore {
    leases: HashMap<String, Lease>,
}

impl LeaseStore {
    pub fn put(&mut self, lease: Lease) -> String {
        let id = lease.id.clone();
        self.leases.insert(id.clone(), lease);
        id
    }

    pub fn get(&self, id: &str) -> Option<&Lease> {
        self.leases.get(id)
    }

    pub fn renew(&mut self, id: &str, increment_secs: i64) -> VaultResult<&Lease> {
        let lease = self.leases.get_mut(id).ok_or(VaultError::LeaseNotFound)?;
        if !lease.renewable {
            return Err(VaultError::InvalidRequest("lease is not renewable".into()));
        }
        let increment = increment_secs.min(lease.max_ttl);
        lease.expires_at = Utc::now() + Duration::seconds(increment);
        lease.ttl = increment;
        Ok(self.leases.get(id).unwrap())
    }

    pub fn revoke(&mut self, id: &str) -> bool {
        self.leases.remove(id).is_some()
    }

    pub fn revoke_prefix(&mut self, prefix: &str) -> usize {
        let to_remove: Vec<String> = self.leases.keys()
            .filter(|k| k.starts_with(prefix))
            .cloned().collect();
        let count = to_remove.len();
        for k in to_remove { self.leases.remove(&k); }
        count
    }

    pub fn revoke_by_token(&mut self, token: &str) -> usize {
        let to_remove: Vec<String> = self.leases.values()
            .filter(|l| l.token == token)
            .map(|l| l.id.clone()).collect();
        let count = to_remove.len();
        for k in to_remove { self.leases.remove(&k); }
        count
    }

    pub fn list_by_prefix(&self, prefix: &str) -> Vec<&Lease> {
        self.leases.values().filter(|l| l.path.starts_with(prefix)).collect()
    }

    pub fn purge_expired(&mut self) -> usize {
        let expired: Vec<String> = self.leases.values()
            .filter(|l| l.is_expired())
            .map(|l| l.id.clone()).collect();
        let count = expired.len();
        for k in expired { self.leases.remove(&k); }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lease(path: &str, ttl: i64, renewable: bool) -> Lease {
        Lease::new(path, "tok-test", "secret/", ttl, ttl, renewable)
    }

    #[test]
    fn test_lease_creation_and_lookup() {
        let mut store = LeaseStore::default();
        let lease = make_lease("secret/data/foo", 60, true);
        let id = store.put(lease);
        assert!(store.get(&id).is_some());
        assert_eq!(store.get(&id).unwrap().path, "secret/data/foo");
        assert_eq!(store.get(&id).unwrap().ttl, 60);
    }

    #[test]
    fn test_lease_renew() {
        let mut store = LeaseStore::default();
        let id = store.put(make_lease("secret/data/x", 30, true));
        let renewed = store.renew(&id, 20).unwrap();
        assert_eq!(renewed.ttl, 20);
    }

    #[test]
    fn test_lease_renew_non_renewable_fails() {
        let mut store = LeaseStore::default();
        let id = store.put(make_lease("secret/data/y", 30, false));
        assert!(store.renew(&id, 20).is_err());
    }

    #[test]
    fn test_lease_revoke() {
        let mut store = LeaseStore::default();
        let id = store.put(make_lease("secret/data/z", 30, true));
        assert!(store.revoke(&id));
        assert!(store.get(&id).is_none());
        assert!(!store.revoke(&id)); // second revoke is a no-op
    }

    #[test]
    fn test_lease_revoke_by_token() {
        let mut store = LeaseStore::default();
        store.put(Lease::new("a", "tok-1", "m/", 30, 30, true));
        store.put(Lease::new("b", "tok-1", "m/", 30, 30, true));
        store.put(Lease::new("c", "tok-2", "m/", 30, 30, true));
        let removed = store.revoke_by_token("tok-1");
        assert_eq!(removed, 2);
        assert_eq!(store.list_by_prefix("").len(), 1);
    }

    #[test]
    fn test_lease_revoke_prefix() {
        let mut store = LeaseStore::default();
        store.put(Lease::new("kv/data/a", "t", "m/", 30, 30, true));
        store.put(Lease::new("kv/data/b", "t", "m/", 30, 30, true));
        store.put(Lease::new("other/x", "t", "m/", 30, 30, true));
        let removed = store.revoke_prefix("kv/data");
        assert_eq!(removed, 2);
        assert_eq!(store.list_by_prefix("").len(), 1);
    }

    #[test]
    fn test_lease_remaining_secs_positive_for_future_expiry() {
        let lease = make_lease("p", 100, true);
        assert!(lease.remaining_secs() > 0);
        assert!(lease.remaining_secs() <= 100);
    }
}
