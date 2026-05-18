// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for the new pulp_helm content plugin.

use cave_artifacts::pulp::plugins::helm::{
    generate_helm_index_yaml, parse_chart_yaml, ChartDependency, ChartYaml, HelmPlugin,
};
use cave_artifacts::pulp::models::PluginType;
use cave_artifacts::pulp::plugin::ArtifactsPlugin;

const CHART_V2: &str = r#"
apiVersion: v2
name: my-app
description: A test chart
version: 1.2.3
appVersion: "4.5.6"
type: application
keywords:
  - test
  - example
dependencies:
  - name: redis
    version: 17.0.0
    repository: https://charts.bitnami.com/bitnami
  - name: postgres
    version: 12.0.0
    repository: file://../postgres
"#;

const CHART_V1: &str = r#"
apiVersion: v1
name: legacy-chart
version: 0.1.0
"#;

#[test]
fn parse_chart_v2_full() {
    let c: ChartYaml = parse_chart_yaml(CHART_V2).unwrap();
    assert_eq!(c.api_version, "v2");
    assert_eq!(c.name, "my-app");
    assert_eq!(c.version, "1.2.3");
    assert_eq!(c.app_version.as_deref(), Some("4.5.6"));
    assert_eq!(c.chart_type.as_deref(), Some("application"));
    assert_eq!(c.keywords, vec!["test", "example"]);
    assert_eq!(c.dependencies.len(), 2);
    let d0: &ChartDependency = &c.dependencies[0];
    assert_eq!(d0.name, "redis");
    assert_eq!(d0.version, "17.0.0");
    assert_eq!(d0.repository, "https://charts.bitnami.com/bitnami");
}

#[test]
fn parse_chart_v1_minimal() {
    let c = parse_chart_yaml(CHART_V1).unwrap();
    assert_eq!(c.api_version, "v1");
    assert_eq!(c.name, "legacy-chart");
    assert_eq!(c.version, "0.1.0");
    assert!(c.app_version.is_none());
}

#[test]
fn parse_chart_rejects_missing_name_or_version() {
    let bad = "apiVersion: v2\nname: x\n";
    assert!(parse_chart_yaml(bad).is_err());
    let bad2 = "apiVersion: v2\nversion: 1.0\n";
    assert!(parse_chart_yaml(bad2).is_err());
}

#[test]
fn generate_index_yaml_basic() {
    let c1 = parse_chart_yaml(CHART_V2).unwrap();
    let mut c2 = parse_chart_yaml(CHART_V2).unwrap();
    c2.version = "1.3.0".into();
    let entries = vec![
        (c1, "my-app-1.2.3.tgz", "abc123"),
        (c2, "my-app-1.3.0.tgz", "def456"),
    ];
    let entries_refs: Vec<(&ChartYaml, &str, &str)> = entries
        .iter()
        .map(|(c, f, d)| (c, *f, *d))
        .collect();
    let yaml = generate_helm_index_yaml(&entries_refs, "https://charts.example.com");
    assert!(yaml.contains("apiVersion: v1"));
    assert!(yaml.contains("entries:"));
    assert!(yaml.contains("my-app:"));
    assert!(yaml.contains("version: 1.2.3"));
    assert!(yaml.contains("version: 1.3.0"));
    assert!(yaml.contains("urls:"));
    assert!(yaml.contains("https://charts.example.com/my-app-1.2.3.tgz"));
}

#[test]
fn helm_plugin_basics() {
    let plugin = HelmPlugin;
    assert_eq!(plugin.name(), "pulp_helm");
    assert_eq!(plugin.plugin_type(), PluginType::Helm);
    assert!(plugin.content_types().iter().any(|t| t.contains("helm")));
}
