// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/pkg/blob/manager.go + src/pkg/blob/storage.go
//! Content-addressable in-memory registry storage.
//!
//! Production deployments would back this with S3/GCS/Azure Blob, but the
//! interface is the same — only the Store impl changes.

use crate::harbor::models::{Descriptor, ManifestEntry, UploadState};
use bytes::Bytes;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;
use tracing::debug;
use uuid::Uuid;

// ── Storage structs ───────────────────────────────────────────────────────────

/// Index of all manifests across all repos.
#[derive(Default)]
struct ManifestIndex {
    /// (repo, digest) → ManifestEntry
    by_digest: HashMap<(String, String), ManifestEntry>,
    /// (repo, tag) → digest
    by_tag: HashMap<(String, String), String>,
    /// subject_digest → Vec<manifest_digest> (OCI 1.1 referrers)
    referrers: HashMap<String, Vec<String>>,
    /// known repositories
    repos: HashSet<String>,
}

/// Thread-safe, content-addressable registry storage.
pub struct RegistryStorage {
    blobs: RwLock<HashMap<String, Bytes>>,
    blob_refs: RwLock<HashMap<String, HashSet<String>>>,
    manifests: RwLock<ManifestIndex>,
    uploads: RwLock<HashMap<String, UploadState>>,
    aliases: RwLock<HashMap<String, String>>,
}

impl Default for RegistryStorage {
    fn default() -> Self {
        Self {
            blobs: RwLock::new(HashMap::new()),
            blob_refs: RwLock::new(HashMap::new()),
            manifests: RwLock::new(ManifestIndex::default()),
            uploads: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashMap::new()),
        }
    }
}

impl RegistryStorage {
    pub async fn get_blob_by_alias(&self, alias: &str) -> Option<Bytes> {
        let digest = self.aliases.read().await.get(alias).cloned()?;
        self.blobs.read().await.get(&digest).cloned()
    }

    pub async fn put_blob_with_alias(&self, alias: String, data: Bytes) {
        let digest = compute_digest(&data);
        self.blobs
            .write()
            .await
            .entry(digest.clone())
            .or_insert(data);
        self.aliases.write().await.insert(alias, digest);
    }
}

// ── Digest helpers ────────────────────────────────────────────────────────────

pub fn compute_digest(data: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(data)))
}

pub fn verify_digest(data: &[u8], expected: &str) -> bool {
    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    hex::encode(Sha256::digest(data)) == expected
}

// ── RegistryStorage impl ──────────────────────────────────────────────────────

impl RegistryStorage {
    // ── Repositories ─────────────────────────────────────────────────────────

    pub async fn list_repos(&self) -> Vec<String> {
        let idx = self.manifests.read().await;
        let mut repos: Vec<String> = idx.repos.iter().cloned().collect();
        repos.sort();
        repos
    }

    // ── Blobs ─────────────────────────────────────────────────────────────────

    pub async fn has_blob(&self, digest: &str) -> bool {
        self.blobs.read().await.contains_key(digest)
    }

    pub async fn get_blob(&self, digest: &str) -> Option<Bytes> {
        self.blobs.read().await.get(digest).cloned()
    }

    pub async fn store_blob(&self, digest: String, data: Bytes, repo: &str) {
        {
            let mut blobs = self.blobs.write().await;
            blobs.insert(digest.clone(), data);
        }
        {
            let mut refs = self.blob_refs.write().await;
            refs.entry(digest.clone())
                .or_default()
                .insert(repo.to_string());
        }
        debug!(digest = %digest, repo = %repo, "blob stored");
    }

    /// Cross-repo mount: link an existing blob into another repo.
    pub async fn mount_blob(&self, digest: &str, from_repo: &str, to_repo: &str) -> bool {
        let has = self.has_blob(digest).await;
        if has {
            let mut refs = self.blob_refs.write().await;
            let entry = refs.entry(digest.to_string()).or_default();
            if entry.contains(from_repo) {
                entry.insert(to_repo.to_string());
                debug!(digest = %digest, from = %from_repo, to = %to_repo, "blob mounted");
                return true;
            }
        }
        false
    }

    pub async fn delete_blob(&self, digest: &str, repo: &str) -> bool {
        let mut refs = self.blob_refs.write().await;
        if let Some(repos) = refs.get_mut(digest) {
            repos.remove(repo);
            if repos.is_empty() {
                refs.remove(digest);
                drop(refs);
                self.blobs.write().await.remove(digest);
                debug!(digest = %digest, "blob fully removed");
            }
            return true;
        }
        false
    }

    // ── Upload sessions ───────────────────────────────────────────────────────

    pub async fn start_upload(&self, repo: &str) -> String {
        let uuid = Uuid::new_v4().to_string();
        let session = UploadState::new(uuid.clone(), repo.to_string());
        self.uploads.write().await.insert(uuid.clone(), session);
        debug!(uuid = %uuid, repo = %repo, "upload started");
        uuid
    }

    /// Append bytes to an in-progress upload. Returns new total offset or None
    /// if the session doesn't exist.
    pub async fn patch_upload(&self, uuid: &str, data: Bytes) -> Option<usize> {
        let mut uploads = self.uploads.write().await;
        let session = uploads.get_mut(uuid)?;
        session.data.extend_from_slice(&data);
        let offset = session.data.len();
        debug!(uuid = %uuid, offset = offset, "upload chunk appended");
        Some(offset)
    }

    /// Finalise an upload: verify digest, store blob, drop session.
    /// Returns (digest, repo) on success.
    pub async fn complete_upload(
        &self,
        uuid: &str,
        final_chunk: Bytes,
        expected_digest: &str,
    ) -> Result<(String, String), &'static str> {
        let mut uploads = self.uploads.write().await;
        let mut session = uploads.remove(uuid).ok_or("upload session not found")?;
        session.data.extend_from_slice(&final_chunk);

        if !verify_digest(&session.data, expected_digest) {
            // Restore session so the client can retry
            uploads.insert(uuid.to_string(), session);
            return Err("digest mismatch");
        }

        let digest = compute_digest(&session.data);
        let repo = session.repository.clone();
        drop(uploads);

        let bytes = Bytes::from(session.data);
        self.store_blob(digest.clone(), bytes, &repo).await;
        Ok((digest, repo))
    }

    pub async fn cancel_upload(&self, uuid: &str) -> bool {
        self.uploads.write().await.remove(uuid).is_some()
    }

    pub async fn upload_offset(&self, uuid: &str) -> Option<usize> {
        self.uploads.read().await.get(uuid).map(|s| s.offset())
    }

    // ── Manifests ─────────────────────────────────────────────────────────────

    /// Look up by tag or digest reference.
    pub async fn get_manifest(&self, repo: &str, reference: &str) -> Option<ManifestEntry> {
        let idx = self.manifests.read().await;
        if reference.starts_with("sha256:") {
            idx.by_digest
                .get(&(repo.to_string(), reference.to_string()))
                .cloned()
        } else {
            // tag lookup
            let digest = idx.by_tag.get(&(repo.to_string(), reference.to_string()))?;
            idx.by_digest
                .get(&(repo.to_string(), digest.clone()))
                .cloned()
        }
    }

    pub async fn store_manifest(
        &self,
        repo: &str,
        reference: &str,
        content_type: String,
        data: Bytes,
        subject_digest: Option<String>,
        artifact_type: Option<String>,
    ) -> String {
        let digest = compute_digest(&data);
        let entry = ManifestEntry {
            digest: digest.clone(),
            content_type,
            data,
            subject_digest: subject_digest.clone(),
            artifact_type: artifact_type.clone(),
            created_at: Utc::now(),
        };
        let mut idx = self.manifests.write().await;
        idx.repos.insert(repo.to_string());
        idx.by_digest
            .insert((repo.to_string(), digest.clone()), entry);
        // If reference is a tag, track the tag→digest mapping
        if !reference.starts_with("sha256:") {
            idx.by_tag
                .insert((repo.to_string(), reference.to_string()), digest.clone());
        }
        // OCI 1.1 referrers index
        if let Some(subj) = subject_digest {
            idx.referrers.entry(subj).or_default().push(digest.clone());
        }
        debug!(repo = %repo, reference = %reference, digest = %digest, "manifest stored");
        digest
    }

    pub async fn delete_manifest(&self, repo: &str, reference: &str) -> bool {
        let mut idx = self.manifests.write().await;
        let digest = if reference.starts_with("sha256:") {
            reference.to_string()
        } else {
            match idx
                .by_tag
                .remove(&(repo.to_string(), reference.to_string()))
            {
                Some(d) => d,
                None => return false,
            }
        };
        let removed = idx
            .by_digest
            .remove(&(repo.to_string(), digest.clone()))
            .is_some();
        // Remove from repo if no manifests remain
        let still_has = idx.by_digest.keys().any(|(r, _)| r == repo);
        if !still_has {
            idx.repos.remove(repo);
        }
        removed
    }

    pub async fn list_tags(&self, repo: &str) -> Vec<String> {
        let idx = self.manifests.read().await;
        let mut tags: Vec<String> = idx
            .by_tag
            .keys()
            .filter(|(r, _)| r == repo)
            .map(|(_, t)| t.clone())
            .collect();
        tags.sort();
        tags
    }

    /// OCI 1.1 referrers: find all manifests that have `subject = digest`.
    pub async fn get_referrers(
        &self,
        subject_digest: &str,
        artifact_type_filter: Option<&str>,
    ) -> Vec<Descriptor> {
        let idx = self.manifests.read().await;
        let manifest_digests = match idx.referrers.get(subject_digest) {
            Some(v) => v.clone(),
            None => return vec![],
        };
        let mut descriptors = Vec::new();
        for digest in manifest_digests {
            // search across all repos for this digest
            for ((_, d), entry) in &idx.by_digest {
                if d == &digest {
                    if let Some(filter) = artifact_type_filter {
                        if entry.artifact_type.as_deref() != Some(filter) {
                            continue;
                        }
                    }
                    descriptors.push(Descriptor {
                        media_type: entry.content_type.clone(),
                        size: entry.data.len() as i64,
                        digest: entry.digest.clone(),
                        platform: None,
                        artifact_type: entry.artifact_type.clone(),
                        annotations: None,
                        urls: None,
                    });
                    break;
                }
            }
        }
        descriptors
    }

    // ── Garbage collection ────────────────────────────────────────────────────

    pub async fn gc(&self) -> GcStats {
        let idx = self.manifests.read().await;

        // Collect all blob digests referenced by manifests
        let mut live_blobs: HashSet<String> = HashSet::new();
        for entry in idx.by_digest.values() {
            // Parse the manifest to find layer/config digests
            if let Ok(manifest) =
                serde_json::from_slice::<crate::harbor::models::ImageManifest>(&entry.data)
            {
                live_blobs.insert(manifest.config.digest.clone());
                for layer in &manifest.layers {
                    live_blobs.insert(layer.digest.clone());
                }
            }
            // Also keep the manifest blob itself
            live_blobs.insert(entry.digest.clone());
        }
        drop(idx);

        // Remove unreferenced blobs
        let mut blobs = self.blobs.write().await;
        let before = blobs.len();
        blobs.retain(|digest, _| live_blobs.contains(digest));
        let removed = before - blobs.len();

        let mut refs = self.blob_refs.write().await;
        refs.retain(|digest, _| live_blobs.contains(digest));

        GcStats {
            blobs_removed: removed,
            blobs_retained: blobs.len(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GcStats {
    pub blobs_removed: usize,
    pub blobs_retained: usize,
}

use serde::Serialize;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn store_and_retrieve_blob() {
        let s = RegistryStorage::default();
        let data = Bytes::from_static(b"hello world");
        let digest = compute_digest(&data);
        s.store_blob(digest.clone(), data.clone(), "myrepo").await;
        assert!(s.has_blob(&digest).await);
        assert_eq!(s.get_blob(&digest).await.unwrap(), data);
    }

    #[tokio::test]
    async fn upload_session_lifecycle() {
        let s = RegistryStorage::default();
        let uuid = s.start_upload("myrepo").await;
        let offset = s.patch_upload(&uuid, Bytes::from_static(b"chunk1")).await;
        assert_eq!(offset, Some(6));
        let digest = compute_digest(b"chunk1");
        let result = s.complete_upload(&uuid, Bytes::new(), &digest).await;
        assert!(result.is_ok());
        assert!(s.has_blob(&digest).await);
    }

    #[tokio::test]
    async fn upload_digest_mismatch() {
        let s = RegistryStorage::default();
        let uuid = s.start_upload("myrepo").await;
        s.patch_upload(&uuid, Bytes::from_static(b"real data"))
            .await;
        let result = s
            .complete_upload(
                &uuid,
                Bytes::new(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn manifest_store_and_tag() {
        let s = RegistryStorage::default();
        let data = Bytes::from_static(b"{\"schemaVersion\":2}");
        let digest = s
            .store_manifest(
                "library/ubuntu",
                "latest",
                "application/vnd.oci.image.manifest.v1+json".to_string(),
                data,
                None,
                None,
            )
            .await;
        assert!(digest.starts_with("sha256:"));
        let by_tag = s.get_manifest("library/ubuntu", "latest").await;
        let by_digest = s.get_manifest("library/ubuntu", &digest).await;
        assert!(by_tag.is_some());
        assert!(by_digest.is_some());
        let tags = s.list_tags("library/ubuntu").await;
        assert_eq!(tags, vec!["latest"]);
    }

    #[tokio::test]
    async fn cross_repo_mount() {
        let s = RegistryStorage::default();
        let data = Bytes::from_static(b"layer data");
        let digest = compute_digest(&data);
        s.store_blob(digest.clone(), data, "src/repo").await;
        let mounted = s.mount_blob(&digest, "src/repo", "dst/repo").await;
        assert!(mounted);
    }

    #[tokio::test]
    async fn gc_removes_unreferenced_blobs() {
        let s = RegistryStorage::default();
        let data = Bytes::from_static(b"orphan blob");
        let digest = compute_digest(&data);
        s.store_blob(digest.clone(), data, "myrepo").await;
        // no manifest references this blob
        let stats = s.gc().await;
        assert_eq!(stats.blobs_removed, 1);
    }
}
