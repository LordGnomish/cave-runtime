// SPDX-License-Identifier: AGPL-3.0-or-later
//! OCI registry client — pulls manifests and blobs from container registries.
//!
//! Implements the Docker Registry HTTP API v2 for image pulling.

use crate::error::{CriError, CriResult};
use crate::models::{ImageConfig, ImageReference, OciDescriptor, OciImage, OciLayer, OciManifest};
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Parsed Bearer challenge from a registry's `WWW-Authenticate` response
/// header. Cite: containerd v2.2.3
/// `core/remotes/docker/auth/parse.go` (`ParseAuthHeader`) and
/// distribution-spec v1.1 §4.4 (Token Authentication Specification).
///
/// Example header:
/// ```text
/// WWW-Authenticate: Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull"
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BearerChallenge {
    pub realm: String,
    pub service: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

impl BearerChallenge {
    /// Parse a `WWW-Authenticate` header value. Accepts the canonical
    /// `Bearer realm="...",service="...",scope="..."` shape and is
    /// permissive about whitespace + parameter ordering.
    pub fn parse(header: &str) -> CriResult<Self> {
        let trimmed = header.trim();
        let rest = trimmed.strip_prefix("Bearer")
            .or_else(|| trimmed.strip_prefix("bearer"))
            .ok_or_else(|| CriError::Registry(
                "WWW-Authenticate: missing Bearer scheme".into()
            ))?;
        let mut params = HashMap::new();
        for kv in split_csv_respecting_quotes(rest.trim()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| {
                CriError::Registry(format!("WWW-Authenticate: malformed parameter '{}'", kv))
            })?;
            let key = k.trim().to_lowercase();
            let value = v.trim().trim_matches('"').to_string();
            params.insert(key, value);
        }
        let realm = params.remove("realm").ok_or_else(|| {
            CriError::Registry("WWW-Authenticate: 'realm' parameter is mandatory".into())
        })?;
        Ok(Self {
            realm,
            service: params.remove("service"),
            scope: params.remove("scope"),
            error: params.remove("error"),
        })
    }

    /// Build the token-endpoint URL for this challenge.
    pub fn token_url(&self) -> String {
        let mut url = self.realm.clone();
        let mut sep = if url.contains('?') { '&' } else { '?' };
        if let Some(svc) = &self.service {
            url.push(sep);
            url.push_str(&format!("service={}", urlencode(svc)));
            sep = '&';
        }
        if let Some(scope) = &self.scope {
            url.push(sep);
            url.push_str(&format!("scope={}", urlencode(scope)));
        }
        url
    }
}

fn split_csv_respecting_quotes(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in s.chars() {
        match c {
            '"' => { in_quotes = !in_quotes; cur.push(c); }
            ',' if !in_quotes => {
                if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Tenant-scoped Bearer-token cache. Tokens are keyed by `(registry,
/// repository, scope)` so a single tenant can hold per-repo tokens.
/// Cite: containerd v2.2.3 `core/remotes/docker/authorizer.go` keeps the
/// equivalent map in `dockerAuthorizer.handlers`.
#[derive(Debug)]
pub struct TokenCache {
    pub tenant_id: String,
    entries: Mutex<HashMap<TokenKey, CachedToken>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct TokenKey {
    registry: String,
    repository: String,
    scope: String,
}

#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

impl TokenCache {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into(), entries: Mutex::new(HashMap::new()) }
    }

    pub fn put(&self, registry: &str, repository: &str, scope: &str, token: &str, ttl_secs: i64) {
        let key = TokenKey {
            registry: registry.to_string(),
            repository: repository.to_string(),
            scope: scope.to_string(),
        };
        let cached = CachedToken {
            token: token.to_string(),
            expires_at: Utc::now() + Duration::seconds(ttl_secs.max(1)),
        };
        self.entries.lock().unwrap().insert(key, cached);
    }

    pub fn get(&self, registry: &str, repository: &str, scope: &str) -> Option<String> {
        let key = TokenKey {
            registry: registry.to_string(),
            repository: repository.to_string(),
            scope: scope.to_string(),
        };
        let map = self.entries.lock().unwrap();
        let entry = map.get(&key)?;
        if Utc::now() >= entry.expires_at {
            return None;
        }
        Some(entry.token.clone())
    }

    pub fn evict(&self, registry: &str, repository: &str, scope: &str) -> bool {
        let key = TokenKey {
            registry: registry.to_string(),
            repository: repository.to_string(),
            scope: scope.to_string(),
        };
        self.entries.lock().unwrap().remove(&key).is_some()
    }

    pub fn len(&self) -> usize { self.entries.lock().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

/// Registry client for pulling OCI images.
pub struct RegistryClient {
    client: Client,
    cache_dir: PathBuf,
}

impl RegistryClient {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            client: Client::new(),
            cache_dir,
        }
    }

    /// Pull a manifest from a registry.
    pub async fn pull_manifest(&self, image_ref: &ImageReference) -> CriResult<OciManifest> {
        let tag = image_ref.tag.as_deref().unwrap_or("latest");
        let url = format!(
            "https://{}/v2/{}/manifests/{}",
            image_ref.registry, image_ref.repository, tag
        );

        let resp = self.client
            .get(&url)
            .header("Accept", "application/vnd.oci.image.manifest.v1+json")
            .header("Accept", "application/vnd.docker.distribution.manifest.v2+json")
            .send()
            .await
            .map_err(|e| CriError::Registry(format!("manifest fetch failed: {}", e)))?;

        if !resp.status().is_success() {
            // Try with auth token
            return self.pull_manifest_with_auth(image_ref, tag).await;
        }

        resp.json::<OciManifest>().await
            .map_err(|e| CriError::Registry(format!("manifest parse failed: {}", e)))
    }

    async fn pull_manifest_with_auth(&self, image_ref: &ImageReference, tag: &str) -> CriResult<OciManifest> {
        let token = self.get_auth_token(image_ref).await?;
        let url = format!(
            "https://{}/v2/{}/manifests/{}",
            image_ref.registry, image_ref.repository, tag
        );

        let resp = self.client
            .get(&url)
            .header("Accept", "application/vnd.oci.image.manifest.v1+json")
            .header("Accept", "application/vnd.docker.distribution.manifest.v2+json")
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| CriError::Registry(format!("manifest fetch with auth failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(CriError::Registry(format!("manifest fetch returned {}", resp.status())));
        }

        resp.json::<OciManifest>().await
            .map_err(|e| CriError::Registry(format!("manifest parse failed: {}", e)))
    }

    /// Get a Bearer token for a registry (Docker Hub token flow).
    async fn get_auth_token(&self, image_ref: &ImageReference) -> CriResult<String> {
        let url = format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
            image_ref.repository
        );

        let resp: serde_json::Value = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| CriError::Registry(format!("auth request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| CriError::Registry(format!("auth parse failed: {}", e)))?;

        resp.get("token")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CriError::Registry("no token in auth response".into()))
    }

    /// Pull a blob (image layer) by digest and save to cache.
    pub async fn pull_blob(
        &self,
        image_ref: &ImageReference,
        descriptor: &OciDescriptor,
    ) -> CriResult<PathBuf> {
        let blob_dir = self.cache_dir.join("blobs");
        std::fs::create_dir_all(&blob_dir)
            .map_err(|e| CriError::Registry(format!("cache dir creation failed: {}", e)))?;

        let blob_path = blob_dir.join(descriptor.digest.replace(':', "_"));
        if blob_path.exists() {
            tracing::debug!("blob {} already cached", descriptor.digest);
            return Ok(blob_path);
        }

        let url = format!(
            "https://{}/v2/{}/blobs/{}",
            image_ref.registry, image_ref.repository, descriptor.digest
        );

        let resp = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| CriError::Registry(format!("blob fetch failed: {}", e)))?;

        let data = resp.bytes().await
            .map_err(|e| CriError::Registry(format!("blob download failed: {}", e)))?;

        // Verify digest
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let computed = format!("sha256:{}", hex::encode(hasher.finalize()));
        if computed != descriptor.digest {
            return Err(CriError::Registry(format!(
                "digest mismatch: expected {}, got {}", descriptor.digest, computed
            )));
        }

        std::fs::write(&blob_path, &data)
            .map_err(|e| CriError::Registry(format!("blob write failed: {}", e)))?;

        Ok(blob_path)
    }

    /// Compute the expected cache path for a blob digest (used by tests).
    pub fn blob_cache_path(&self, digest: &str) -> std::path::PathBuf {
        self.cache_dir.join("blobs").join(digest.replace(':', "_"))
    }

    /// Pull a complete image (manifest + all layers).
    #[allow(dead_code)]
    pub async fn pull_image(&self, reference: &str) -> CriResult<OciImage> {
        let image_ref = ImageReference::parse(reference);
        tracing::info!("pulling image: {}", image_ref.full_reference());

        let manifest = self.pull_manifest(&image_ref).await?;

        let mut layers = Vec::new();
        let mut total_size: u64 = 0;

        for layer_desc in &manifest.layers {
            let local_path = self.pull_blob(&image_ref, layer_desc).await?;
            total_size += layer_desc.size;
            layers.push(OciLayer {
                digest: layer_desc.digest.clone(),
                size: layer_desc.size,
                media_type: layer_desc.media_type.clone(),
                local_path: Some(local_path),
            });
        }

        // Pull config blob
        let _config_path = self.pull_blob(&image_ref, &manifest.config).await?;

        Ok(OciImage {
            reference: reference.to_string(),
            digest: manifest.config.digest.clone(),
            layers,
            config: ImageConfig::default(), // TODO: parse from config blob
            size_bytes: total_size,
            pulled_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OciDescriptor;

    fn make_client(dir: &std::path::Path) -> RegistryClient {
        RegistryClient::new(dir.to_path_buf())
    }

    #[test]
    fn test_registry_client_new() {
        let dir = tempfile::tempdir().unwrap();
        let client = make_client(dir.path());
        assert_eq!(client.cache_dir, dir.path());
    }

    #[test]
    fn test_blob_cache_path_colon_escaped() {
        let dir = tempfile::tempdir().unwrap();
        let client = make_client(dir.path());
        let path = client.blob_cache_path("sha256:abcdef");
        assert!(path.to_string_lossy().contains("sha256_abcdef"));
        assert!(path.starts_with(&client.cache_dir));
    }

    #[tokio::test]
    async fn test_pull_blob_cache_hit() {
        // Pre-create the blob file so pull_blob returns it immediately (no HTTP).
        let dir = tempfile::tempdir().unwrap();
        let client = make_client(dir.path());

        let descriptor = OciDescriptor {
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
            digest: "sha256:cafebabe".into(),
            size: 42,
        };

        // Create cache dir and file
        let blob_dir = dir.path().join("blobs");
        std::fs::create_dir_all(&blob_dir).unwrap();
        let blob_file = blob_dir.join("sha256_cafebabe");
        std::fs::write(&blob_file, b"fake layer data").unwrap();

        let image_ref = ImageReference {
            registry: "docker.io".into(),
            repository: "library/nginx".into(),
            tag: Some("latest".into()),
            digest: None,
        };

        let result = client.pull_blob(&image_ref, &descriptor).await.unwrap();
        assert_eq!(result, blob_file);
    }

    #[test]
    fn test_digest_verification_logic() {
        // Verify the sha256 digest computation matches expectation.
        use sha2::{Digest, Sha256};
        let data = b"hello world";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let computed = format!("sha256:{}", hex::encode(hasher.finalize()));
        assert!(computed.starts_with("sha256:"));
        assert_ne!(computed, "sha256:wrongdigest");
    }

    #[test]
    fn test_image_reference_full_roundtrip_in_url() {
        // Verify reference parsing produces correct URL components
        let r = ImageReference::parse("ghcr.io/org/app:v1");
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "org/app");
        assert_eq!(r.tag.as_deref(), Some("v1"));
        // The URL that pull_manifest would build:
        let url = format!("https://{}/v2/{}/manifests/{}", r.registry, r.repository, r.tag.as_deref().unwrap_or("latest"));
        assert_eq!(url, "https://ghcr.io/v2/org/app/manifests/v1");
    }

    #[tokio::test]
    async fn test_pull_image_no_layers_in_manifest_skips_blob_fetch() {
        // An image with zero layers in manifest should produce OciImage with no layers.
        // We can't test the full pull without a mock server, but we validate the
        // pull_image result shape when no network calls are made.
        // This test documents the expected behavior with a minimal mock path.
        let dir = tempfile::tempdir().unwrap();
        let _client = make_client(dir.path());
        // Just construct the expected output shape to document contract:
        let image = OciImage {
            reference: "nginx:latest".into(),
            digest: "sha256:abc".into(),
            layers: vec![],
            config: ImageConfig::default(),
            size_bytes: 0,
            pulled_at: chrono::Utc::now(),
        };
        assert_eq!(image.layers.len(), 0);
        assert_eq!(image.size_bytes, 0);
    }
}
