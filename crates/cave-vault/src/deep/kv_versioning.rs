//! KV-v2 versioning state machine.
//!
//! Mirrors `vault/builtin/logical/kv/path_data.go`:
//!
//! * `put` creates a new version; the previous becomes `version - 1`.
//! * `delete` sets `deletion_time` on a version → soft-delete.
//! * `undelete` clears `deletion_time`.
//! * `destroy` hard-deletes a specific version (cannot be undone).
//! * `cas` requires a `cas` parameter equal to the current version.
//! * `versions` are capped by `max_versions` per metadata.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionMeta {
    pub version: u64,
    pub created_at: DateTime<Utc>,
    pub deletion_time: Option<DateTime<Utc>>,
    pub destroyed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub meta: VersionMeta,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SecretMetadata {
    pub current_version: u64,
    pub max_versions: u32,
    pub cas_required: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct KvStore {
    secrets: BTreeMap<String, (SecretMetadata, BTreeMap<u64, Version>)>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum KvError {
    #[error("secret not found: {0}")]
    NotFound(String),
    #[error("version not found: {0} v{1}")]
    VersionNotFound(String, u64),
    #[error("cas mismatch: expected {0}, got {1}")]
    CasMismatch(u64, u64),
    #[error("destroyed: cannot operate on hard-deleted version")]
    Destroyed,
}

impl KvStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, key: &str, data: Vec<u8>, cas: Option<u64>, max_versions: u32, now: DateTime<Utc>) -> Result<u64, KvError> {
        let entry = self
            .secrets
            .entry(key.to_string())
            .or_insert_with(|| {
                (
                    SecretMetadata {
                        current_version: 0,
                        max_versions,
                        cas_required: false,
                        created_at: now,
                        updated_at: now,
                    },
                    BTreeMap::new(),
                )
            });
        let (meta, versions) = entry;
        if let Some(c) = cas {
            if c != meta.current_version {
                return Err(KvError::CasMismatch(meta.current_version, c));
            }
        } else if meta.cas_required {
            return Err(KvError::CasMismatch(meta.current_version, 0));
        }
        meta.current_version += 1;
        meta.updated_at = now;
        let version = meta.current_version;
        versions.insert(
            version,
            Version {
                meta: VersionMeta {
                    version,
                    created_at: now,
                    deletion_time: None,
                    destroyed: false,
                },
                data,
            },
        );
        // Trim oldest versions over max_versions cap
        while versions.len() > meta.max_versions as usize {
            if let Some((&oldest, _)) = versions.iter().next() {
                versions.remove(&oldest);
            }
        }
        Ok(version)
    }

    pub fn require_cas(&mut self, key: &str, required: bool) -> Result<(), KvError> {
        let (meta, _) = self
            .secrets
            .get_mut(key)
            .ok_or_else(|| KvError::NotFound(key.into()))?;
        meta.cas_required = required;
        Ok(())
    }

    pub fn get(&self, key: &str, version: Option<u64>) -> Result<&Version, KvError> {
        let (meta, versions) = self
            .secrets
            .get(key)
            .ok_or_else(|| KvError::NotFound(key.into()))?;
        let v = version.unwrap_or(meta.current_version);
        let entry = versions
            .get(&v)
            .ok_or_else(|| KvError::VersionNotFound(key.into(), v))?;
        if entry.meta.destroyed {
            return Err(KvError::Destroyed);
        }
        Ok(entry)
    }

    pub fn delete(&mut self, key: &str, version: u64, now: DateTime<Utc>) -> Result<(), KvError> {
        let (_, versions) = self
            .secrets
            .get_mut(key)
            .ok_or_else(|| KvError::NotFound(key.into()))?;
        let v = versions
            .get_mut(&version)
            .ok_or_else(|| KvError::VersionNotFound(key.into(), version))?;
        if v.meta.destroyed {
            return Err(KvError::Destroyed);
        }
        v.meta.deletion_time = Some(now);
        Ok(())
    }

    pub fn undelete(&mut self, key: &str, version: u64) -> Result<(), KvError> {
        let (_, versions) = self
            .secrets
            .get_mut(key)
            .ok_or_else(|| KvError::NotFound(key.into()))?;
        let v = versions
            .get_mut(&version)
            .ok_or_else(|| KvError::VersionNotFound(key.into(), version))?;
        if v.meta.destroyed {
            return Err(KvError::Destroyed);
        }
        v.meta.deletion_time = None;
        Ok(())
    }

    pub fn destroy(&mut self, key: &str, version: u64) -> Result<(), KvError> {
        let (_, versions) = self
            .secrets
            .get_mut(key)
            .ok_or_else(|| KvError::NotFound(key.into()))?;
        let v = versions
            .get_mut(&version)
            .ok_or_else(|| KvError::VersionNotFound(key.into(), version))?;
        v.meta.destroyed = true;
        v.data.clear();
        Ok(())
    }

    pub fn list(&self, prefix: &str) -> Vec<String> {
        self.secrets
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    pub fn metadata(&self, key: &str) -> Option<&SecretMetadata> {
        self.secrets.get(key).map(|(m, _)| m)
    }

    pub fn versions(&self, key: &str) -> Option<Vec<&VersionMeta>> {
        self.secrets
            .get(key)
            .map(|(_, vs)| vs.values().map(|v| &v.meta).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_first_version_returns_one() {
        let mut s = KvStore::new();
        let v = s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        assert_eq!(v, 1);
    }

    #[test]
    fn put_increments_version() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        let v = s.put("k", b"v2".to_vec(), None, 10, Utc::now()).unwrap();
        assert_eq!(v, 2);
    }

    #[test]
    fn put_cas_match_succeeds() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        let v = s.put("k", b"v2".to_vec(), Some(1), 10, Utc::now()).unwrap();
        assert_eq!(v, 2);
    }

    #[test]
    fn put_cas_mismatch_errors() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        let err = s.put("k", b"v2".to_vec(), Some(99), 10, Utc::now()).unwrap_err();
        assert!(matches!(err, KvError::CasMismatch(_, _)));
    }

    #[test]
    fn cas_required_blocks_no_cas() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        s.require_cas("k", true).unwrap();
        let err = s.put("k", b"v2".to_vec(), None, 10, Utc::now()).unwrap_err();
        assert!(matches!(err, KvError::CasMismatch(_, _)));
    }

    #[test]
    fn get_returns_current_version_by_default() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        s.put("k", b"v2".to_vec(), None, 10, Utc::now()).unwrap();
        assert_eq!(s.get("k", None).unwrap().data, b"v2");
    }

    #[test]
    fn get_specific_version() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        s.put("k", b"v2".to_vec(), None, 10, Utc::now()).unwrap();
        assert_eq!(s.get("k", Some(1)).unwrap().data, b"v1");
    }

    #[test]
    fn get_unknown_secret_errors() {
        let s = KvStore::new();
        let err = s.get("nope", None).unwrap_err();
        assert!(matches!(err, KvError::NotFound(_)));
    }

    #[test]
    fn get_unknown_version_errors() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        let err = s.get("k", Some(99)).unwrap_err();
        assert!(matches!(err, KvError::VersionNotFound(_, _)));
    }

    #[test]
    fn delete_marks_deletion_time() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        let now = Utc::now();
        s.delete("k", 1, now).unwrap();
        assert_eq!(s.versions("k").unwrap()[0].deletion_time, Some(now));
    }

    #[test]
    fn undelete_clears_deletion_time() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        s.delete("k", 1, Utc::now()).unwrap();
        s.undelete("k", 1).unwrap();
        assert!(s.versions("k").unwrap()[0].deletion_time.is_none());
    }

    #[test]
    fn destroy_cannot_be_undone() {
        let mut s = KvStore::new();
        s.put("k", b"v1".to_vec(), None, 10, Utc::now()).unwrap();
        s.destroy("k", 1).unwrap();
        let err = s.get("k", Some(1)).unwrap_err();
        assert!(matches!(err, KvError::Destroyed));
    }

    #[test]
    fn destroy_wipes_data() {
        let mut s = KvStore::new();
        s.put("k", b"secret".to_vec(), None, 10, Utc::now()).unwrap();
        s.destroy("k", 1).unwrap();
        // Data wiped — we can read meta but get() refuses.
        let v = s.versions("k").unwrap();
        assert!(v[0].destroyed);
    }

    #[test]
    fn max_versions_trims_oldest() {
        let mut s = KvStore::new();
        for i in 0..10 {
            s.put("k", format!("v{}", i).into_bytes(), None, 3, Utc::now()).unwrap();
        }
        let versions = s.versions("k").unwrap();
        assert_eq!(versions.len(), 3);
        let nums: Vec<u64> = versions.iter().map(|v| v.version).collect();
        assert_eq!(nums, vec![8, 9, 10]);
    }

    #[test]
    fn list_filters_by_prefix() {
        let mut s = KvStore::new();
        s.put("app/db/pw", vec![], None, 10, Utc::now()).unwrap();
        s.put("app/web/pw", vec![], None, 10, Utc::now()).unwrap();
        s.put("other/x", vec![], None, 10, Utc::now()).unwrap();
        let l = s.list("app/");
        assert_eq!(l.len(), 2);
    }
}
