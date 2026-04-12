//! pulp_container — Container image content plugin (OCI / Docker).
//!
//! For full container functionality, this delegates to cave-registry.
//! This plugin manages the content units and metadata; blob storage and
//! the OCI distribution spec are handled by the registry layer.

use crate::error::ArtifactsError;
use crate::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct ContainerPlugin;

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

        // Determine if this is a manifest or a blob from relative_path.
        let content_type = if relative_path.contains("manifests") {
            "manifest"
        } else {
            "blob"
        };

        let mut unit = ContentUnit::new(
            PluginType::Container,
            serde_json::json!({
                "content_type": content_type,
                "digest": format!("sha256:{sha256}"),
                "relative_path": relative_path,
            }),
        );
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
        serde_json::json!({ "manifests": manifests })
    }
}
