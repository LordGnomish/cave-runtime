// SPDX-License-Identifier: AGPL-3.0-or-later
//! KV secrets engine v2 — versioned key-value store.
//!
//! Mirrors HashiCorp Vault's KV v2 semantics: versioning, soft-delete,
//! undelete, destroy, and metadata management.

use crate::models::{KVMetadata, KVSecret, KVVersionMeta};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KVError {
    #[error("Secret not found: {0}")]
    NotFound(String),
    #[error("Version not found: {0}")]
    VersionNotFound(u32),
    #[error("Version is permanently destroyed")]
    Destroyed,
}

/// Data stored for a single version
#[derive(Debug, Clone)]
pub struct KVVersionData {
    pub data: HashMap<String, Value>,
    pub created_time: DateTime<Utc>,
    pub deletion_time: Option<DateTime<Utc>>,
    pub destroyed: bool,
}

/// All versions + metadata for one KV path
#[derive(Debug, Clone)]
pub struct KVEntry {
    pub versions: HashMap<u32, KVVersionData>,
    pub current_version: u32,
    pub max_versions: u32,
    pub custom_metadata: HashMap<String, String>,
    pub created_time: DateTime<Utc>,
    pub updated_time: DateTime<Utc>,
}

impl KVEntry {
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            versions: HashMap::new(),
            current_version: 0,
            max_versions: 10,
            custom_metadata: HashMap::new(),
            created_time: now,
            updated_time: now,
        }
    }
}

impl Default for KVEntry {
    fn default() -> Self {
        Self::new()
    }
}

/// Read a secret, optionally pinning to a specific version.
pub fn kv_get(
    store: &HashMap<String, KVEntry>,
    path: &str,
    version: Option<u32>,
) -> Result<KVSecret, KVError> {
    let entry = store
        .get(path)
        .ok_or_else(|| KVError::NotFound(path.to_string()))?;
    let ver = version.unwrap_or(entry.current_version);
    let vdata = entry.versions.get(&ver).ok_or(KVError::VersionNotFound(ver))?;
    if vdata.destroyed {
        return Err(KVError::Destroyed);
    }
    Ok(KVSecret {
        path: path.to_string(),
        data: vdata.data.clone(),
        version: ver,
        metadata: build_metadata(entry),
    })
}

/// Write a new version of a secret.
pub fn kv_put(
    store: &mut HashMap<String, KVEntry>,
    path: &str,
    data: HashMap<String, Value>,
    custom_metadata: Option<HashMap<String, String>>,
) -> KVSecret {
    let now = Utc::now();
    let entry = store.entry(path.to_string()).or_default();
    entry.current_version += 1;
    entry.updated_time = now;
    if let Some(meta) = custom_metadata {
        entry.custom_metadata.extend(meta);
    }
    entry.versions.insert(
        entry.current_version,
        KVVersionData {
            data: data.clone(),
            created_time: now,
            deletion_time: None,
            destroyed: false,
        },
    );
    // Prune versions older than max_versions
    let max = entry.max_versions;
    let cur = entry.current_version;
    if cur > max {
        entry.versions.retain(|&v, _| v > cur - max);
    }
    let version = entry.current_version;
    let metadata = build_metadata(entry);
    KVSecret { path: path.to_string(), data, version, metadata }
}

/// Soft-delete the latest version (marks deletion_time, data preserved).
pub fn kv_delete(
    store: &mut HashMap<String, KVEntry>,
    path: &str,
) -> Result<(), KVError> {
    let entry = store
        .get_mut(path)
        .ok_or_else(|| KVError::NotFound(path.to_string()))?;
    let ver = entry.current_version;
    if let Some(v) = entry.versions.get_mut(&ver) {
        v.deletion_time = Some(Utc::now());
    }
    Ok(())
}

/// List paths under a prefix (returns full paths).
pub fn kv_list(store: &HashMap<String, KVEntry>, prefix: &str) -> Vec<String> {
    let prefix = if prefix.ends_with('/') {
        prefix.to_string()
    } else {
        format!("{prefix}/")
    };
    let mut keys: Vec<String> = store
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// Soft-delete specific versions.
pub fn kv_soft_delete(
    store: &mut HashMap<String, KVEntry>,
    path: &str,
    versions: &[u32],
) -> Result<(), KVError> {
    let entry = store
        .get_mut(path)
        .ok_or_else(|| KVError::NotFound(path.to_string()))?;
    let now = Utc::now();
    for &ver in versions {
        if let Some(v) = entry.versions.get_mut(&ver) {
            v.deletion_time = Some(now);
        }
    }
    Ok(())
}

/// Undelete (restore) soft-deleted versions.
pub fn kv_undelete(
    store: &mut HashMap<String, KVEntry>,
    path: &str,
    versions: &[u32],
) -> Result<(), KVError> {
    let entry = store
        .get_mut(path)
        .ok_or_else(|| KVError::NotFound(path.to_string()))?;
    for &ver in versions {
        if let Some(v) = entry.versions.get_mut(&ver) {
            if v.destroyed {
                return Err(KVError::Destroyed);
            }
            v.deletion_time = None;
        }
    }
    Ok(())
}

/// Permanently destroy specific versions (data zeroed, irrecoverable).
pub fn kv_destroy(
    store: &mut HashMap<String, KVEntry>,
    path: &str,
    versions: &[u32],
) -> Result<(), KVError> {
    let entry = store
        .get_mut(path)
        .ok_or_else(|| KVError::NotFound(path.to_string()))?;
    for &ver in versions {
        if let Some(v) = entry.versions.get_mut(&ver) {
            v.destroyed = true;
            v.data.clear();
        }
    }
    Ok(())
}

/// Read metadata for a path without returning secret data.
pub fn kv_read_metadata(
    store: &HashMap<String, KVEntry>,
    path: &str,
) -> Result<KVMetadata, KVError> {
    let entry = store
        .get(path)
        .ok_or_else(|| KVError::NotFound(path.to_string()))?;
    Ok(build_metadata(entry))
}

/// Update path-level metadata (max_versions, custom_metadata).
pub fn kv_update_metadata(
    store: &mut HashMap<String, KVEntry>,
    path: &str,
    max_versions: Option<u32>,
    custom_metadata: Option<HashMap<String, String>>,
) -> Result<(), KVError> {
    let entry = store
        .get_mut(path)
        .ok_or_else(|| KVError::NotFound(path.to_string()))?;
    if let Some(max) = max_versions {
        entry.max_versions = max;
    }
    if let Some(meta) = custom_metadata {
        entry.custom_metadata = meta;
    }
    entry.updated_time = Utc::now();
    Ok(())
}

fn build_metadata(entry: &KVEntry) -> KVMetadata {
    let versions: HashMap<String, KVVersionMeta> = entry
        .versions
        .iter()
        .map(|(ver, data)| {
            (
                ver.to_string(),
                KVVersionMeta {
                    created_time: data.created_time,
                    deletion_time: data.deletion_time,
                    destroyed: data.destroyed,
                },
            )
        })
        .collect();
    let oldest = entry.versions.keys().copied().min().unwrap_or(0);
    let cur = entry.versions.get(&entry.current_version);
    KVMetadata {
        created_time: entry.created_time,
        updated_time: entry.updated_time,
        deletion_time: cur.and_then(|v| v.deletion_time),
        destroyed: cur.map(|v| v.destroyed).unwrap_or(false),
        custom_metadata: entry.custom_metadata.clone(),
        max_versions: entry.max_versions,
        current_version: entry.current_version,
        oldest_version: oldest,
        versions,
    }
}
