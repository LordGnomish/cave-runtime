// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_rpm — RPM package content plugin.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct RpmPlugin;

impl ArtifactsPlugin for RpmPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Rpm
    }

    fn name(&self) -> &str {
        "pulp_rpm"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["rpm.package", "rpm.advisory", "rpm.modulemd", "rpm.repo_metadata_file"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        // Parse: {name}-{version}-{release}.{arch}.rpm
        let (name, version, release, arch) = parse_rpm_filename(filename);
        let sha256 = hex::encode(Sha256::digest(data));

        let mut unit = ContentUnit::new(
            PluginType::Rpm,
            serde_json::json!({
                "name": name,
                "version": version,
                "release": release,
                "arch": arch,
                "epoch": "0",
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
        // In production: generate repomd.xml, primary.xml.gz, filelists.xml.gz, other.xml.gz
        let packages: Vec<serde_json::Value> = units
            .iter()
            .map(|u| {
                serde_json::json!({
                    "name": u.metadata["name"],
                    "version": u.metadata["version"],
                    "arch": u.metadata["arch"],
                })
            })
            .collect();
        serde_json::json!({
            "repomd": {
                "revision": chrono::Utc::now().timestamp(),
                "packages": packages.len(),
            },
            "primary": packages,
        })
    }
}

fn parse_rpm_filename(filename: &str) -> (String, String, String, String) {
    let stem = filename.strip_suffix(".rpm").unwrap_or(filename);
    // arch is last segment after final dot
    let (rest, arch) = stem.rsplit_once('.').unwrap_or((stem, "noarch"));
    // release is last segment after final dash
    let (rest2, release) = rest.rsplit_once('-').unwrap_or((rest, "1"));
    // version is last segment after final dash
    let (name, version) = rest2.rsplit_once('-').unwrap_or((rest2, "0"));
    (
        name.to_string(),
        version.to_string(),
        release.to_string(),
        arch.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rpm_name() {
        let (name, ver, rel, arch) = parse_rpm_filename("bash-5.1.8-6.el9.x86_64.rpm");
        assert_eq!(name, "bash");
        assert_eq!(ver, "5.1.8");
        assert_eq!(rel, "6.el9");
        assert_eq!(arch, "x86_64");
    }
}
