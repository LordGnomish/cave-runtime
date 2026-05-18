// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_ansible — Ansible collection and role content plugin.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct AnsiblePlugin;

impl ArtifactsPlugin for AnsiblePlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Ansible
    }

    fn name(&self) -> &str {
        "pulp_ansible"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["ansible.collection_version", "ansible.role"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        // Ansible collections: {namespace}-{name}-{version}.tar.gz
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let (namespace, name, version) = parse_collection_filename(filename);
        let sha256 = hex::encode(Sha256::digest(data));

        let mut unit = ContentUnit::new(
            PluginType::Ansible,
            serde_json::json!({
                "namespace": namespace,
                "name": name,
                "version": version,
                "filename": filename,
                "sha256": sha256,
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
        // Galaxy API v3 compatible index.
        let collections: Vec<serde_json::Value> = units
            .iter()
            .map(|u| {
                serde_json::json!({
                    "namespace": u.metadata["namespace"],
                    "name": u.metadata["name"],
                    "version": u.metadata["version"],
                    "download_url": format!(
                        "../../artifacts/{}-{}-{}.tar.gz",
                        u.metadata["namespace"].as_str().unwrap_or(""),
                        u.metadata["name"].as_str().unwrap_or(""),
                        u.metadata["version"].as_str().unwrap_or(""),
                    ),
                })
            })
            .collect();
        serde_json::json!({ "results": collections, "count": collections.len() })
    }
}

fn parse_collection_filename(filename: &str) -> (String, String, String) {
    let stem = filename
        .strip_suffix(".tar.gz")
        .unwrap_or(filename);
    let parts: Vec<&str> = stem.splitn(3, '-').collect();
    (
        parts.first().copied().unwrap_or("unknown").to_string(),
        parts.get(1).copied().unwrap_or("unknown").to_string(),
        parts.get(2).copied().unwrap_or("0.0.0").to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_collection_name() {
        let (ns, name, ver) = parse_collection_filename("community-general-7.3.0.tar.gz");
        assert_eq!(ns, "community");
        assert_eq!(name, "general");
        assert_eq!(ver, "7.3.0");
    }
}
