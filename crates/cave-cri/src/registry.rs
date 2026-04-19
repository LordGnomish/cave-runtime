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

    /// Pull a complete image (manifest + all layers).
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
