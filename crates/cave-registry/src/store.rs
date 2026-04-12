//! In-memory content-addressable registry store.
//! All public methods take `&self` and use internal RwLock for concurrency.

use crate::types::{
    AccessRule, StoredBlob, StoredManifest, TagPolicy, UploadSession,
    WebhookConfig, ReplicationTarget, ScanResult, Permission,
};
use std::collections::HashMap;
use tokio::sync::RwLock;

// ── Digest helpers ────────────────────────────────────────────────────────────

/// Compute `sha256:<hex>` for the given bytes using ring.
pub fn compute_digest(data: &[u8]) -> String {
    use ring::digest::{digest, SHA256};
    let d = digest(&SHA256, data);
    let hex: String = d.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Return true if the bytes match the claimed digest string (`sha256:<hex>`).
pub fn verify_digest(data: &[u8], claimed: &str) -> bool {
    compute_digest(data) == claimed
}

// ── Inner state ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct Inner {
    blobs: HashMap<String, StoredBlob>,
    /// (repository, digest) -> manifest
    manifests: HashMap<(String, String), StoredManifest>,
    /// (repository, tag) -> digest
    tags: HashMap<(String, String), String>,
    sessions: HashMap<String, UploadSession>,
    /// repository -> access rules
    access_rules: HashMap<String, Vec<AccessRule>>,
    /// repository -> tag policy
    tag_policies: HashMap<String, TagPolicy>,
    scan_results: HashMap<String, Vec<ScanResult>>,
    webhooks: Vec<WebhookConfig>,
    replication_targets: Vec<ReplicationTarget>,
}

// ── Public store ──────────────────────────────────────────────────────────────

pub struct RegistryStore {
    inner: RwLock<Inner>,
}

impl Default for RegistryStore {
    fn default() -> Self {
        Self { inner: RwLock::new(Inner::default()) }
    }
}

impl RegistryStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Blob operations ───────────────────────────────────────────────────────

    pub async fn get_blob(&self, digest: &str) -> Option<StoredBlob> {
        self.inner.read().await.blobs.get(digest).cloned()
    }

    pub async fn blob_exists(&self, digest: &str) -> bool {
        self.inner.read().await.blobs.contains_key(digest)
    }

    /// Store a blob; returns its digest. Returns Err if `expected_digest` is
    /// provided and does not match.
    pub async fn put_blob(
        &self,
        data: Vec<u8>,
        expected_digest: Option<&str>,
    ) -> Result<String, String> {
        let digest = compute_digest(&data);
        if let Some(expected) = expected_digest {
            if digest != expected {
                return Err(format!("digest mismatch: expected {expected}, got {digest}"));
            }
        }
        let size = data.len() as u64;
        self.inner.write().await.blobs.insert(
            digest.clone(),
            StoredBlob { digest: digest.clone(), size, content: data },
        );
        Ok(digest)
    }

    pub async fn delete_blob(&self, digest: &str) -> bool {
        self.inner.write().await.blobs.remove(digest).is_some()
    }

    /// Return all blob digests not referenced by any manifest in any repo.
    pub async fn unreferenced_blobs(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        let mut referenced = std::collections::HashSet::new();
        for manifest in inner.manifests.values() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&manifest.content) {
                collect_digests(&v, &mut referenced);
            }
        }
        inner
            .blobs
            .keys()
            .filter(|d| !referenced.contains(d.as_str()))
            .cloned()
            .collect()
    }

    // ── Manifest operations ───────────────────────────────────────────────────

    pub async fn get_manifest(&self, repo: &str, reference: &str) -> Option<StoredManifest> {
        let inner = self.inner.read().await;
        // Try as digest first, then as tag.
        if reference.starts_with("sha256:") {
            return inner.manifests.get(&(repo.to_string(), reference.to_string())).cloned();
        }
        // Look up tag -> digest
        let digest = inner.tags.get(&(repo.to_string(), reference.to_string()))?.clone();
        inner.manifests.get(&(repo.to_string(), digest)).cloned()
    }

    pub async fn put_manifest(
        &self,
        repo: &str,
        reference: &str,
        media_type: String,
        content: Vec<u8>,
    ) -> Result<String, String> {
        let digest = compute_digest(&content);
        let manifest = StoredManifest {
            digest: digest.clone(),
            media_type,
            content,
        };
        let mut inner = self.inner.write().await;
        inner.manifests.insert((repo.to_string(), digest.clone()), manifest);
        // If reference is a tag (not a digest), store tag mapping.
        if !reference.starts_with("sha256:") {
            inner.tags.insert((repo.to_string(), reference.to_string()), digest.clone());
        }
        Ok(digest)
    }

    pub async fn delete_manifest(&self, repo: &str, reference: &str) -> bool {
        let mut inner = self.inner.write().await;
        let digest = if reference.starts_with("sha256:") {
            reference.to_string()
        } else {
            match inner.tags.remove(&(repo.to_string(), reference.to_string())) {
                Some(d) => d,
                None => return false,
            }
        };
        inner.manifests.remove(&(repo.to_string(), digest)).is_some()
    }

    // ── Tag operations ────────────────────────────────────────────────────────

    pub async fn list_tags(&self, repo: &str) -> Vec<String> {
        let inner = self.inner.read().await;
        inner
            .tags
            .keys()
            .filter(|(r, _)| r == repo)
            .map(|(_, t)| t.clone())
            .collect()
    }

    pub async fn tag_exists(&self, repo: &str, tag: &str) -> bool {
        self.inner.read().await.tags.contains_key(&(repo.to_string(), tag.to_string()))
    }

    // ── Repository catalog ────────────────────────────────────────────────────

    pub async fn list_repositories(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        let mut repos: std::collections::HashSet<String> = inner
            .manifests
            .keys()
            .map(|(r, _)| r.clone())
            .collect();
        repos.extend(inner.tags.keys().map(|(r, _)| r.clone()));
        let mut v: Vec<String> = repos.into_iter().collect();
        v.sort();
        v
    }

    // ── Upload sessions ───────────────────────────────────────────────────────

    pub async fn create_session(&self, repo: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let session = UploadSession {
            session_id: id.clone(),
            repository: repo.to_string(),
            data: Vec::new(),
            offset: 0,
        };
        self.inner.write().await.sessions.insert(id.clone(), session);
        id
    }

    pub async fn get_session(&self, id: &str) -> Option<UploadSession> {
        self.inner.read().await.sessions.get(id).cloned()
    }

    pub async fn append_session(&self, id: &str, chunk: Vec<u8>) -> Option<u64> {
        let mut inner = self.inner.write().await;
        let session = inner.sessions.get_mut(id)?;
        session.data.extend_from_slice(&chunk);
        session.offset = session.data.len() as u64;
        Some(session.offset)
    }

    pub async fn complete_session(
        &self,
        id: &str,
        digest: &str,
    ) -> Result<(String, String), String> {
        let mut inner = self.inner.write().await;
        let session = inner.sessions.remove(id).ok_or_else(|| format!("session {id} not found"))?;
        let repo = session.repository.clone();
        let actual = compute_digest(&session.data);
        if actual != digest {
            // Put session back so caller can retry
            inner.sessions.insert(id.to_string(), session);
            return Err(format!("digest mismatch: expected {digest}, got {actual}"));
        }
        let size = session.data.len() as u64;
        inner.blobs.insert(
            digest.to_string(),
            StoredBlob { digest: digest.to_string(), size, content: session.data },
        );
        Ok((repo, actual))
    }

    pub async fn delete_session(&self, id: &str) -> bool {
        self.inner.write().await.sessions.remove(id).is_some()
    }

    // ── Policy / access ───────────────────────────────────────────────────────

    pub async fn set_tag_policy(&self, repo: &str, policy: TagPolicy) {
        self.inner.write().await.tag_policies.insert(repo.to_string(), policy);
    }

    pub async fn get_tag_policy(&self, repo: &str) -> TagPolicy {
        self.inner.read().await.tag_policies.get(repo).cloned().unwrap_or_default()
    }

    pub async fn is_tag_immutable(&self, repo: &str, tag: &str) -> bool {
        let inner = self.inner.read().await;
        let policy = match inner.tag_policies.get(repo) {
            Some(p) => p,
            None => return false,
        };
        if policy.all_immutable {
            return true;
        }
        policy.immutable_tags.iter().any(|t| t == tag)
    }

    pub async fn set_access_rules(&self, repo: &str, rules: Vec<AccessRule>) {
        self.inner.write().await.access_rules.insert(repo.to_string(), rules);
    }

    pub async fn check_permission(&self, repo: &str, subject: &str, perm: &Permission) -> bool {
        let inner = self.inner.read().await;
        let rules = match inner.access_rules.get(repo) {
            Some(r) => r,
            // No rules = open
            None => return true,
        };
        rules.iter().any(|r| {
            r.subject == subject
                && (r.permission == *perm
                    || r.permission == Permission::Admin
                    || (*perm == Permission::Pull && r.permission == Permission::Push))
        })
    }

    // ── Webhooks ──────────────────────────────────────────────────────────────

    pub async fn add_webhook(&self, wh: WebhookConfig) {
        self.inner.write().await.webhooks.push(wh);
    }

    pub async fn get_webhooks(&self) -> Vec<WebhookConfig> {
        self.inner.read().await.webhooks.clone()
    }

    // ── Replication ───────────────────────────────────────────────────────────

    pub async fn add_replication_target(&self, target: ReplicationTarget) {
        self.inner.write().await.replication_targets.push(target);
    }

    pub async fn get_replication_targets(&self) -> Vec<ReplicationTarget> {
        self.inner.read().await.replication_targets.clone()
    }

    // ── Scan results ──────────────────────────────────────────────────────────

    pub async fn store_scan_result(&self, result: ScanResult) {
        let mut inner = self.inner.write().await;
        inner
            .scan_results
            .entry(result.manifest_digest.clone())
            .or_default()
            .push(result);
    }

    pub async fn get_scan_results(&self, digest: &str) -> Vec<ScanResult> {
        self.inner.read().await.scan_results.get(digest).cloned().unwrap_or_default()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn collect_digests(value: &serde_json::Value, out: &mut std::collections::HashSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(d)) = map.get("digest") {
                out.insert(d.clone());
            }
            for v in map.values() {
                collect_digests(v, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_digests(v, out);
            }
        }
        _ => {}
    }
}
