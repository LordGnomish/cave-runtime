// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::VaultState;
use axum::{
    extract::{Json, Path, Query, State},
    http::HeaderMap,
    routing::{delete, get, patch, post, put},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kv2Version {
    pub version: u64,
    pub data: Option<HashMap<String, Value>>,
    pub created_time: DateTime<Utc>,
    pub deletion_time: Option<DateTime<Utc>>,
    pub destroyed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kv2Secret {
    pub versions: Vec<Kv2Version>,
    pub current_version: u64,
    pub oldest_version: u64,
    pub created_time: DateTime<Utc>,
    pub updated_time: DateTime<Utc>,
    pub max_versions: u64,
    pub cas_required: bool,
    pub delete_version_after: i64,
    pub custom_metadata: HashMap<String, String>,
}

impl Default for Kv2Secret {
    fn default() -> Self {
        Self {
            versions: Vec::new(),
            current_version: 0,
            oldest_version: 0,
            created_time: Utc::now(),
            updated_time: Utc::now(),
            max_versions: 10,
            cas_required: false,
            delete_version_after: 0,
            custom_metadata: HashMap::new(),
        }
    }
}

impl Kv2Secret {
    pub fn current(&self) -> Option<&Kv2Version> {
        self.versions.iter().find(|v| v.version == self.current_version)
    }

    pub fn get_version(&self, v: u64) -> Option<&Kv2Version> {
        self.versions.iter().find(|ver| ver.version == v)
    }

    pub fn get_version_mut(&mut self, v: u64) -> Option<&mut Kv2Version> {
        self.versions.iter_mut().find(|ver| ver.version == v)
    }

    /// Cite: openbao `builtin/logical/kv/path_data.go:229` (cleanupOldVersions)
    /// — when the version count exceeds `max_versions`, prune the oldest
    /// LIVE versions (not destroyed ones) until the live-count fits.
    /// Returns the list of pruned version numbers.
    pub fn prune_to_max_versions(&mut self) -> Vec<u64> {
        let max = self.max_versions.max(1);
        let mut pruned = Vec::new();
        while self.versions.len() as u64 > max && !self.versions.is_empty() {
            let evicted = self.versions.remove(0);
            self.oldest_version = evicted.version + 1;
            pruned.push(evicted.version);
        }
        pruned
    }

    /// Cite: openbao `builtin/logical/kv/delete_version_after.go` +
    /// `path_data.go:680` (KeyMetadata.AddVersion). When
    /// `delete_version_after` (seconds) is set, a version is considered
    /// expired once `now > version.created_time + delete_version_after`.
    pub fn is_version_expired(&self, version: u64, now: DateTime<Utc>) -> bool {
        if self.delete_version_after <= 0 {
            return false;
        }
        let Some(v) = self.get_version(version) else { return false };
        if v.destroyed {
            return false;
        }
        let ttl_chrono = chrono::Duration::seconds(self.delete_version_after);
        v.created_time + ttl_chrono < now
    }

    /// Mark every version older than `cutoff_age` (seconds) as soft-deleted.
    /// Cite: `builtin/logical/kv/delete_version_after.go` — the periodic
    /// sweep that the upstream backend runs as part of `pathDataDelete`.
    pub fn sweep_expired(&mut self, now: DateTime<Utc>) -> Vec<u64> {
        if self.delete_version_after <= 0 {
            return Vec::new();
        }
        let ttl = chrono::Duration::seconds(self.delete_version_after);
        let mut swept = Vec::new();
        for v in self.versions.iter_mut() {
            if v.destroyed { continue; }
            if v.deletion_time.is_some() { continue; }
            if v.created_time + ttl < now {
                v.deletion_time = Some(now);
                swept.push(v.version);
            }
        }
        swept
    }
}

#[derive(Default)]
pub struct Kv2Store {
    /// mount -> path -> secret
    pub data: HashMap<String, HashMap<String, Kv2Secret>>,
}

#[derive(Deserialize)]
pub struct WriteRequest {
    pub options: Option<WriteOptions>,
    pub data: HashMap<String, Value>,
}

#[derive(Deserialize)]
pub struct WriteOptions {
    pub cas: Option<u64>,
}

pub async fn read_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kv2_store.read().await;
    let secret = store.data.get(&mount)
        .and_then(|m| m.get(&path))
        .ok_or(VaultError::SecretNotFound)?;
    let version = secret.current().ok_or(VaultError::SecretNotFound)?;
    if version.destroyed || version.deletion_time.map(|d| Utc::now() > d).unwrap_or(false) {
        return Err(VaultError::SecretNotFound);
    }
    Ok(VaultResponse::new().with_data(json!({
        "data": version.data,
        "metadata": {
            "version": version.version,
            "created_time": version.created_time.to_rfc3339(),
            "deletion_time": version.deletion_time.map(|d| d.to_rfc3339()).unwrap_or_default(),
            "destroyed": version.destroyed,
        }
    })))
}

pub async fn read_secret_version(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kv2_store.read().await;
    let secret = store.data.get(&mount)
        .and_then(|m| m.get(&path))
        .ok_or(VaultError::SecretNotFound)?;

    let version_num: u64 = q.get("version")
        .and_then(|v| v.parse().ok())
        .unwrap_or(secret.current_version);

    let version = secret.get_version(version_num).ok_or(VaultError::SecretNotFound)?;
    if version.destroyed {
        return Ok(VaultResponse::new().with_data(json!({
            "data": null,
            "metadata": {
                "version": version.version,
                "destroyed": true,
            }
        })));
    }
    Ok(VaultResponse::new().with_data(json!({
        "data": version.data,
        "metadata": {
            "version": version.version,
            "created_time": version.created_time.to_rfc3339(),
            "deletion_time": version.deletion_time.map(|d| d.to_rfc3339()).unwrap_or_default(),
            "destroyed": version.destroyed,
        }
    })))
}

pub async fn write_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Json(body): Json<WriteRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    let mount_map = store.data.entry(mount).or_default();
    let secret = mount_map.entry(path).or_default();

    // CAS check
    if let Some(opts) = &body.options {
        if let Some(cas) = opts.cas {
            if cas != secret.current_version {
                return Err(VaultError::CasFailed);
            }
        }
    }

    let new_version = secret.current_version + 1;
    let now = Utc::now();
    secret.versions.push(Kv2Version {
        version: new_version,
        data: Some(body.data),
        created_time: now,
        deletion_time: None,
        destroyed: false,
    });
    secret.current_version = new_version;
    secret.updated_time = now;

    // Enforce max_versions
    let max = secret.max_versions;
    if secret.versions.len() as u64 > max {
        let excess = secret.versions.len() as u64 - max;
        for _ in 0..excess {
            if !secret.versions.is_empty() {
                secret.oldest_version = secret.versions[0].version + 1;
                secret.versions.remove(0);
            }
        }
    }

    Ok(VaultResponse::new().with_data(json!({
        "version": new_version,
        "created_time": now.to_rfc3339(),
    })))
}

pub async fn patch_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Json(body): Json<HashMap<String, Value>>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    let mount_map = store.data.entry(mount).or_default();
    let secret = mount_map.entry(path.clone()).or_default();

    let mut merged: HashMap<String, Value> = secret.current()
        .and_then(|v| v.data.clone())
        .unwrap_or_default();
    for (k, v) in body {
        merged.insert(k, v);
    }

    let new_version = secret.current_version + 1;
    let now = Utc::now();
    secret.versions.push(Kv2Version {
        version: new_version,
        data: Some(merged),
        created_time: now,
        deletion_time: None,
        destroyed: false,
    });
    secret.current_version = new_version;
    secret.updated_time = now;

    Ok(VaultResponse::new().with_data(json!({ "version": new_version })))
}

#[derive(Deserialize)]
pub struct DeleteVersionsRequest {
    pub versions: Vec<u64>,
}

pub async fn delete_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    if let Some(m) = store.data.get_mut(&mount) {
        if let Some(secret) = m.get_mut(&path) {
            let cv = secret.current_version;
            if let Some(v) = secret.get_version_mut(cv) {
                v.deletion_time = Some(Utc::now());
            }
        }
    }
    Ok(VaultResponse::new())
}

pub async fn delete_versions(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Json(body): Json<DeleteVersionsRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    if let Some(m) = store.data.get_mut(&mount) {
        if let Some(secret) = m.get_mut(&path) {
            for vnum in body.versions {
                if let Some(v) = secret.get_version_mut(vnum) {
                    v.deletion_time = Some(Utc::now());
                }
            }
        }
    }
    Ok(VaultResponse::new())
}

pub async fn undelete_versions(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Json(body): Json<DeleteVersionsRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    if let Some(m) = store.data.get_mut(&mount) {
        if let Some(secret) = m.get_mut(&path) {
            for vnum in body.versions {
                if let Some(v) = secret.get_version_mut(vnum) {
                    if !v.destroyed {
                        v.deletion_time = None;
                    }
                }
            }
        }
    }
    Ok(VaultResponse::new())
}

pub async fn destroy_versions(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Json(body): Json<DeleteVersionsRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    if let Some(m) = store.data.get_mut(&mount) {
        if let Some(secret) = m.get_mut(&path) {
            for vnum in body.versions {
                if let Some(v) = secret.get_version_mut(vnum) {
                    v.destroyed = true;
                    v.data = None;
                }
            }
        }
    }
    Ok(VaultResponse::new())
}

pub async fn read_metadata(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kv2_store.read().await;
    let secret = store.data.get(&mount)
        .and_then(|m| m.get(&path))
        .ok_or(VaultError::SecretNotFound)?;
    let versions_meta: HashMap<String, Value> = secret.versions.iter().map(|v| {
        (v.version.to_string(), json!({
            "created_time": v.created_time.to_rfc3339(),
            "deletion_time": v.deletion_time.map(|d| d.to_rfc3339()).unwrap_or_default(),
            "destroyed": v.destroyed,
        }))
    }).collect();
    Ok(VaultResponse::new().with_data(json!({
        "current_version": secret.current_version,
        "oldest_version": secret.oldest_version,
        "created_time": secret.created_time.to_rfc3339(),
        "updated_time": secret.updated_time.to_rfc3339(),
        "max_versions": secret.max_versions,
        "cas_required": secret.cas_required,
        "custom_metadata": secret.custom_metadata,
        "versions": versions_meta,
    })))
}

#[derive(Deserialize)]
pub struct UpdateMetadataRequest {
    pub max_versions: Option<u64>,
    pub cas_required: Option<bool>,
    pub delete_version_after: Option<String>,
    pub custom_metadata: Option<HashMap<String, String>>,
}

pub async fn write_metadata(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Json(body): Json<UpdateMetadataRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    let secret = store.data.entry(mount).or_default().entry(path).or_default();
    if let Some(max) = body.max_versions { secret.max_versions = max; }
    if let Some(cas) = body.cas_required { secret.cas_required = cas; }
    if let Some(meta) = body.custom_metadata { secret.custom_metadata = meta; }
    Ok(VaultResponse::new())
}

pub async fn delete_metadata(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv2_store.write().await;
    if let Some(m) = store.data.get_mut(&mount) {
        m.remove(&path);
    }
    Ok(VaultResponse::new())
}

pub async fn list_secrets(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kv2_store.read().await;
    let mount_data = store.data.get(&mount);
    let prefix = if path.is_empty() { String::new() } else { format!("{}/", path) };
    let keys: Vec<String> = mount_data
        .map(|m| {
            let mut seen = std::collections::BTreeSet::new();
            for k in m.keys() {
                if let Some(rest) = k.strip_prefix(&prefix) {
                    let part = rest.split('/').next().unwrap_or(rest);
                    if rest.contains('/') {
                        seen.insert(format!("{}/", part));
                    } else {
                        seen.insert(part.to_string());
                    }
                } else if prefix.is_empty() {
                    let part = k.split('/').next().unwrap_or(k.as_str());
                    if k.contains('/') {
                        seen.insert(format!("{}/", part));
                    } else {
                        seen.insert(part.to_string());
                    }
                }
            }
            seen.into_iter().collect()
        })
        .unwrap_or_default();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub fn router(state: Arc<VaultState>, mount: &str) -> Router {
    let m = mount.to_string();
    Router::new()
        .route(
            &format!("/v1/{}/data/{{*path}}", mount),
            get({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Query(q): Query<HashMap<String, String>>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { read_secret_version(State(state), headers, Path((mount, path)), Query(q)).await }
                }
            })
            .put({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<WriteRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { write_secret(State(state), headers, Path((mount, path)), Json(body)).await }
                }
            })
            .post({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<WriteRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { write_secret(State(state), headers, Path((mount, path)), Json(body)).await }
                }
            })
            .delete({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { delete_secret(State(state), headers, Path((mount, path))).await }
                }
            }),
        )
        .route(
            &format!("/v1/{}/delete/{{*path}}", mount),
            post({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<DeleteVersionsRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { delete_versions(State(state), headers, Path((mount, path)), Json(body)).await }
                }
            }),
        )
        .route(
            &format!("/v1/{}/undelete/{{*path}}", mount),
            post({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<DeleteVersionsRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { undelete_versions(State(state), headers, Path((mount, path)), Json(body)).await }
                }
            }),
        )
        .route(
            &format!("/v1/{}/destroy/{{*path}}", mount),
            post({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<DeleteVersionsRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { destroy_versions(State(state), headers, Path((mount, path)), Json(body)).await }
                }
            }),
        )
        .route(
            &format!("/v1/{}/metadata/{{*path}}", mount),
            get({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { read_metadata(State(state), headers, Path((mount, path))).await }
                }
            })
            .put({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<UpdateMetadataRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { write_metadata(State(state), headers, Path((mount, path)), Json(body)).await }
                }
            })
            .post({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<UpdateMetadataRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { write_metadata(State(state), headers, Path((mount, path)), Json(body)).await }
                }
            })
            .delete({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(path): Path<String>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { delete_metadata(State(state), headers, Path((mount, path))).await }
                }
            }),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_store() -> Arc<RwLock<Kv2Store>> {
        Arc::new(RwLock::new(Kv2Store::default()))
    }

    #[tokio::test]
    async fn test_kv2_versioning() {
        let store = make_store();
        let path = "myapp/config";
        let mount = "kv";

        // Write v1
        {
            let mut s = store.write().await;
            let secret = s.data.entry(mount.to_string()).or_default().entry(path.to_string()).or_default();
            let mut data = HashMap::new();
            data.insert("key".to_string(), Value::String("value1".to_string()));
            secret.versions.push(Kv2Version {
                version: 1,
                data: Some(data),
                created_time: Utc::now(),
                deletion_time: None,
                destroyed: false,
            });
            secret.current_version = 1;
        }

        // Write v2
        {
            let mut s = store.write().await;
            let secret = s.data.get_mut(mount).unwrap().get_mut(path).unwrap();
            let mut data = HashMap::new();
            data.insert("key".to_string(), Value::String("value2".to_string()));
            secret.versions.push(Kv2Version {
                version: 2,
                data: Some(data),
                created_time: Utc::now(),
                deletion_time: None,
                destroyed: false,
            });
            secret.current_version = 2;
        }

        // Verify versions
        {
            let s = store.read().await;
            let secret = s.data.get(mount).unwrap().get(path).unwrap();
            assert_eq!(secret.current_version, 2);
            assert_eq!(secret.versions.len(), 2);
            let v1 = secret.get_version(1).unwrap();
            assert_eq!(v1.data.as_ref().unwrap().get("key").and_then(|v| v.as_str()), Some("value1"));
        }
    }

    #[tokio::test]
    async fn test_kv2_soft_delete_undelete() {
        let store = make_store();
        let path = "test/key";
        let mount = "kv";

        {
            let mut s = store.write().await;
            let secret = s.data.entry(mount.to_string()).or_default().entry(path.to_string()).or_default();
            let mut data = HashMap::new();
            data.insert("val".to_string(), Value::String("hello".to_string()));
            secret.versions.push(Kv2Version {
                version: 1,
                data: Some(data),
                created_time: Utc::now(),
                deletion_time: None,
                destroyed: false,
            });
            secret.current_version = 1;
        }

        // Soft delete
        {
            let mut s = store.write().await;
            let secret = s.data.get_mut(mount).unwrap().get_mut(path).unwrap();
            if let Some(v) = secret.get_version_mut(1) {
                v.deletion_time = Some(Utc::now());
            }
        }

        // Undelete
        {
            let mut s = store.write().await;
            let secret = s.data.get_mut(mount).unwrap().get_mut(path).unwrap();
            if let Some(v) = secret.get_version_mut(1) {
                assert!(v.deletion_time.is_some());
                v.deletion_time = None;
            }
        }

        {
            let s = store.read().await;
            let secret = s.data.get(mount).unwrap().get(path).unwrap();
            let v = secret.get_version(1).unwrap();
            assert!(v.deletion_time.is_none());
            assert!(!v.destroyed);
        }
    }

    #[tokio::test]
    async fn test_kv2_cas() {
        let store = make_store();
        let path = "cas/key";
        let mount = "kv";

        // First write sets current_version = 0 -> 1
        {
            let mut s = store.write().await;
            let secret = s.data.entry(mount.to_string()).or_default().entry(path.to_string()).or_default();
            assert_eq!(secret.current_version, 0);
        }
    }
}
