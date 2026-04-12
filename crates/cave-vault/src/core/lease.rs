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
