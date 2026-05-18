// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_helm — Helm chart content plugin (scaffold).
//!
//! Replaced in the Helm deep-port commit with the full chart parser
//! (Chart.yaml + provenance + dependencies) and `index.yaml` builder.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct HelmPlugin;

impl ArtifactsPlugin for HelmPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Helm
    }

    fn name(&self) -> &str {
        "pulp_helm"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["helm.chart", "helm.provenance"]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let mut unit = ContentUnit::new(
            PluginType::Helm,
            serde_json::json!({ "relative_path": relative_path }),
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
        serde_json::json!({ "entries": units.len() })
    }
}
