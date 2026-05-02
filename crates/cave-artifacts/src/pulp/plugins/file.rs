//! pulp_file — generic file content plugin.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct FilePlugin;

impl ArtifactsPlugin for FilePlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::File
    }

    fn name(&self) -> &str {
        "pulp_file"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["file.file"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let mut unit = ContentUnit::new(
            PluginType::File,
            serde_json::json!({
                "relative_path": relative_path,
                "digest": sha256,
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
        // pulp_file generates a MANIFEST.json listing all files.
        let manifest: Vec<serde_json::Value> = units
            .iter()
            .map(|u| {
                serde_json::json!({
                    "relative_path": u.relative_path,
                    "sha256": u.sha256,
                    "size": u.size,
                })
            })
            .collect();
        serde_json::json!({ "manifest": manifest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file_content() {
        let plugin = FilePlugin;
        let unit = plugin.parse_content(b"hello world", "docs/readme.txt").unwrap();
        assert_eq!(unit.plugin_type, PluginType::File);
        assert_eq!(unit.relative_path.as_deref(), Some("docs/readme.txt"));
        assert!(unit.sha256.is_some());
        assert_eq!(unit.size, Some(11));
    }

    #[test]
    fn generate_file_manifest() {
        let plugin = FilePlugin;
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/file/file/x/", 1);
        let units = vec![
            plugin.parse_content(b"a", "a.txt").unwrap(),
            plugin.parse_content(b"bb", "b.txt").unwrap(),
        ];
        let meta = plugin.generate_metadata(&ver, &units);
        let manifest = meta["manifest"].as_array().unwrap();
        assert_eq!(manifest.len(), 2);
    }
}
