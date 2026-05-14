// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! pulp_python — Python Package Index (PyPI) content plugin.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct PythonPlugin;

impl ArtifactsPlugin for PythonPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Python
    }

    fn name(&self) -> &str {
        "pulp_python"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["python.python"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        // Extract package name and version from the filename.
        // Typical wheel: {name}-{ver}-{python}-{abi}-{platform}.whl
        // Typical sdist: {name}-{ver}.tar.gz
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let (name, version) = parse_python_filename(filename);
        let sha256 = hex::encode(Sha256::digest(data));

        let mut unit = ContentUnit::new(
            PluginType::Python,
            serde_json::json!({
                "name": name,
                "version": version,
                "filename": filename,
                "packagetype": if filename.ends_with(".whl") { "bdist_wheel" } else { "sdist" },
                "sha256_digest": sha256,
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
        // Generate a PyPI Simple API index.
        let mut packages: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();

        for unit in units {
            let name = unit
                .metadata
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_lowercase();
            let filename = unit.metadata.get("filename").and_then(|v| v.as_str()).unwrap_or("");
            let sha256 = unit.sha256.clone().unwrap_or_default();

            packages.entry(name).or_default().push(serde_json::json!({
                "filename": filename,
                "url": format!("../../packages/{filename}"),
                "digests": { "sha256": sha256 },
            }));
        }

        serde_json::json!({ "packages": packages })
    }
}

/// Parse `name` and `version` from a Python distribution filename.
fn parse_python_filename(filename: &str) -> (String, String) {
    // Strip extension(s): .whl, .tar.gz, .zip
    let stem = filename
        .strip_suffix(".whl")
        .or_else(|| filename.strip_suffix(".tar.gz"))
        .or_else(|| filename.strip_suffix(".zip"))
        .unwrap_or(filename);

    let parts: Vec<&str> = stem.splitn(3, '-').collect();
    let name = parts.first().copied().unwrap_or("unknown").replace('_', "-");
    let version = parts.get(1).copied().unwrap_or("0.0.0").to_string();
    (name, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wheel_filename() {
        let (name, ver) = parse_python_filename("requests-2.31.0-py3-none-any.whl");
        assert_eq!(name, "requests");
        assert_eq!(ver, "2.31.0");
    }

    #[test]
    fn parse_sdist_filename() {
        let (name, ver) = parse_python_filename("my_package-1.2.3.tar.gz");
        assert_eq!(name, "my-package");
        assert_eq!(ver, "1.2.3");
    }

    #[test]
    fn python_plugin_parse_content() {
        let plugin = PythonPlugin;
        let unit = plugin
            .parse_content(b"fake wheel data", "simple/requests-2.31.0-py3-none-any.whl")
            .unwrap();
        assert_eq!(unit.plugin_type, PluginType::Python);
        assert_eq!(unit.metadata["name"], "requests");
        assert_eq!(unit.metadata["version"], "2.31.0");
        assert_eq!(unit.metadata["packagetype"], "bdist_wheel");
    }

    #[test]
    fn python_simple_index_generation() {
        let plugin = PythonPlugin;
        let ver = RepositoryVersion::new("/repo/", 1);
        let unit = plugin
            .parse_content(b"data", "requests-2.31.0-py3-none-any.whl")
            .unwrap();
        let meta = plugin.generate_metadata(&ver, &[unit]);
        assert!(meta["packages"]["requests"].is_array());
    }
}
