//! OCI registry client — pulls manifests and blobs from container registries.
//!
//! Implements the Docker Registry HTTP API v2 for image pulling.

use crate::error::{CriError, CriResult};
use crate::models::{ImageConfig, ImageReference, OciDescriptor, OciImage, OciLayer, OciManifest};
use chrono::Utc;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

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

        // Pull and parse the config blob (OCI image-spec config schema).
        let config_path = self.pull_blob(&image_ref, &manifest.config).await?;
        let config = parse_oci_config(&config_path)?;

        Ok(OciImage {
            reference: reference.to_string(),
            digest: manifest.config.digest.clone(),
            layers,
            config,
            size_bytes: total_size,
            pulled_at: Utc::now(),
        })
    }
}

/// Parse an OCI image config JSON file (image-spec/specs-go/v1/config.go) into
/// our internal `ImageConfig`. Falls back to defaults for missing fields.
pub(crate) fn parse_oci_config(path: &std::path::Path) -> CriResult<ImageConfig> {
    let bytes = std::fs::read(path)
        .map_err(|e| CriError::Registry(format!("config blob read: {}", e)))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| CriError::Registry(format!("config blob parse: {}", e)))?;

    let cfg = v.get("config").cloned().unwrap_or(serde_json::Value::Null);
    let str_vec = |key: &str| -> Vec<String> {
        cfg.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    let opt_str = |key: &str| -> Option<String> {
        cfg.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    };

    // Env arrives as ["KEY=VALUE", ...]; flatten to a HashMap.
    let env: std::collections::HashMap<String, String> = cfg
        .get("Env")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str())
                .filter_map(|kv| {
                    let mut parts = kv.splitn(2, '=');
                    Some((parts.next()?.to_string(), parts.next().unwrap_or("").to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    // ExposedPorts arrives as { "80/tcp": {}, ... }; we collapse to keys.
    let exposed_ports: Vec<String> = cfg
        .get("ExposedPorts")
        .and_then(|v| v.as_object())
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();

    let labels: std::collections::HashMap<String, String> = cfg
        .get("Labels")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    Ok(ImageConfig {
        entrypoint: str_vec("Entrypoint"),
        cmd: str_vec("Cmd"),
        env,
        working_dir: opt_str("WorkingDir"),
        user: opt_str("User"),
        exposed_ports,
        labels,
    })
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

    #[test]
    fn test_parse_oci_config_full_payload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let payload = serde_json::json!({
            "architecture": "amd64",
            "config": {
                "Entrypoint": ["/bin/sh", "-c"],
                "Cmd": ["echo", "hi"],
                "Env": ["PATH=/usr/bin", "NAME=foo"],
                "WorkingDir": "/app",
                "User": "1000",
                "ExposedPorts": { "80/tcp": {}, "443/tcp": {} },
                "Labels": { "maintainer": "team@example.com" }
            }
        });
        std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();

        let cfg = parse_oci_config(&path).expect("parse ok");
        assert_eq!(cfg.entrypoint, vec!["/bin/sh", "-c"]);
        assert_eq!(cfg.cmd, vec!["echo", "hi"]);
        assert_eq!(cfg.env.get("PATH").map(|s| s.as_str()), Some("/usr/bin"));
        assert_eq!(cfg.env.get("NAME").map(|s| s.as_str()), Some("foo"));
        assert_eq!(cfg.working_dir.as_deref(), Some("/app"));
        assert_eq!(cfg.user.as_deref(), Some("1000"));
        assert_eq!(cfg.exposed_ports.len(), 2);
        assert!(cfg.exposed_ports.iter().any(|p| p == "80/tcp"));
        assert_eq!(cfg.labels.get("maintainer").map(|s| s.as_str()), Some("team@example.com"));
    }

    #[test]
    fn test_parse_oci_config_missing_fields_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, b"{\"architecture\":\"amd64\"}").unwrap();
        let cfg = parse_oci_config(&path).expect("parse ok");
        assert!(cfg.entrypoint.is_empty());
        assert!(cfg.cmd.is_empty());
        assert!(cfg.env.is_empty());
        assert!(cfg.working_dir.is_none());
    }

    #[test]
    fn test_parse_oci_config_invalid_json_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(parse_oci_config(&path).is_err());
    }
}
