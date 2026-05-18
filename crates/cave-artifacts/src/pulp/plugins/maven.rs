// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_maven — Maven2 artifact content plugin.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct MavenPlugin;

/// Parsed Maven GAV (Group : Artifact : Version) coordinates.
#[derive(Debug, PartialEq)]
pub struct MavenCoordinates {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl MavenCoordinates {
    /// Parse from a repository-relative path like
    /// `com/example/mylib/1.0/mylib-1.0-sources.jar`.
    pub fn from_path(path: &str) -> Option<Self> {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let filename = *parts.last()?;
        let version = parts[parts.len() - 2].to_string();
        let artifact_id = parts[parts.len() - 3].to_string();
        let group_id = parts[..parts.len() - 3].join(".");

        // Parse filename: {artifact_id}-{version}[-{classifier}].{ext}
        let stem = filename.splitn(2, &format!("{artifact_id}-")).nth(1)?;
        let (classifier, ext) = if let Some(rest) = stem.strip_prefix(&format!("{version}-")) {
            // has classifier
            let dot = rest.rfind('.')?;
            (Some(rest[..dot].to_string()), rest[dot + 1..].to_string())
        } else {
            let dot = stem.rfind('.')?;
            (None, stem[dot + 1..].to_string())
        };

        Some(Self {
            group_id,
            artifact_id,
            version,
            classifier,
            extension: ext,
        })
    }

    /// Is this a SNAPSHOT version?
    pub fn is_snapshot(&self) -> bool {
        self.version.contains("SNAPSHOT")
    }
}

impl ArtifactsPlugin for MavenPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Maven
    }

    fn name(&self) -> &str {
        "pulp_maven"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["maven.artifact"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let coords = MavenCoordinates::from_path(relative_path);

        let metadata = if let Some(ref c) = coords {
            serde_json::json!({
                "group_id": c.group_id,
                "artifact_id": c.artifact_id,
                "version": c.version,
                "classifier": c.classifier,
                "extension": c.extension,
                "is_snapshot": c.is_snapshot(),
            })
        } else {
            serde_json::json!({ "relative_path": relative_path })
        };

        let mut unit = ContentUnit::new(PluginType::Maven, metadata);
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
        // In production: generate maven-metadata.xml per groupId/artifactId.
        let mut groups: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

        for unit in units {
            if let (Some(gid), Some(aid), Some(ver)) = (
                unit.metadata.get("group_id").and_then(|v| v.as_str()),
                unit.metadata.get("artifact_id").and_then(|v| v.as_str()),
                unit.metadata.get("version").and_then(|v| v.as_str()),
            ) {
                groups
                    .entry(format!("{gid}:{aid}"))
                    .or_default()
                    .push(ver.to_string());
            }
        }

        serde_json::json!({ "maven_metadata": groups })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maven_path_jar() {
        let coords = MavenCoordinates::from_path(
            "com/example/mylib/1.0.0/mylib-1.0.0.jar",
        )
        .unwrap();
        assert_eq!(coords.group_id, "com.example");
        assert_eq!(coords.artifact_id, "mylib");
        assert_eq!(coords.version, "1.0.0");
        assert_eq!(coords.classifier, None);
        assert_eq!(coords.extension, "jar");
        assert!(!coords.is_snapshot());
    }

    #[test]
    fn parse_maven_snapshot() {
        let coords = MavenCoordinates::from_path(
            "org/acme/service/2.0.0-SNAPSHOT/service-2.0.0-SNAPSHOT.jar",
        )
        .unwrap();
        assert!(coords.is_snapshot());
    }

    #[test]
    fn parse_maven_path_sources() {
        let coords = MavenCoordinates::from_path(
            "com/example/mylib/1.0.0/mylib-1.0.0-sources.jar",
        )
        .unwrap();
        assert_eq!(coords.classifier, Some("sources".to_string()));
    }
}
