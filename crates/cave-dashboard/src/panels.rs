//! Loader for the YAML panel catalog under `crates/cave-dashboard/dashboards/`.
//!
//! Each YAML file describes one crate's ten standard observability panels
//! (RED + USE + per-resource breakdowns + tenant cardinality guard).
//! The loader validates structural invariants and returns `PanelCatalog`
//! records that the runtime can compile into Grafana JSON dashboards.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelCatalog {
    pub crate_: String,
    pub title: String,
    pub uid: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub variables: Vec<VariableSpec>,
    pub panels: Vec<PanelSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub var_type: String,
    pub query: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub multi: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelSpec {
    pub id: u32,
    pub title: String,
    #[serde(rename = "type")]
    pub panel_type: PanelType,
    pub unit: String,
    /// Single-query form. Either `query` or `queries` is required.
    #[serde(default)]
    pub query: Option<String>,
    /// Multi-query form (e.g. p50/p95/p99 in one panel).
    #[serde(default)]
    pub queries: Option<Vec<String>>,
    /// Legend label or list of labels (must align with `queries`).
    #[serde(default)]
    pub legend: Option<LegendSpec>,
    #[serde(default)]
    pub thresholds: Vec<Threshold>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LegendSpec {
    Single(String),
    Multi(Vec<String>),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PanelType {
    Timeseries,
    Stat,
    Gauge,
    Heatmap,
    Table,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Threshold {
    pub value: f64,
    pub color: String,
}

#[derive(Debug, Error)]
pub enum PanelError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("validation: {0}")]
    Validation(String),
}

pub fn parse_catalog(input: &str) -> Result<PanelCatalog, PanelError> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(input)?;
    if let Some(map) = value.as_mapping_mut() {
        if let Some(v) = map.remove(&serde_yaml::Value::String("crate".into())) {
            map.insert(serde_yaml::Value::String("crate_".into()), v);
        }
    }
    let cat: PanelCatalog = serde_yaml::from_value(value)?;
    validate(&cat)?;
    Ok(cat)
}

pub fn load_catalog(path: impl AsRef<Path>) -> Result<PanelCatalog, PanelError> {
    parse_catalog(&std::fs::read_to_string(path)?)
}

pub fn load_directory(dir: impl AsRef<Path>) -> Result<Vec<PanelCatalog>, PanelError> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            out.push(load_catalog(&path)?);
        }
    }
    Ok(out)
}

fn validate(cat: &PanelCatalog) -> Result<(), PanelError> {
    if cat.panels.is_empty() {
        return Err(PanelError::Validation(format!("{} has no panels", cat.crate_)));
    }
    let mut ids = std::collections::HashSet::new();
    for p in &cat.panels {
        if !ids.insert(p.id) {
            return Err(PanelError::Validation(format!(
                "{}: duplicate panel id {}",
                cat.crate_, p.id
            )));
        }
        if p.query.is_none() && p.queries.as_ref().map_or(true, |q| q.is_empty()) {
            return Err(PanelError::Validation(format!(
                "{}: panel {} ({}) has no query",
                cat.crate_, p.id, p.title
            )));
        }
        if p.title.trim().is_empty() {
            return Err(PanelError::Validation(format!(
                "{}: panel {} has empty title",
                cat.crate_, p.id
            )));
        }
        if p.unit.trim().is_empty() {
            return Err(PanelError::Validation(format!(
                "{}: panel {} ({}) has empty unit",
                cat.crate_, p.id, p.title
            )));
        }
        if let (Some(qs), Some(LegendSpec::Multi(legs))) = (&p.queries, &p.legend) {
            if qs.len() != legs.len() {
                return Err(PanelError::Validation(format!(
                    "{}: panel {} has {} queries but {} legends",
                    cat.crate_,
                    p.id,
                    qs.len(),
                    legs.len()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dashboards_dir() -> std::path::PathBuf {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        std::path::PathBuf::from(manifest_dir).join("dashboards")
    }

    #[test]
    fn test_parse_minimal_catalog() {
        let yaml = r#"
crate: cave-x
title: "X"
uid: cave-x
tags: [observability]
panels:
  - id: 1
    title: "Rate"
    type: timeseries
    unit: reqps
    query: 'sum(rate(x_total[5m]))'
"#;
        let cat = parse_catalog(yaml).unwrap();
        assert_eq!(cat.panels.len(), 1);
        assert_eq!(cat.panels[0].panel_type, PanelType::Timeseries);
    }

    #[test]
    fn test_validation_rejects_duplicate_panel_id() {
        let yaml = r#"
crate: x
title: x
uid: x
panels:
  - {id: 1, title: a, type: stat, unit: short, query: 'up'}
  - {id: 1, title: b, type: stat, unit: short, query: 'up'}
"#;
        assert!(parse_catalog(yaml).is_err());
    }

    #[test]
    fn test_validation_rejects_missing_query() {
        let yaml = r#"
crate: x
title: x
uid: x
panels:
  - {id: 1, title: a, type: stat, unit: short}
"#;
        assert!(parse_catalog(yaml).is_err());
    }

    #[test]
    fn test_validation_rejects_legend_query_count_mismatch() {
        let yaml = r#"
crate: x
title: x
uid: x
panels:
  - id: 1
    title: a
    type: timeseries
    unit: s
    queries: ['a', 'b', 'c']
    legend: ['a', 'b']
"#;
        assert!(parse_catalog(yaml).is_err());
    }

    #[test]
    fn test_each_catalog_has_ten_panels() {
        let cats = load_directory(dashboards_dir()).unwrap();
        for c in &cats {
            assert_eq!(c.panels.len(), 10, "crate {} has {} panels", c.crate_, c.panels.len());
        }
    }

    #[test]
    fn test_catalog_covers_eight_crates_total_80_panels() {
        let cats = load_directory(dashboards_dir()).unwrap();
        let crates: std::collections::HashSet<_> = cats.iter().map(|c| c.crate_.clone()).collect();
        assert_eq!(crates.len(), 8);
        let total: usize = cats.iter().map(|c| c.panels.len()).sum();
        assert_eq!(total, 80);
    }

    #[test]
    fn test_every_catalog_has_observability_tag() {
        let cats = load_directory(dashboards_dir()).unwrap();
        for c in &cats {
            assert!(c.tags.contains(&"observability".to_string()), "{} missing observability tag", c.crate_);
        }
    }

    #[test]
    fn test_every_catalog_has_unique_uid() {
        let cats = load_directory(dashboards_dir()).unwrap();
        let uids: std::collections::HashSet<_> = cats.iter().map(|c| c.uid.clone()).collect();
        assert_eq!(uids.len(), cats.len(), "duplicate UIDs in catalog");
    }

    #[test]
    fn test_every_catalog_has_tenant_breakdown_panel() {
        let cats = load_directory(dashboards_dir()).unwrap();
        for c in &cats {
            assert!(
                c.panels.iter().any(|p| p.title.to_lowercase().contains("tenant")
                    || p.query.as_deref().map_or(false, |q| q.contains("tenant_id"))),
                "{} missing tenant breakdown panel",
                c.crate_
            );
        }
    }

    #[test]
    fn test_panel_type_enum_serde() {
        let t: PanelType = serde_yaml::from_str("heatmap").unwrap();
        assert_eq!(t, PanelType::Heatmap);
    }

    #[test]
    fn test_legend_single_and_multi_serde() {
        let single: LegendSpec = serde_yaml::from_str(r#""{{instance}}""#).unwrap();
        match single {
            LegendSpec::Single(s) => assert_eq!(s, "{{instance}}"),
            _ => panic!("expected single"),
        }
        let multi: LegendSpec = serde_yaml::from_str(r#"["a", "b"]"#).unwrap();
        match multi {
            LegendSpec::Multi(v) => assert_eq!(v, vec!["a".to_string(), "b".to_string()]),
            _ => panic!("expected multi"),
        }
    }

    #[test]
    fn test_threshold_parses_with_color_and_value() {
        let yaml = r#"{value: 100, color: red}"#;
        let t: Threshold = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(t.value, 100.0);
        assert_eq!(t.color, "red");
    }
}
