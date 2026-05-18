// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_helm — Helm chart content plugin (NEW in Phase 2).
//!
//! Implements:
//! - Chart.yaml reader (`parse_chart_yaml`) for apiVersion v1 + v2.
//! - index.yaml generator (`generate_helm_index_yaml`).
//! - HelmPlugin trait impl. Chart.tgz extraction is reachable through
//!   the same flate2 + tar pair we already pulled in for deb / ansible.
//!
//! Upstream parity: pulp/pulp_helm `pulp_helm/app/models.py` +
//! Helm spec: https://helm.sh/docs/topics/charts/#the-chartyaml-file

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub struct HelmPlugin;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ChartDependency {
    pub name: String,
    pub version: String,
    pub repository: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChartYaml {
    pub api_version: String,
    pub name: String,
    pub version: String,
    pub app_version: Option<String>,
    pub chart_type: Option<String>,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    pub dependencies: Vec<ChartDependency>,
    pub maintainers: Vec<String>,
    pub home: Option<String>,
    pub sources: Vec<String>,
    pub icon: Option<String>,
}

/// Internal serde shape (matches the YAML field naming directly).
#[derive(Deserialize, Default)]
#[serde(default)]
struct RawChart {
    #[serde(rename = "apiVersion")]
    api_version: Option<String>,
    name: Option<String>,
    version: Option<String>,
    #[serde(rename = "appVersion")]
    app_version: Option<serde_yaml::Value>,
    #[serde(rename = "type")]
    chart_type: Option<String>,
    description: Option<String>,
    keywords: Vec<String>,
    dependencies: Vec<ChartDependency>,
    maintainers: Vec<RawMaintainer>,
    home: Option<String>,
    sources: Vec<String>,
    icon: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMaintainer {
    name: String,
    #[allow(dead_code)]
    email: Option<String>,
    #[allow(dead_code)]
    url: Option<String>,
}

/// Parse a Chart.yaml body (both v1 and v2 supported).
pub fn parse_chart_yaml(raw: &str) -> Result<ChartYaml, ArtifactsError> {
    let r: RawChart = serde_yaml::from_str(raw)
        .map_err(|e| ArtifactsError::InvalidRequest(format!("Chart.yaml: {e}")))?;
    let name = r
        .name
        .ok_or_else(|| ArtifactsError::InvalidRequest("Chart.yaml: missing name".into()))?;
    let version = r
        .version
        .ok_or_else(|| ArtifactsError::InvalidRequest("Chart.yaml: missing version".into()))?;
    let api_version = r.api_version.unwrap_or_else(|| "v1".to_string());
    let app_version = r.app_version.and_then(|v| match v {
        serde_yaml::Value::String(s) => Some(s),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    Ok(ChartYaml {
        api_version,
        name,
        version,
        app_version,
        chart_type: r.chart_type,
        description: r.description,
        keywords: r.keywords,
        dependencies: r.dependencies,
        maintainers: r.maintainers.into_iter().map(|m| m.name).collect(),
        home: r.home,
        sources: r.sources,
        icon: r.icon,
    })
}

/// Render a Helm-compatible `index.yaml` body.
///
/// `entries` is `(chart, filename, digest_hex)`. URLs are
/// `{base_url}/{filename}` (one URL per entry). The output groups
/// versions by chart name like upstream helm-repo serves it.
pub fn generate_helm_index_yaml(
    entries: &[(&ChartYaml, &str, &str)],
    base_url: &str,
) -> String {
    use std::collections::BTreeMap;
    let mut by_name: BTreeMap<&str, Vec<&(&ChartYaml, &str, &str)>> = BTreeMap::new();
    for e in entries {
        by_name.entry(e.0.name.as_str()).or_default().push(e);
    }
    let mut s = String::with_capacity(512);
    s.push_str("apiVersion: v1\n");
    s.push_str(&format!("generated: \"{}\"\n", chrono::Utc::now().to_rfc3339()));
    s.push_str("entries:\n");
    for (name, items) in &by_name {
        s.push_str(&format!("  {name}:\n"));
        for (chart, filename, digest) in items {
            s.push_str(&format!("    - apiVersion: {}\n", yaml_scalar(&chart.api_version)));
            s.push_str(&format!("      name: {name}\n"));
            s.push_str(&format!("      version: {}\n", yaml_scalar(&chart.version)));
            if let Some(av) = &chart.app_version {
                s.push_str(&format!("      appVersion: {}\n", yaml_scalar(av)));
            }
            if let Some(t) = &chart.chart_type {
                s.push_str(&format!("      type: {}\n", yaml_scalar(t)));
            }
            if let Some(desc) = &chart.description {
                s.push_str(&format!("      description: {}\n", yaml_scalar(desc)));
            }
            s.push_str("      urls:\n");
            s.push_str(&format!("        - {}/{}\n", base_url.trim_end_matches('/'), filename));
            s.push_str(&format!("      digest: {digest}\n"));
        }
    }
    s
}

fn yaml_scalar(v: &str) -> String {
    // If string contains any special yaml chars, quote it; else emit bare.
    if v.is_empty() || v.chars().any(|c| matches!(c, ':' | '#' | '@' | '`' | '!' | '*' | '&' | '?' | '|' | '>' | '%' | '"' | '\'' | '{' | '}' | '[' | ']' | ',' | '\n'))
        || v.starts_with('-')
        || v.parse::<f64>().is_ok()
    {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
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
        vec!["helm.chart"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        // Try to crack open a .tgz chart and pull out Chart.yaml.
        let mut chart: Option<ChartYaml> = None;
        if filename.ends_with(".tgz") || filename.ends_with(".tar.gz") {
            if let Some(ch) = extract_chart_yaml(data) {
                if let Ok(c) = parse_chart_yaml(&ch) {
                    chart = Some(c);
                }
            }
        }
        let metadata = if let Some(c) = chart {
            serde_json::json!({
                "kind": "chart",
                "api_version": c.api_version,
                "name": c.name,
                "version": c.version,
                "app_version": c.app_version,
                "type": c.chart_type,
                "description": c.description,
                "keywords": c.keywords,
                "dependency_count": c.dependencies.len(),
                "filename": filename,
                "digest_sha256": sha256,
            })
        } else {
            serde_json::json!({
                "kind": "chart",
                "filename": filename,
                "digest_sha256": sha256,
                "decoded": false,
            })
        };
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
        // Reconstruct minimal ChartYaml from each unit to feed
        // generate_helm_index_yaml.
        let mut charts: Vec<ChartYaml> = Vec::new();
        let mut filenames: Vec<String> = Vec::new();
        let mut digests: Vec<String> = Vec::new();
        for u in units {
            let m = &u.metadata;
            let Some(name) = m.get("name").and_then(|v| v.as_str()) else { continue };
            let Some(version) = m.get("version").and_then(|v| v.as_str()) else { continue };
            charts.push(ChartYaml {
                api_version: m.get("api_version").and_then(|v| v.as_str()).unwrap_or("v2").to_string(),
                name: name.into(),
                version: version.into(),
                app_version: m.get("app_version").and_then(|v| v.as_str()).map(String::from),
                chart_type: m.get("type").and_then(|v| v.as_str()).map(String::from),
                description: m.get("description").and_then(|v| v.as_str()).map(String::from),
                ..Default::default()
            });
            filenames.push(m.get("filename").and_then(|v| v.as_str()).unwrap_or("").to_string());
            digests.push(u.sha256.clone().unwrap_or_default());
        }
        let entries: Vec<(&ChartYaml, &str, &str)> = charts
            .iter()
            .zip(filenames.iter())
            .zip(digests.iter())
            .map(|((c, f), d)| (c, f.as_str(), d.as_str()))
            .collect();
        let index = generate_helm_index_yaml(&entries, "https://helm.example.com/charts");
        serde_json::json!({
            "index_yaml": index,
            "count": charts.len(),
        })
    }
}

fn extract_chart_yaml(tgz: &[u8]) -> Option<String> {
    use std::io::Read;
    let mut d = flate2::read::GzDecoder::new(tgz);
    let mut decoded = Vec::new();
    d.read_to_end(&mut decoded).ok()?;
    let mut a = tar::Archive::new(&decoded[..]);
    for entry in a.entries().ok()? {
        let mut e = entry.ok()?;
        let path = e.path().ok()?.into_owned();
        let path_str = path.to_string_lossy();
        // Charts archive their files under <chart_name>/Chart.yaml
        if path_str.ends_with("/Chart.yaml") || path_str == "Chart.yaml" {
            let mut s = String::new();
            e.read_to_string(&mut s).ok()?;
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_scalar_quotes_special() {
        assert_eq!(yaml_scalar("simple"), "simple");
        assert_eq!(yaml_scalar("has:colon"), "\"has:colon\"");
        assert_eq!(yaml_scalar("1.2.3"), "1.2.3"); // version-like passes as bare
        assert_eq!(yaml_scalar("1.0"), "\"1.0\""); // parses as float → quoted
    }
}
