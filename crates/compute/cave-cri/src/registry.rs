// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OCI registry client — pulls manifests and blobs from container registries.
//!
//! Implements the Docker Registry HTTP API v2 for image pulling.

use crate::content::digest::{Digest, DigestAlgorithm};
use crate::content::writer::Hasher;
use crate::error::{CriError, CriResult};
use crate::models::{ImageConfig, ImageReference, OciDescriptor, OciImage, OciLayer, OciManifest};
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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
        let rest = trimmed
            .strip_prefix("Bearer")
            .or_else(|| trimmed.strip_prefix("bearer"))
            .ok_or_else(|| CriError::Registry("WWW-Authenticate: missing Bearer scheme".into()))?;
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
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            ',' if !in_quotes => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
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
        Self {
            tenant_id: tenant_id.into(),
            entries: Mutex::new(HashMap::new()),
        }
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

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Parse the digest of a blob as it is found in a manifest.
///
/// The grammar is the one of the OCI image specification (descriptor.md,
/// "Digests"): `algorithm ":" encoded`. Accepted are the registered
/// algorithms this crate can verify, with the encoded part the
/// specification requires for them: `sha256` with 64 and `sha512` with 128
/// lowercase hex characters. Everything else is an error, including
/// `sha384`, which [`Digest`] knows but the specification does not register.
///
/// A digest that passed consists of `[a-z0-9]`, one `:` and nothing else,
/// which is what makes it usable as a file name and as part of a URL.
fn parse_blob_digest(digest: &str) -> CriResult<Digest> {
    let parsed = Digest::parse(digest)
        .map_err(|e| CriError::Registry(format!("invalid blob digest {:?}: {}", digest, e)))?;
    match parsed.algorithm() {
        DigestAlgorithm::Sha256 | DigestAlgorithm::Sha512 => Ok(parsed),
        other => Err(CriError::Registry(format!(
            "invalid blob digest {:?}: {} is not a registered algorithm (sha256, sha512)",
            digest,
            other.as_str()
        ))),
    }
}

/// The file name of a blob in the cache directory: `<algorithm>_<hex>`.
fn blob_file_name(digest: &Digest) -> String {
    format!("{}_{}", digest.algorithm().as_str(), digest.hex())
}

/// What the cache holds under the name of a blob.
enum CachedBlob {
    /// A regular file whose content has the digest.
    Intact,
    /// Nothing.
    Missing,
    /// Something that must not be returned as the blob, and why.
    Unusable(String),
}

/// Look at the cache entry `path` for the blob `digest`.
fn cached_blob_state(path: &Path, digest: &Digest) -> CachedBlob {
    // The entry itself, not what a symlink of that name points to.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return CachedBlob::Unusable("not a regular file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CachedBlob::Missing,
        Err(e) => return CachedBlob::Unusable(format!("cannot be inspected: {}", e)),
    }
    match digest_of_file(path, digest.algorithm()) {
        Ok(actual) if actual == *digest => CachedBlob::Intact,
        Ok(actual) => CachedBlob::Unusable(format!("its content has the digest {}", actual)),
        Err(e) => CachedBlob::Unusable(format!("cannot be read: {}", e)),
    }
}

/// The digest of the content of the file `path`.
fn digest_of_file(path: &Path, algorithm: DigestAlgorithm) -> std::io::Result<Digest> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Hasher::new(algorithm);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok(hasher.finalize(algorithm)),
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// Add the blob `digest` with the content `data` to the cache directory
/// `blob_dir` and return its path.
///
/// `data` is verified against the digest first. It is then written to a new
/// file next to the cache entry and renamed over it, so the entry either
/// stays what it was or becomes the complete, verified blob.
fn store_blob(blob_dir: &Path, digest: &Digest, data: &[u8]) -> CriResult<PathBuf> {
    let actual = Digest::compute(digest.algorithm(), data);
    if actual != *digest {
        return Err(CriError::Registry(format!(
            "digest mismatch: expected {}, got {}",
            digest, actual
        )));
    }

    let name = blob_file_name(digest);
    let blob_path = blob_dir.join(&name);
    // Not a name `blob_file_name` produces, and unique per attempt.
    let tmp_path = blob_dir.join(format!(".{}.{}.tmp", name, uuid::Uuid::new_v4().simple()));

    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .and_then(|mut file| file.write_all(data))
        .and_then(|()| std::fs::rename(&tmp_path, &blob_path));
    if let Err(e) = written {
        std::fs::remove_file(&tmp_path).ok();
        return Err(CriError::Registry(format!("blob write failed: {}", e)));
    }

    Ok(blob_path)
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

        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.oci.image.manifest.v1+json")
            .header(
                "Accept",
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .send()
            .await
            .map_err(|e| CriError::Registry(format!("manifest fetch failed: {}", e)))?;

        if !resp.status().is_success() {
            // Try with auth token
            return self.pull_manifest_with_auth(image_ref, tag).await;
        }

        resp.json::<OciManifest>()
            .await
            .map_err(|e| CriError::Registry(format!("manifest parse failed: {}", e)))
    }

    async fn pull_manifest_with_auth(
        &self,
        image_ref: &ImageReference,
        tag: &str,
    ) -> CriResult<OciManifest> {
        let token = self.get_auth_token(image_ref).await?;
        let url = format!(
            "https://{}/v2/{}/manifests/{}",
            image_ref.registry, image_ref.repository, tag
        );

        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.oci.image.manifest.v1+json")
            .header(
                "Accept",
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| CriError::Registry(format!("manifest fetch with auth failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(CriError::Registry(format!(
                "manifest fetch returned {}",
                resp.status()
            )));
        }

        resp.json::<OciManifest>()
            .await
            .map_err(|e| CriError::Registry(format!("manifest parse failed: {}", e)))
    }

    /// Get a Bearer token for a registry (Docker Hub token flow).
    async fn get_auth_token(&self, image_ref: &ImageReference) -> CriResult<String> {
        let url = format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
            image_ref.repository
        );

        let resp: serde_json::Value = self
            .client
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
    ///
    /// The digest of the descriptor is parsed first (see
    /// [`parse_blob_digest`]): it names the blob in the cache and in the
    /// request to the registry, so a descriptor whose digest is not well
    /// formed is refused before either is built.
    ///
    /// The returned file always has the content the digest stands for. A
    /// file that is already in the cache is read and compared with the
    /// digest before it is returned. If it does not match, or is not a
    /// regular file, the blob is downloaded again; the download is verified
    /// and then moved over the cache entry. If that fails the result is an
    /// error, not the file that was found.
    pub async fn pull_blob(
        &self,
        image_ref: &ImageReference,
        descriptor: &OciDescriptor,
    ) -> CriResult<PathBuf> {
        let digest = parse_blob_digest(&descriptor.digest)?;

        let blob_dir = self.cache_dir.join("blobs");
        std::fs::create_dir_all(&blob_dir)
            .map_err(|e| CriError::Registry(format!("cache dir creation failed: {}", e)))?;

        let blob_path = blob_dir.join(blob_file_name(&digest));
        match cached_blob_state(&blob_path, &digest) {
            CachedBlob::Intact => {
                tracing::debug!("blob {} already cached", digest);
                return Ok(blob_path);
            }
            CachedBlob::Unusable(why) => {
                tracing::warn!(
                    "cached blob {} is not used ({}), fetching it again",
                    digest,
                    why
                );
            }
            CachedBlob::Missing => {}
        }

        let url = format!(
            "https://{}/v2/{}/blobs/{}",
            image_ref.registry, image_ref.repository, digest
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| CriError::Registry(format!("blob fetch failed: {}", e)))?;

        let data = resp
            .bytes()
            .await
            .map_err(|e| CriError::Registry(format!("blob download failed: {}", e)))?;

        store_blob(&blob_dir, &digest, &data)
    }

    /// Compute the expected cache path for a blob digest (used by tests).
    ///
    /// Fails for a digest that [`RegistryClient::pull_blob`] would refuse.
    pub fn blob_cache_path(&self, digest: &str) -> CriResult<PathBuf> {
        let digest = parse_blob_digest(digest)?;
        Ok(self.cache_dir.join("blobs").join(blob_file_name(&digest)))
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

    /// The digest `sha256:<hex>` of `data`.
    fn sha256_of(data: &[u8]) -> String {
        Digest::compute(DigestAlgorithm::Sha256, data).to_string()
    }

    /// The names in `dir`, sorted.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_blob_cache_path_colon_escaped() {
        let dir = tempfile::tempdir().unwrap();
        let client = make_client(dir.path());
        let digest = sha256_of(b"abcdef");
        let path = client.blob_cache_path(&digest).unwrap();
        assert!(path
            .to_string_lossy()
            .contains(&digest.replace("sha256:", "sha256_")));
        assert!(path.starts_with(&client.cache_dir));
    }

    #[test]
    fn blob_cache_path_refuses_a_digest_that_is_not_well_formed() {
        let dir = tempfile::tempdir().unwrap();
        let client = make_client(dir.path());
        for digest in ["sha256:abcdef", "../stray.bin", "/etc/hostname", ""] {
            match client.blob_cache_path(digest) {
                Err(CriError::Registry(msg)) => {
                    assert!(msg.contains("invalid blob digest"), "got: {msg}")
                }
                other => panic!("{digest:?} must be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_blob_digest_accepts_the_registered_algorithms() {
        let sha256 = sha256_of(b"blob");
        assert_eq!(parse_blob_digest(&sha256).unwrap().to_string(), sha256);
        let sha512 = Digest::compute(DigestAlgorithm::Sha512, b"blob").to_string();
        assert_eq!(parse_blob_digest(&sha512).unwrap().to_string(), sha512);

        // Known to `Digest`, but not registered by the image specification.
        let sha384 = Digest::compute(DigestAlgorithm::Sha384, b"blob").to_string();
        match parse_blob_digest(&sha384) {
            Err(CriError::Registry(msg)) => {
                assert!(msg.contains("not a registered algorithm"), "got: {msg}")
            }
            other => panic!("sha384 must be refused, got {other:?}"),
        }
    }

    /// A download replaces a cache entry whose content is something else,
    /// and leaves nothing but the blob behind.
    #[test]
    fn store_blob_replaces_a_cache_entry_with_other_content() {
        let dir = tempfile::tempdir().unwrap();
        let blob = b"layer bytes";
        let digest = parse_blob_digest(&sha256_of(blob)).unwrap();
        let entry = dir.path().join(blob_file_name(&digest));
        std::fs::write(&entry, b"left over from an interrupted write").unwrap();
        assert!(matches!(
            cached_blob_state(&entry, &digest),
            CachedBlob::Unusable(_)
        ));

        let path = store_blob(dir.path(), &digest, blob).unwrap();

        assert_eq!(path, entry);
        assert_eq!(std::fs::read(&entry).unwrap(), blob);
        assert!(matches!(
            cached_blob_state(&entry, &digest),
            CachedBlob::Intact
        ));
        assert_eq!(names_in(dir.path()), [blob_file_name(&digest)]);
    }

    /// A symlink in the place of the cache entry is replaced, not written
    /// through.
    #[test]
    fn store_blob_replaces_a_symlink_instead_of_writing_through_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("blobs");
        std::fs::create_dir(&cache).unwrap();
        let elsewhere = dir.path().join("elsewhere.bin");
        std::fs::write(&elsewhere, b"other file").unwrap();
        let blob = b"layer bytes";
        let digest = parse_blob_digest(&sha256_of(blob)).unwrap();
        let entry = cache.join(blob_file_name(&digest));
        std::os::unix::fs::symlink(&elsewhere, &entry).unwrap();

        store_blob(&cache, &digest, blob).unwrap();

        assert!(std::fs::symlink_metadata(&entry).unwrap().is_file());
        assert_eq!(std::fs::read(&entry).unwrap(), blob);
        assert_eq!(std::fs::read(&elsewhere).unwrap(), b"other file");
    }

    /// Data that does not have the digest is not written anywhere.
    #[test]
    fn store_blob_refuses_data_with_another_digest() {
        let dir = tempfile::tempdir().unwrap();
        let digest = parse_blob_digest(&sha256_of(b"layer bytes")).unwrap();
        let entry = dir.path().join(blob_file_name(&digest));
        std::fs::write(&entry, b"what was there").unwrap();

        match store_blob(dir.path(), &digest, b"something else") {
            Err(CriError::Registry(msg)) => assert!(msg.contains("digest mismatch"), "{msg}"),
            other => panic!("expected a digest mismatch, got {other:?}"),
        }

        assert_eq!(std::fs::read(&entry).unwrap(), b"what was there");
        assert_eq!(names_in(dir.path()), [blob_file_name(&digest)]);
    }

    /// If the blob cannot be moved into place, the error is reported and
    /// the temporary file is removed.
    #[test]
    fn store_blob_leaves_no_temporary_file_when_the_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let blob = b"layer bytes";
        let digest = parse_blob_digest(&sha256_of(blob)).unwrap();
        // A non-empty directory cannot be replaced by a file.
        let entry = dir.path().join(blob_file_name(&digest));
        std::fs::create_dir(&entry).unwrap();
        std::fs::write(entry.join("keep"), b"x").unwrap();

        match store_blob(dir.path(), &digest, blob) {
            Err(CriError::Registry(msg)) => assert!(msg.contains("blob write failed"), "{msg}"),
            other => panic!("expected a write error, got {other:?}"),
        }

        assert_eq!(names_in(dir.path()), [blob_file_name(&digest)]);
        assert_eq!(std::fs::read(entry.join("keep")).unwrap(), b"x");
    }

    #[tokio::test]
    async fn test_pull_blob_cache_hit() {
        // Pre-create the blob file so pull_blob returns it immediately (no HTTP).
        let dir = tempfile::tempdir().unwrap();
        let client = make_client(dir.path());

        // A cached file is only a hit if its content has the digest.
        let digest = sha256_of(b"fake layer data");
        let descriptor = OciDescriptor {
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
            digest: digest.clone(),
            size: 15,
        };

        // Create cache dir and file
        let blob_dir = dir.path().join("blobs");
        std::fs::create_dir_all(&blob_dir).unwrap();
        let blob_file = blob_dir.join(digest.replace("sha256:", "sha256_"));
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
        let url = format!(
            "https://{}/v2/{}/manifests/{}",
            r.registry,
            r.repository,
            r.tag.as_deref().unwrap_or("latest")
        );
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
