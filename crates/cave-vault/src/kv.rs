//! KV Secrets Engine — v1 (unversioned) and v2 (versioned with soft-delete).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::VaultError;

// ── KV v2 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVVersionData {
    pub data: HashMap<String, serde_json::Value>,
    pub created_time: DateTime<Utc>,
    pub deletion_time: Option<DateTime<Utc>>,
    pub destroyed: bool,
    pub version: u32,
}

impl KVVersionData {
    pub fn is_readable(&self) -> bool {
        !self.destroyed && self.deletion_time.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVMetadata {
    pub created_time: DateTime<Utc>,
    pub current_version: u32,
    pub max_versions: u32,
    pub oldest_version: u32,
    pub updated_time: DateTime<Utc>,
    pub custom_metadata: HashMap<String, String>,
    pub cas_required: bool,
    pub versions: HashMap<u32, KVVersionMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVVersionMeta {
    pub created_time: DateTime<Utc>,
    pub deletion_time: Option<DateTime<Utc>>,
    pub destroyed: bool,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVEntry {
    pub versions: HashMap<u32, KVVersionData>,
    pub current_version: u32,
    pub max_versions: u32,
    pub custom_metadata: HashMap<String, String>,
    pub cas_required: bool,
    pub created_time: DateTime<Utc>,
    pub updated_time: DateTime<Utc>,
}

impl KVEntry {
    pub fn new(max_versions: u32) -> Self {
        Self {
            versions: HashMap::new(),
            current_version: 0,
            max_versions: max_versions.max(1),
            custom_metadata: HashMap::new(),
            cas_required: false,
            created_time: Utc::now(),
            updated_time: Utc::now(),
        }
    }

    /// Write a new version (optionally with CAS check-and-set version).
    pub fn put(
        &mut self,
        data: HashMap<String, serde_json::Value>,
        cas: Option<u32>,
    ) -> Result<u32, VaultError> {
        if self.cas_required {
            let expected = cas.ok_or_else(|| {
                VaultError::InvalidRequest("CAS value required but not provided".into())
            })?;
            if expected != self.current_version {
                return Err(VaultError::InvalidRequest(format!(
                    "CAS mismatch: expected {}, current {}",
                    expected, self.current_version
                )));
            }
        }

        self.current_version += 1;
        let version = self.current_version;
        self.versions.insert(
            version,
            KVVersionData {
                data,
                created_time: Utc::now(),
                deletion_time: None,
                destroyed: false,
                version,
            },
        );
        self.updated_time = Utc::now();
        self.prune();
        Ok(version)
    }

    /// Read a specific version (or current if None).
    pub fn get(&self, version: Option<u32>) -> Result<&KVVersionData, VaultError> {
        let v = version.unwrap_or(self.current_version);
        if v == 0 {
            return Err(VaultError::NotFound("no versions written yet".into()));
        }
        let entry = self
            .versions
            .get(&v)
            .ok_or_else(|| VaultError::NotFound(format!("version {v}")))?;
        if entry.destroyed {
            return Err(VaultError::SecretDestroyed);
        }
        if entry.deletion_time.is_some() {
            return Err(VaultError::SecretDeleted);
        }
        Ok(entry)
    }

    /// Soft-delete the current (latest) version.
    pub fn delete_latest(&mut self) {
        let v = self.current_version;
        if let Some(entry) = self.versions.get_mut(&v) {
            if !entry.destroyed {
                entry.deletion_time = Some(Utc::now());
            }
        }
    }

    /// Soft-delete specific versions.
    pub fn soft_delete(&mut self, versions: &[u32]) {
        let now = Utc::now();
        for &v in versions {
            if let Some(entry) = self.versions.get_mut(&v) {
                if !entry.destroyed {
                    entry.deletion_time = Some(now);
                }
            }
        }
    }

    /// Undelete soft-deleted versions.
    pub fn undelete(&mut self, versions: &[u32]) {
        for &v in versions {
            if let Some(entry) = self.versions.get_mut(&v) {
                if !entry.destroyed {
                    entry.deletion_time = None;
                }
            }
        }
    }

    /// Permanently destroy versions (zeroes data, marks destroyed).
    pub fn destroy(&mut self, versions: &[u32]) {
        for &v in versions {
            if let Some(entry) = self.versions.get_mut(&v) {
                entry.destroyed = true;
                entry.data.clear();
                entry.deletion_time = Some(Utc::now());
            }
        }
    }

    /// Return metadata without data payloads.
    pub fn metadata(&self) -> KVMetadata {
        let oldest = self.versions.keys().copied().min().unwrap_or(0);
        let versions_meta: HashMap<u32, KVVersionMeta> = self
            .versions
            .iter()
            .map(|(&v, d)| {
                (
                    v,
                    KVVersionMeta {
                        created_time: d.created_time,
                        deletion_time: d.deletion_time,
                        destroyed: d.destroyed,
                        version: d.version,
                    },
                )
            })
            .collect();

        KVMetadata {
            created_time: self.created_time,
            current_version: self.current_version,
            max_versions: self.max_versions,
            oldest_version: oldest,
            updated_time: self.updated_time,
            custom_metadata: self.custom_metadata.clone(),
            cas_required: self.cas_required,
            versions: versions_meta,
        }
    }

    fn prune(&mut self) {
        if self.versions.len() as u32 > self.max_versions {
            let keep_from = self.current_version.saturating_sub(self.max_versions - 1);
            self.versions.retain(|&v, _| v >= keep_from);
        }
    }
}

// ── KV v1 (unversioned) ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVV1Entry {
    pub data: HashMap<String, serde_json::Value>,
    pub created_time: DateTime<Utc>,
    pub updated_time: DateTime<Utc>,
}

impl KVV1Entry {
    pub fn new(data: HashMap<String, serde_json::Value>) -> Self {
        let now = Utc::now();
        Self {
            data,
            created_time: now,
            updated_time: now,
        }
    }

    pub fn update(&mut self, data: HashMap<String, serde_json::Value>) {
        self.data = data;
        self.updated_time = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_data(v: &str) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert("value".into(), json!(v));
        m
    }

    #[test]
    fn test_kv_v2_put_get() {
        let mut entry = KVEntry::new(10);
        let v = entry.put(sample_data("hello"), None).unwrap();
        assert_eq!(v, 1);
        let data = entry.get(None).unwrap();
        assert_eq!(data.data["value"], json!("hello"));
    }

    #[test]
    fn test_kv_v2_versioning() {
        let mut entry = KVEntry::new(10);
        entry.put(sample_data("v1"), None).unwrap();
        entry.put(sample_data("v2"), None).unwrap();
        entry.put(sample_data("v3"), None).unwrap();

        assert_eq!(entry.current_version, 3);
        assert_eq!(entry.get(Some(1)).unwrap().data["value"], json!("v1"));
        assert_eq!(entry.get(Some(2)).unwrap().data["value"], json!("v2"));
        assert_eq!(entry.get(None).unwrap().data["value"], json!("v3"));
    }

    #[test]
    fn test_kv_v2_soft_delete_undelete() {
        let mut entry = KVEntry::new(10);
        entry.put(sample_data("hello"), None).unwrap();

        entry.soft_delete(&[1]);
        assert!(matches!(entry.get(Some(1)), Err(VaultError::SecretDeleted)));

        entry.undelete(&[1]);
        assert!(entry.get(Some(1)).is_ok());
    }

    #[test]
    fn test_kv_v2_destroy() {
        let mut entry = KVEntry::new(10);
        entry.put(sample_data("hello"), None).unwrap();
        entry.destroy(&[1]);
        assert!(matches!(entry.get(Some(1)), Err(VaultError::SecretDestroyed)));
        // data is cleared
        assert!(entry.versions[&1].data.is_empty());
    }

    #[test]
    fn test_kv_v2_metadata() {
        let mut entry = KVEntry::new(10);
        entry.put(sample_data("a"), None).unwrap();
        entry.put(sample_data("b"), None).unwrap();
        let meta = entry.metadata();
        assert_eq!(meta.current_version, 2);
        assert_eq!(meta.versions.len(), 2);
    }

    #[test]
    fn test_kv_v2_max_versions_prune() {
        let mut entry = KVEntry::new(3);
        for i in 0..5 {
            entry.put(sample_data(&i.to_string()), None).unwrap();
        }
        // Only 3 versions kept
        assert!(entry.versions.len() <= 3);
        // Latest still readable
        assert!(entry.get(None).is_ok());
    }

    #[test]
    fn test_kv_v2_cas() {
        let mut entry = KVEntry::new(10);
        entry.cas_required = true;
        // First write must match current_version=0
        entry.put(sample_data("v1"), Some(0)).unwrap();
        // CAS mismatch
        let err = entry.put(sample_data("v2"), Some(0));
        assert!(err.is_err());
        // Correct CAS
        entry.put(sample_data("v2"), Some(1)).unwrap();
    }

    #[test]
    fn test_kv_v1_crud() {
        let mut store: HashMap<String, KVV1Entry> = HashMap::new();
        let data = sample_data("password123");
        store.insert("secret/db".into(), KVV1Entry::new(data.clone()));

        let entry = store.get("secret/db").unwrap();
        assert_eq!(entry.data["value"], json!("password123"));

        // Update
        let mut updated = entry.clone();
        updated.update(sample_data("newpassword"));
        store.insert("secret/db".into(), updated);

        assert_eq!(store["secret/db"].data["value"], json!("newpassword"));

        // Delete
        store.remove("secret/db");
        assert!(!store.contains_key("secret/db"));
    }

    #[test]
    fn test_kv_v2_list() {
        let mut store: HashMap<String, KVEntry> = HashMap::new();
        for key in &["secret/db/pass", "secret/db/user", "secret/api/key"] {
            let mut e = KVEntry::new(10);
            e.put(sample_data("x"), None).unwrap();
            store.insert(key.to_string(), e);
        }

        let prefix = "secret/db/";
        let mut keys: Vec<String> = store
            .keys()
            .filter(|k| k.starts_with(prefix))
            .map(|k| k.trim_start_matches(prefix).to_string())
            .collect();
        keys.sort();
        assert_eq!(keys, vec!["pass", "user"]);
    }
}
