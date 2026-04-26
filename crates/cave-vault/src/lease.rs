//! Lease management and automatic revocation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::VaultError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseEntry {
    pub lease_id: String,
    pub path: String,
    pub renewable: bool,
    pub ttl: u64,
    pub max_ttl: u64,
    pub issued_at: DateTime<Utc>,
    pub expire_time: DateTime<Utc>,
    pub revoked: bool,
}

impl LeaseEntry {
    pub fn new(path: &str, ttl: u64, max_ttl: u64, renewable: bool) -> Self {
        let now = Utc::now();
        Self {
            lease_id: format!("{}/{}", path, Uuid::new_v4()),
            path: path.to_string(),
            renewable,
            ttl,
            max_ttl,
            issued_at: now,
            expire_time: now + chrono::Duration::seconds(ttl as i64),
            revoked: false,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.revoked || Utc::now() > self.expire_time
    }

    pub fn remaining_secs(&self) -> i64 {
        let rem = (self.expire_time - Utc::now()).num_seconds();
        rem.max(0)
    }
}

pub struct LeaseStore {
    pub leases: HashMap<String, LeaseEntry>,
}

impl LeaseStore {
    pub fn new() -> Self {
        Self {
            leases: HashMap::new(),
        }
    }

    pub fn create(&mut self, path: &str, ttl: u64, max_ttl: u64, renewable: bool) -> LeaseEntry {
        let entry = LeaseEntry::new(path, ttl, max_ttl, renewable);
        self.leases.insert(entry.lease_id.clone(), entry.clone());
        entry
    }

    pub fn lookup(&self, lease_id: &str) -> Result<&LeaseEntry, VaultError> {
        let entry = self
            .leases
            .get(lease_id)
            .ok_or_else(|| VaultError::NotFound(format!("lease {lease_id}")))?;
        if entry.is_expired() {
            return Err(VaultError::LeaseExpired);
        }
        Ok(entry)
    }

    pub fn renew(&mut self, lease_id: &str, increment: u64) -> Result<&LeaseEntry, VaultError> {
        let entry = self
            .leases
            .get_mut(lease_id)
            .ok_or_else(|| VaultError::NotFound(format!("lease {lease_id}")))?;
        if !entry.renewable {
            return Err(VaultError::InvalidRequest("lease is not renewable".into()));
        }
        if entry.revoked {
            return Err(VaultError::LeaseExpired);
        }
        let inc = if increment > 0 { increment } else { entry.ttl };
        let new_expire = Utc::now() + chrono::Duration::seconds(inc as i64);
        let max_expire = entry.issued_at + chrono::Duration::seconds(entry.max_ttl as i64);
        entry.expire_time = new_expire.min(max_expire);
        Ok(self.leases.get(lease_id).unwrap())
    }

    pub fn revoke(&mut self, lease_id: &str) -> Result<(), VaultError> {
        let entry = self
            .leases
            .get_mut(lease_id)
            .ok_or_else(|| VaultError::NotFound(format!("lease {lease_id}")))?;
        entry.revoked = true;
        Ok(())
    }

    pub fn revoke_prefix(&mut self, prefix: &str) -> usize {
        let mut count = 0;
        for entry in self.leases.values_mut() {
            if entry.path.starts_with(prefix) && !entry.revoked {
                entry.revoked = true;
                count += 1;
            }
        }
        count
    }

    pub fn prune_expired(&mut self) {
        self.leases.retain(|_, e| !e.is_expired());
    }

    pub fn list_by_prefix(&self, prefix: &str) -> Vec<&LeaseEntry> {
        self.leases
            .values()
            .filter(|e| e.path.starts_with(prefix) && !e.is_expired())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lease_creation_and_lookup() {
        let mut store = LeaseStore::new();
        let lease = store.create("secret/data/foo", 3600, 86400, true);
        assert!(!lease.is_expired());
        assert!(lease.remaining_secs() > 3500);
        store.lookup(&lease.lease_id).unwrap();
    }

    #[test]
    fn test_lease_revoke() {
        let mut store = LeaseStore::new();
        let lease = store.create("secret/data/bar", 3600, 86400, true);
        store.revoke(&lease.lease_id).unwrap();
        assert!(store.lookup(&lease.lease_id).is_err());
    }

    #[test]
    fn test_lease_renew() {
        let mut store = LeaseStore::new();
        let lease = store.create("auth/approle/x", 60, 86400, true);
        let renewed = store.renew(&lease.lease_id, 7200).unwrap();
        assert!(renewed.remaining_secs() > 60);
    }

    #[test]
    fn test_lease_revoke_prefix() {
        let mut store = LeaseStore::new();
        store.create("auth/userpass/alice", 3600, 86400, true);
        store.create("auth/userpass/bob", 3600, 86400, true);
        store.create("secret/data/x", 3600, 86400, false);
        let count = store.revoke_prefix("auth/userpass/");
        assert_eq!(count, 2);
    }
}
