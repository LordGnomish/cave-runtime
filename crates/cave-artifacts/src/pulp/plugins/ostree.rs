// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_ostree — OSTree commit/ref/object content plugin (scaffold).
//!
//! Replaced in the OSTree deep-port commit with the full
//! refs/commits/objects parser, summary file, and remote-resolution surface.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct OstreePlugin;

impl ArtifactsPlugin for OstreePlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::OSTree
    }

    fn name(&self) -> &str {
        "pulp_ostree"
    }

    fn content_types(&self) -> Vec<&str> {
        vec![
            "ostree.refs",
            "ostree.commit",
            "ostree.object",
            "ostree.summary",
            "ostree.config",
        ]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let mut unit = ContentUnit::new(
            PluginType::OSTree,
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
        serde_json::json!({ "summary": { "objects": units.len() } })
    }
}
