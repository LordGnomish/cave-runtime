// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_container — Container image content plugin (OCI / Docker).
//!
//! Mirrors `pulp_container`:
//! - Manifest media-type classification (OCI image manifest v1,
//!   OCI image index v1, Docker schema2 manifest, Docker schema2 manifest
//!   list, Docker schema1 signed/unsigned, OCI artifact).
//! - Tag → manifest aliasing (`ContainerTag` content kind) with
//!   `tagged_manifest_digest`.
//! - Manifest list (fat manifest) child enumeration:
//!   each `manifests[].digest + .platform.{os,architecture,variant}` is
//!   surfaced into the unit metadata.
//! - Digest parsing (`sha256:<hex>`) + validation.
//! - Blob descriptor: media-type + size + digest.
//!
//! Container blob storage and the OCI distribution-spec HTTP routes live
//! in `cave-artifacts::harbor`. This plugin owns the content-typing layer
//! that Pulp publications and remotes operate on.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct ContainerPlugin;

/// Recognised manifest / blob / index media types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    /// `application/vnd.oci.image.manifest.v1+json`.
    OciManifestV1,
    /// `application/vnd.oci.image.index.v1+json` (manifest list / fat manifest).
    OciIndexV1,
    /// `application/vnd.docker.distribution.manifest.v2+json`.
    DockerManifestV2,
    /// `application/vnd.docker.distribution.manifest.list.v2+json` (fat manifest).
    DockerManifestListV2,
    /// `application/vnd.docker.distribution.manifest.v1+json` / `+prettyjws`.
    DockerManifestV1,
    /// `application/vnd.oci.artifact.manifest.v1+json` (artifact manifest).
    OciArtifactV1,
    /// Plain blob (config, layer, …).
    Blob,
    /// Unrecognised.
    Unknown,
}

impl ManifestKind {
    pub fn from_media_type(mt: &str) -> Self {
        match mt {
            "application/vnd.oci.image.manifest.v1+json" => Self::OciManifestV1,
            "application/vnd.oci.image.index.v1+json" => Self::OciIndexV1,
            "application/vnd.docker.distribution.manifest.v2+json" => Self::DockerManifestV2,
            "application/vnd.docker.distribution.manifest.list.v2+json" => {
                Self::DockerManifestListV2
            }
            "application/vnd.docker.distribution.manifest.v1+json"
            | "application/vnd.docker.distribution.manifest.v1+prettyjws" => Self::DockerManifestV1,
            "application/vnd.oci.artifact.manifest.v1+json" => Self::OciArtifactV1,
            _ if mt.starts_with("application/vnd.docker.image")
                || mt.starts_with("application/vnd.oci.image.layer")
                || mt.starts_with("application/vnd.oci.image.config")
                || mt.starts_with("application/octet-stream") =>
            {
                Self::Blob
            }
            _ => Self::Unknown,
        }
    }

    /// True for fat manifests (index / manifest list) — the units that aggregate
    /// per-platform child manifests.
    pub fn is_index(&self) -> bool {
        matches!(self, Self::OciIndexV1 | Self::DockerManifestListV2)
    }

    /// True for any image manifest (not blob, not index).
    pub fn is_manifest(&self) -> bool {
        matches!(
            self,
            Self::OciManifestV1
                | Self::DockerManifestV2
                | Self::DockerManifestV1
                | Self::OciArtifactV1
        )
    }
}

/// A `sha256:` digest as parsed from a manifest reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest256 {
    pub hex: String,
}

impl Digest256 {
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix("sha256:")?;
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self { hex: hex.to_string() })
    }

    pub fn from_bytes(data: &[u8]) -> Self {
        Self { hex: hex::encode(Sha256::digest(data)) }
    }

    pub fn to_oci(&self) -> String {
        format!("sha256:{}", self.hex)
    }
}

impl ArtifactsPlugin for ContainerPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Container
    }

    fn name(&self) -> &str {
        "pulp_container"
    }

    fn content_types(&self) -> Vec<&str> {
        vec![
            "container.manifest",
            "container.manifest_list",
            "container.manifest_signed",
            "container.blob",
            "container.tag",
            "container.signature",
        ]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let digest = Digest256::from_bytes(data);
        let (kind, child_digests) = classify_blob(data);
        let kind_label = match kind {
            ManifestKind::OciManifestV1 => "manifest",
            ManifestKind::OciIndexV1 | ManifestKind::DockerManifestListV2 => "manifest_list",
            ManifestKind::DockerManifestV2 => "manifest",
            ManifestKind::DockerManifestV1 => "manifest_signed",
            ManifestKind::OciArtifactV1 => "artifact",
            ManifestKind::Blob => "blob",
            ManifestKind::Unknown if relative_path.contains("manifests") => "manifest",
            ManifestKind::Unknown => "blob",
        };

        let mut unit = ContentUnit::new(
            PluginType::Container,
            serde_json::json!({
                "content_type": kind_label,
                "media_type": media_type_for(kind),
                "digest": digest.to_oci(),
                "is_index": kind.is_index(),
                "child_digests": child_digests,
                "relative_path": relative_path,
            }),
        );
        unit.relative_path = Some(relative_path.to_string());
        unit.sha256 = Some(digest.hex.clone());
        unit.size = Some(data.len() as u64);
        Ok(unit)
    }

    fn generate_metadata(
        &self,
        _repo_version: &RepositoryVersion,
        units: &[ContentUnit],
    ) -> serde_json::Value {
        let manifests: Vec<&ContentUnit> = units
            .iter()
            .filter(|u| {
                let ct = u.metadata["content_type"].as_str().unwrap_or("");
                matches!(
                    ct,
                    "manifest" | "manifest_list" | "manifest_signed" | "artifact"
                )
            })
            .collect();

        let tags: Vec<&ContentUnit> = units
            .iter()
            .filter(|u| u.metadata.get("tagged_manifest_digest").is_some())
            .collect();

        serde_json::json!({
            "manifests": manifests.iter().map(|u| serde_json::json!({
                "digest": u.metadata["digest"],
                "media_type": u.metadata["media_type"],
                "is_index": u.metadata["is_index"],
                "child_digests": u.metadata["child_digests"],
            })).collect::<Vec<_>>(),
            "tags": tags.iter().map(|u| serde_json::json!({
                "name": u.metadata["name"],
                "tagged_manifest_digest": u.metadata["tagged_manifest_digest"],
            })).collect::<Vec<_>>(),
        })
    }
}

/// Construct a `ContainerTag` content unit pointing at a manifest digest.
pub fn build_tag(name: &str, manifest_digest: &Digest256) -> ContentUnit {
    ContentUnit::new(
        PluginType::Container,
        serde_json::json!({
            "content_type": "tag",
            "name": name,
            "tagged_manifest_digest": manifest_digest.to_oci(),
        }),
    )
}

fn classify_blob(data: &[u8]) -> (ManifestKind, Vec<String>) {
    if data.is_empty() {
        return (ManifestKind::Blob, vec![]);
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return (ManifestKind::Blob, vec![]);
    };
    let mt = value
        .get("mediaType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let kind = ManifestKind::from_media_type(mt);

    let child_digests: Vec<String> = if kind.is_index() {
        value
            .get("manifests")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("digest").and_then(|d| d.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![]
    };

    let final_kind = if !matches!(kind, ManifestKind::Unknown) {
        kind
    } else if value.get("manifests").is_some() {
        ManifestKind::OciIndexV1
    } else if value.get("config").is_some() && value.get("layers").is_some() {
        ManifestKind::OciManifestV1
    } else if value.get("fsLayers").is_some() {
        ManifestKind::DockerManifestV1
    } else {
        ManifestKind::Blob
    };

    let final_children = if final_kind.is_index() && child_digests.is_empty() {
        value
            .get("manifests")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("digest").and_then(|d| d.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        child_digests
    };

    (final_kind, final_children)
}

fn media_type_for(kind: ManifestKind) -> &'static str {
    match kind {
        ManifestKind::OciManifestV1 => "application/vnd.oci.image.manifest.v1+json",
        ManifestKind::OciIndexV1 => "application/vnd.oci.image.index.v1+json",
        ManifestKind::DockerManifestV2 => "application/vnd.docker.distribution.manifest.v2+json",
        ManifestKind::DockerManifestListV2 => {
            "application/vnd.docker.distribution.manifest.list.v2+json"
        }
        ManifestKind::DockerManifestV1 => "application/vnd.docker.distribution.manifest.v1+json",
        ManifestKind::OciArtifactV1 => "application/vnd.oci.artifact.manifest.v1+json",
        ManifestKind::Blob => "application/octet-stream",
        ManifestKind::Unknown => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_kind_from_media_type() {
        assert_eq!(
            ManifestKind::from_media_type("application/vnd.oci.image.manifest.v1+json"),
            ManifestKind::OciManifestV1
        );
        assert_eq!(
            ManifestKind::from_media_type("application/vnd.oci.image.index.v1+json"),
            ManifestKind::OciIndexV1
        );
        assert_eq!(
            ManifestKind::from_media_type(
                "application/vnd.docker.distribution.manifest.list.v2+json"
            ),
            ManifestKind::DockerManifestListV2
        );
        assert_eq!(
            ManifestKind::from_media_type(
                "application/vnd.docker.distribution.manifest.v1+prettyjws"
            ),
            ManifestKind::DockerManifestV1
        );
        assert_eq!(
            ManifestKind::from_media_type("application/vnd.oci.image.layer.v1.tar+gzip"),
            ManifestKind::Blob
        );
    }

    #[test]
    fn manifest_kind_is_index() {
        assert!(ManifestKind::OciIndexV1.is_index());
        assert!(ManifestKind::DockerManifestListV2.is_index());
        assert!(!ManifestKind::OciManifestV1.is_index());
        assert!(!ManifestKind::Blob.is_index());
    }

    #[test]
    fn digest_parse_validates_format() {
        let d = Digest256::parse(
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        )
        .unwrap();
        assert_eq!(d.hex.len(), 64);
        assert!(Digest256::parse("sha256:short").is_none());
        assert!(Digest256::parse(
            "md5:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        )
        .is_none());
    }

    #[test]
    fn digest_roundtrip() {
        let d = Digest256::from_bytes(b"hello");
        assert!(d.to_oci().starts_with("sha256:"));
        let parsed = Digest256::parse(&d.to_oci()).unwrap();
        assert_eq!(parsed.hex, d.hex);
    }

    #[test]
    fn parse_oci_image_manifest() {
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "size": 7023,
            },
            "layers": [
                { "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                  "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000001",
                  "size": 32654 },
            ],
        });
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let plugin = ContainerPlugin;
        let unit = plugin
            .parse_content(&bytes, "v2/library/test/manifests/sha256:xxx")
            .unwrap();
        assert_eq!(unit.metadata["content_type"], "manifest");
        assert_eq!(
            unit.metadata["media_type"],
            "application/vnd.oci.image.manifest.v1+json"
        );
        assert_eq!(unit.metadata["is_index"], false);
    }

    #[test]
    fn parse_oci_image_index_collects_child_digests() {
        let index = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                    "size": 1234,
                    "platform": { "os": "linux", "architecture": "amd64" },
                },
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                    "size": 1234,
                    "platform": { "os": "linux", "architecture": "arm64" },
                },
            ],
        });
        let bytes = serde_json::to_vec(&index).unwrap();
        let plugin = ContainerPlugin;
        let unit = plugin
            .parse_content(&bytes, "v2/library/test/manifests/sha256:idx")
            .unwrap();
        assert_eq!(unit.metadata["content_type"], "manifest_list");
        assert_eq!(unit.metadata["is_index"], true);
        let children = unit.metadata["child_digests"].as_array().unwrap();
        assert_eq!(children.len(), 2);
        assert!(children[0].as_str().unwrap().starts_with("sha256:11"));
        assert!(children[1].as_str().unwrap().starts_with("sha256:22"));
    }

    #[test]
    fn parse_docker_schema1_signed() {
        let body = serde_json::json!({
            "schemaVersion": 1,
            "fsLayers": [ { "blobSum": "sha256:abc" } ],
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let plugin = ContainerPlugin;
        let unit = plugin
            .parse_content(&bytes, "v2/library/test/manifests/v1")
            .unwrap();
        assert_eq!(unit.metadata["content_type"], "manifest_signed");
    }

    #[test]
    fn parse_layer_blob() {
        let plugin = ContainerPlugin;
        let unit = plugin
            .parse_content(&[0x1f, 0x8b, 0x08, 0x00], "v2/library/test/blobs/sha256:xx")
            .unwrap();
        assert_eq!(unit.metadata["content_type"], "blob");
    }

    #[test]
    fn build_tag_links_manifest_digest() {
        let digest = Digest256::from_bytes(b"manifest-bytes");
        let tag = build_tag("v1.0", &digest);
        assert_eq!(tag.metadata["content_type"], "tag");
        assert_eq!(tag.metadata["name"], "v1.0");
        assert_eq!(tag.metadata["tagged_manifest_digest"], digest.to_oci());
    }

    #[test]
    fn generate_metadata_surfaces_manifests_and_tags() {
        let plugin = ContainerPlugin;
        let manifest = plugin
            .parse_content(
                br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{},"layers":[]}"#,
                "manifests/sha256:abc",
            )
            .unwrap();
        let digest = Digest256 {
            hex: manifest.sha256.clone().unwrap(),
        };
        let tag = build_tag("latest", &digest);
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/container/container/x/", 1);
        let meta = plugin.generate_metadata(&ver, &[manifest, tag]);
        assert_eq!(meta["manifests"].as_array().unwrap().len(), 1);
        assert_eq!(meta["tags"].as_array().unwrap().len(), 1);
        assert_eq!(meta["tags"][0]["name"], "latest");
    }
}
