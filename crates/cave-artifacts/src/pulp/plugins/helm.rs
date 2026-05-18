// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_helm — Helm chart content plugin.
//!
//! Mirrors `pulp_helm`:
//! - Chart filename parser: `{name}-{semver}.tgz` (canonical chart
//!   archive layout).
//! - Provenance recognition: `{name}-{semver}.tgz.prov` (Helm's
//!   detached-PGP signature companion).
//! - Helm classic repository `index.yaml` renderer covering the
//!   `apiVersion`/`entries`/`generated` schema and per-chart fields
//!   (`name`, `version`, `digest`, `urls`, `created`, `description`,
//!   `appVersion`).

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct HelmPlugin;

/// Parsed Helm chart filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmFilename {
    pub name: String,
    pub version: String,
    pub is_provenance: bool,
}

impl HelmFilename {
    pub fn parse(filename: &str) -> Option<Self> {
        let (stem, is_prov) = match filename.strip_suffix(".tgz.prov") {
            Some(s) => (s, true),
            None => (filename.strip_suffix(".tgz")?, false),
        };
        let (name, version) = stem.rsplit_once('-')?;
        if name.is_empty() || version.is_empty() {
            return None;
        }
        Some(Self {
            name: name.to_string(),
            version: version.to_string(),
            is_provenance: is_prov,
        })
    }
}

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
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let parsed = HelmFilename::parse(filename).ok_or_else(|| {
            ArtifactsError::InvalidRequest(format!("not a helm chart filename: {filename}"))
        })?;
        let sha256 = hex::encode(Sha256::digest(data));

        let metadata = serde_json::json!({
            "name": parsed.name,
            "version": parsed.version,
            "filename": filename,
            "content_type": if parsed.is_provenance { "provenance" } else { "chart" },
            "digest": format!("sha256:{sha256}"),
        });

        let mut unit = ContentUnit::new(PluginType::Helm, metadata);
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
        let charts: Vec<&ContentUnit> = units
            .iter()
            .filter(|u| u.metadata.get("content_type").and_then(|v| v.as_str()) == Some("chart"))
            .collect();

        serde_json::json!({
            "index_yaml": render_index_yaml(&charts),
        })
    }
}

/// Render a Helm classic-repository `index.yaml`. Charts are grouped by
/// `name`; within each name the versions are listed in input order (the
/// caller should pre-sort if a specific ordering is desired).
pub fn render_index_yaml(units: &[&ContentUnit]) -> String {
    let mut by_name: BTreeMap<String, Vec<&ContentUnit>> = BTreeMap::new();
    for u in units {
        let name = u.metadata["name"].as_str().unwrap_or("").to_string();
        by_name.entry(name).or_default().push(u);
    }

    let generated = chrono::Utc::now().to_rfc3339();
    let mut out = String::from("apiVersion: v1\nentries:\n");
    for (name, versions) in &by_name {
        out.push_str(&format!("  {name}:\n"));
        for u in versions {
            let ver = u.metadata["version"].as_str().unwrap_or("");
            let filename = u.metadata["filename"].as_str().unwrap_or("");
            let digest = u
                .metadata
                .get("digest")
                .and_then(|v| v.as_str())
                .and_then(|s| s.strip_prefix("sha256:"))
                .unwrap_or_default();
            out.push_str(&format!(
                "    - name: {name}\n      version: {ver}\n      digest: {digest}\n      created: {generated}\n      urls:\n        - charts/{filename}\n",
            ));
            if let Some(desc) = u.metadata.get("description").and_then(|v| v.as_str()) {
                out.push_str(&format!("      description: {desc}\n"));
            }
            if let Some(app) = u.metadata.get("appVersion").and_then(|v| v.as_str()) {
                out.push_str(&format!("      appVersion: {app}\n"));
            }
        }
    }
    out.push_str(&format!("generated: {generated}\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chart_filename() {
        let f = HelmFilename::parse("mysql-8.5.0.tgz").unwrap();
        assert_eq!(f.name, "mysql");
        assert_eq!(f.version, "8.5.0");
        assert!(!f.is_provenance);
    }

    #[test]
    fn parse_chart_filename_with_dashed_name() {
        let f = HelmFilename::parse("ingress-nginx-4.10.1.tgz").unwrap();
        assert_eq!(f.name, "ingress-nginx");
        assert_eq!(f.version, "4.10.1");
    }

    #[test]
    fn parse_provenance_filename() {
        let f = HelmFilename::parse("mysql-8.5.0.tgz.prov").unwrap();
        assert!(f.is_provenance);
        assert_eq!(f.name, "mysql");
        assert_eq!(f.version, "8.5.0");
    }

    #[test]
    fn parse_rejects_non_tgz() {
        assert!(HelmFilename::parse("mysql-8.5.0.zip").is_none());
        assert!(HelmFilename::parse("nodash.tgz").is_none());
    }

    #[test]
    fn parse_content_marks_chart_vs_provenance() {
        let plugin = HelmPlugin;
        let chart = plugin.parse_content(b"chart-tgz", "charts/mysql-8.5.0.tgz").unwrap();
        assert_eq!(chart.metadata["content_type"], "chart");
        let prov = plugin
            .parse_content(b"prov-sig", "charts/mysql-8.5.0.tgz.prov")
            .unwrap();
        assert_eq!(prov.metadata["content_type"], "provenance");
    }

    #[test]
    fn parse_content_emits_oci_digest() {
        let plugin = HelmPlugin;
        let unit = plugin
            .parse_content(b"hello", "charts/mysql-8.5.0.tgz")
            .unwrap();
        let digest = unit.metadata["digest"].as_str().unwrap();
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), 71); // "sha256:" + 64 hex chars
    }

    fn make_chart(name: &str, ver: &str) -> ContentUnit {
        let plugin = HelmPlugin;
        plugin
            .parse_content(b"x", &format!("charts/{name}-{ver}.tgz"))
            .unwrap()
    }

    #[test]
    fn index_yaml_minimal_shape() {
        let chart = make_chart("mysql", "8.5.0");
        let yaml = render_index_yaml(&[&chart]);
        assert!(yaml.starts_with("apiVersion: v1\n"));
        assert!(yaml.contains("entries:"));
        assert!(yaml.contains("  mysql:"));
        assert!(yaml.contains("    - name: mysql"));
        assert!(yaml.contains("      version: 8.5.0"));
        assert!(yaml.contains("      digest: "));
        assert!(yaml.contains("      urls:\n        - charts/mysql-8.5.0.tgz"));
        assert!(yaml.contains("generated: "));
    }

    #[test]
    fn index_yaml_groups_by_chart_name() {
        let a = make_chart("mysql", "8.5.0");
        let b = make_chart("mysql", "8.4.0");
        let c = make_chart("redis", "7.0.0");
        let yaml = render_index_yaml(&[&a, &b, &c]);
        assert!(yaml.contains("  mysql:"));
        assert!(yaml.contains("  redis:"));
        assert!(yaml.matches("- name: mysql").count() == 2);
        assert!(yaml.matches("- name: redis").count() == 1);
    }

    #[test]
    fn generate_metadata_filters_provenance_out_of_index() {
        let plugin = HelmPlugin;
        let chart = plugin
            .parse_content(b"x", "charts/mysql-8.5.0.tgz")
            .unwrap();
        let prov = plugin
            .parse_content(b"x", "charts/mysql-8.5.0.tgz.prov")
            .unwrap();
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/helm/helm/x/", 1);
        let meta = plugin.generate_metadata(&ver, &[chart, prov]);
        let idx = meta["index_yaml"].as_str().unwrap();
        // Only one entry for mysql despite the provenance unit also being present.
        assert_eq!(idx.matches("- name: mysql").count(), 1);
    }

    #[test]
    fn content_types_match_pulp_helm_models() {
        let plugin = HelmPlugin;
        let kinds = plugin.content_types();
        assert!(kinds.contains(&"helm.chart"));
        assert!(kinds.contains(&"helm.provenance"));
    }
}
