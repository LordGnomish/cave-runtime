//! pulp_deb — Debian package content plugin.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct DebPlugin;

impl ArtifactsPlugin for DebPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Deb
    }

    fn name(&self) -> &str {
        "pulp_deb"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["deb.package", "deb.release", "deb.package_index", "deb.installer_package"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        // {name}_{version}_{arch}.deb
        let (name, version, arch) = parse_deb_filename(filename);
        let sha256 = hex::encode(Sha256::digest(data));

        let mut unit = ContentUnit::new(
            PluginType::Deb,
            serde_json::json!({
                "package": name,
                "version": version,
                "architecture": arch,
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
        // In production: generate Release, Packages.gz, InRelease
        let packages: Vec<serde_json::Value> = units
            .iter()
            .map(|u| {
                serde_json::json!({
                    "Package": u.metadata["package"],
                    "Version": u.metadata["version"],
                    "Architecture": u.metadata["architecture"],
                    "SHA256": u.sha256,
                    "Size": u.size,
                })
            })
            .collect();
        serde_json::json!({ "Packages": packages })
    }
}

fn parse_deb_filename(filename: &str) -> (String, String, String) {
    let stem = filename.strip_suffix(".deb").unwrap_or(filename);
    let parts: Vec<&str> = stem.splitn(3, '_').collect();
    let name = parts.first().copied().unwrap_or("unknown").to_string();
    let version = parts.get(1).copied().unwrap_or("0").to_string();
    let arch = parts.get(2).copied().unwrap_or("all").to_string();
    (name, version, arch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_deb_name() {
        let (name, ver, arch) = parse_deb_filename("libc6_2.35-0ubuntu3_amd64.deb");
        assert_eq!(name, "libc6");
        assert_eq!(ver, "2.35-0ubuntu3");
        assert_eq!(arch, "amd64");
    }
}
