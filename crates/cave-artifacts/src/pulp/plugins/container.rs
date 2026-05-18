// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_container — Container image content plugin (OCI / Docker).
//!
//! Implements:
//! - OCI image-manifest v1.1.0 reader (`parse_oci_manifest`).
//! - Docker manifest-v2 schema 2 reader (same function, kind-discriminated).
//! - OCI image-index v1 reader (`parse_oci_manifest_list`).
//! - Descriptor digest validation (`OciDescriptor::validate_digest`).
//!
//! Upstream parity: pulp/pulp_container `pulp_container/app/models.py`
//! + OCI distribution-spec v1.1.0.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct ContainerPlugin;

// ── OCI types ────────────────────────────────────────────────────────────────

pub const MT_DOCKER_V2: &str = "application/vnd.docker.distribution.manifest.v2+json";
pub const MT_OCI_V1: &str = "application/vnd.oci.image.manifest.v1+json";
pub const MT_OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const MT_DOCKER_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    DockerV2,
    OciV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OciDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    pub digest: String,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

impl OciDescriptor {
    /// Validate that `digest` looks like `<algo>:<hex>` with hex length
    /// appropriate for the algorithm.
    pub fn validate_digest(&self) -> Result<(), ArtifactsError> {
        let (algo, hex_part) = self
            .digest
            .split_once(':')
            .ok_or_else(|| ArtifactsError::InvalidRequest(format!("digest '{}' missing ':'", self.digest)))?;
        let need = match algo {
            "sha256" => 64,
            "sha512" => 128,
            other => {
                return Err(ArtifactsError::InvalidRequest(format!(
                    "unsupported digest algorithm '{other}'"
                )))
            }
        };
        if hex_part.len() != need || !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ArtifactsError::InvalidRequest(format!(
                "digest '{}' bad hex length for {}",
                self.digest, algo
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciManifest {
    pub kind: ManifestKind,
    pub schema_version: u32,
    pub media_type: String,
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OciPlatform {
    pub architecture: String,
    pub os: String,
    #[serde(default, rename = "os.version")]
    pub os_version: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciIndexEntry {
    pub descriptor: OciDescriptor,
    pub platform: OciPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciManifestIndex {
    pub schema_version: u32,
    pub media_type: String,
    pub manifests: Vec<OciIndexEntry>,
}

// Internal serde shapes (private) — kept distinct from the public reader
// types so we can validate / normalize before exposing.

#[derive(Deserialize)]
struct RawManifest {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "mediaType", default)]
    media_type: Option<String>,
    config: OciDescriptor,
    #[serde(default)]
    layers: Vec<OciDescriptor>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RawIndex {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "mediaType", default)]
    media_type: Option<String>,
    #[serde(default)]
    manifests: Vec<RawIndexEntry>,
}

#[derive(Deserialize)]
struct RawIndexEntry {
    #[serde(rename = "mediaType")]
    media_type: String,
    size: u64,
    digest: String,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    #[serde(default)]
    platform: Option<OciPlatform>,
}

/// Parse an OCI v1 or Docker v2 schema-2 image manifest.
pub fn parse_oci_manifest(bytes: &[u8]) -> Result<OciManifest, ArtifactsError> {
    let raw: RawManifest = serde_json::from_slice(bytes)
        .map_err(|e| ArtifactsError::InvalidRequest(format!("invalid manifest JSON: {e}")))?;
    if raw.schema_version != 2 {
        return Err(ArtifactsError::InvalidRequest(format!(
            "schemaVersion {} not supported (expect 2)",
            raw.schema_version
        )));
    }
    let media_type = raw
        .media_type
        .clone()
        .unwrap_or_else(|| MT_OCI_V1.to_string());
    let kind = match media_type.as_str() {
        MT_DOCKER_V2 => ManifestKind::DockerV2,
        MT_OCI_V1 => ManifestKind::OciV1,
        // Default to OCI v1 if not specified (per OCI distribution spec).
        _ => {
            if raw.media_type.is_none() {
                ManifestKind::OciV1
            } else {
                return Err(ArtifactsError::InvalidRequest(format!(
                    "unsupported manifest mediaType: {media_type}"
                )));
            }
        }
    };
    raw.config.validate_digest()?;
    for l in &raw.layers {
        l.validate_digest()?;
    }
    Ok(OciManifest {
        kind,
        schema_version: raw.schema_version,
        media_type,
        config: raw.config,
        layers: raw.layers,
        annotations: raw.annotations,
    })
}

/// Parse an OCI image-index (multi-arch manifest list).
pub fn parse_oci_manifest_list(bytes: &[u8]) -> Result<OciManifestIndex, ArtifactsError> {
    let raw: RawIndex = serde_json::from_slice(bytes)
        .map_err(|e| ArtifactsError::InvalidRequest(format!("invalid index JSON: {e}")))?;
    if raw.schema_version != 2 {
        return Err(ArtifactsError::InvalidRequest(format!(
            "schemaVersion {} not supported (expect 2)",
            raw.schema_version
        )));
    }
    let media_type = raw
        .media_type
        .clone()
        .unwrap_or_else(|| MT_OCI_INDEX.to_string());
    let mut manifests = Vec::with_capacity(raw.manifests.len());
    for m in raw.manifests {
        let descriptor = OciDescriptor {
            media_type: m.media_type,
            size: m.size,
            digest: m.digest,
            annotations: m.annotations,
        };
        descriptor.validate_digest()?;
        let platform = m.platform.ok_or_else(|| {
            ArtifactsError::InvalidRequest("manifest index entry missing platform".into())
        })?;
        manifests.push(OciIndexEntry { descriptor, platform });
    }
    Ok(OciManifestIndex {
        schema_version: raw.schema_version,
        media_type,
        manifests,
    })
}

// ── Plugin ───────────────────────────────────────────────────────────────────

impl ArtifactsPlugin for ContainerPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Container
    }

    fn name(&self) -> &str {
        "pulp_container"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["container.manifest", "container.blob", "container.tag"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        // Try to read it as a manifest. The relative_path classifies if we miss.
        let (content_type, manifest_meta) = if let Ok(m) = parse_oci_manifest(data) {
            (
                "manifest",
                Some(serde_json::json!({
                    "kind": match m.kind {
                        ManifestKind::DockerV2 => "docker.v2",
                        ManifestKind::OciV1 => "oci.v1",
                    },
                    "config_digest": m.config.digest,
                    "config_size": m.config.size,
                    "layer_count": m.layers.len(),
                    "layers": m.layers.iter().map(|l| serde_json::json!({
                        "digest": l.digest,
                        "size": l.size,
                        "media_type": l.media_type,
                    })).collect::<Vec<_>>(),
                })),
            )
        } else if let Ok(idx) = parse_oci_manifest_list(data) {
            (
                "manifest_list",
                Some(serde_json::json!({
                    "kind": "oci.index",
                    "entries": idx.manifests.iter().map(|e| serde_json::json!({
                        "digest": e.descriptor.digest,
                        "size": e.descriptor.size,
                        "platform": {
                            "architecture": e.platform.architecture,
                            "os": e.platform.os,
                        }
                    })).collect::<Vec<_>>(),
                })),
            )
        } else if relative_path.contains("manifests") {
            ("manifest", None)
        } else {
            ("blob", None)
        };

        let mut md = serde_json::json!({
            "content_type": content_type,
            "digest": format!("sha256:{sha256}"),
            "relative_path": relative_path,
        });
        if let Some(extra) = manifest_meta {
            md["manifest"] = extra;
        }
        let mut unit = ContentUnit::new(PluginType::Container, md);
        unit.relative_path = Some(relative_path.to_string());
        unit.sha256 = Some(sha256);
        unit.size = Some(data.len() as u64);
        Ok(unit)
    }

    fn generate_metadata(
        &self,
        _repo_version: &RepositoryVersion,
        units: &[ContentUnit],
    ) -> serde_json::Value {
        let manifests: Vec<_> = units
            .iter()
            .filter(|u| u.metadata["content_type"] == "manifest")
            .map(|u| serde_json::json!({ "digest": u.metadata["digest"] }))
            .collect();
        let manifest_lists: Vec<_> = units
            .iter()
            .filter(|u| u.metadata["content_type"] == "manifest_list")
            .map(|u| serde_json::json!({ "digest": u.metadata["digest"] }))
            .collect();
        let blobs: Vec<_> = units
            .iter()
            .filter(|u| u.metadata["content_type"] == "blob")
            .map(|u| serde_json::json!({ "digest": u.metadata["digest"] }))
            .collect();
        serde_json::json!({
            "manifests": manifests,
            "manifest_lists": manifest_lists,
            "blobs": blobs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_blob_classified_as_blob() {
        let plugin = ContainerPlugin;
        let u = plugin
            .parse_content(b"opaque bytes", "v2/library/foo/blobs/sha256:abc")
            .unwrap();
        assert_eq!(u.metadata["content_type"], "blob");
    }
}
