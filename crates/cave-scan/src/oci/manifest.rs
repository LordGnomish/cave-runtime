// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/image/daemon/image.go (manifest decode)

//! OCI / Docker image manifest decoder.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    OciManifestV1,
    OciIndexV1,
    DockerManifestV2,
    DockerImageConfigV1,
    OciLayerV1TarGzip,
    OciLayerV1TarZstd,
    DockerLayerV1TarGzip,
}

impl MediaType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "application/vnd.oci.image.manifest.v1+json" => Some(Self::OciManifestV1),
            "application/vnd.oci.image.index.v1+json" => Some(Self::OciIndexV1),
            "application/vnd.docker.distribution.manifest.v2+json" => {
                Some(Self::DockerManifestV2)
            }
            "application/vnd.docker.container.image.v1+json" => {
                Some(Self::DockerImageConfigV1)
            }
            "application/vnd.oci.image.layer.v1.tar+gzip" => Some(Self::OciLayerV1TarGzip),
            "application/vnd.oci.image.layer.v1.tar+zstd" => Some(Self::OciLayerV1TarZstd),
            "application/vnd.docker.image.rootfs.diff.tar.gzip" => {
                Some(Self::DockerLayerV1TarGzip)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfigDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct LayerDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct ImageManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
    pub config: ConfigDescriptor,
    #[serde(default)]
    pub layers: Vec<LayerDescriptor>,
}

impl ImageManifest {
    pub fn parse(input: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(input)
    }
}
