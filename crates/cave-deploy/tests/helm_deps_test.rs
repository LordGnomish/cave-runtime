// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of ArgoCD reposerver Helm dependency update —
//! upstream `reposerver/repository/repository.go::runHelmDependencyUpdate`
//! (which shells out to `helm dependency build`).  The dependency-resolution
//! logic itself comes from Helm's `pkg/downloader` + `Masterminds/semver`
//! constraint matching; cave-deploy ports the pure-Rust resolution so the
//! umbrella-chart (Helm-of-Helms) Chart.lock can be computed without a
//! subprocess.

use cave_deploy::helm_deps::{
    enabled_dependencies, generate_lock, max_satisfying, parse_chart_yaml, resolve_dependencies,
    semver_satisfies, Chart, ChartDependency,
};
use std::collections::HashMap;

// ─── semver constraint matching (Masterminds/semver subset) ─────────────────

#[test]
fn semver_exact_and_comparators() {
    assert!(semver_satisfies("1.2.3", "1.2.3"));
    assert!(semver_satisfies("1.2.3", "=1.2.3"));
    assert!(!semver_satisfies("1.2.4", "1.2.3"));
    assert!(semver_satisfies("1.2.4", ">1.2.3"));
    assert!(semver_satisfies("1.2.3", ">=1.2.3"));
    assert!(!semver_satisfies("1.2.3", ">1.2.3"));
    assert!(semver_satisfies("1.2.2", "<1.2.3"));
    assert!(semver_satisfies("1.2.3", "<=1.2.3"));
    assert!(semver_satisfies("1.2.4", "!=1.2.3"));
    assert!(!semver_satisfies("1.2.3", "!=1.2.3"));
}

#[test]
fn semver_wildcard_any() {
    assert!(semver_satisfies("9.9.9", "*"));
    assert!(semver_satisfies("0.0.1", ""));
    assert!(semver_satisfies("2.3.4", "x"));
}

#[test]
fn semver_tilde_allows_patch_only() {
    // ~1.2.3 := >=1.2.3, <1.3.0
    assert!(semver_satisfies("1.2.3", "~1.2.3"));
    assert!(semver_satisfies("1.2.9", "~1.2.3"));
    assert!(!semver_satisfies("1.3.0", "~1.2.3"));
    assert!(!semver_satisfies("1.2.2", "~1.2.3"));
}

#[test]
fn semver_caret_allows_minor_and_patch() {
    // ^1.2.3 := >=1.2.3, <2.0.0
    assert!(semver_satisfies("1.2.3", "^1.2.3"));
    assert!(semver_satisfies("1.9.9", "^1.2.3"));
    assert!(!semver_satisfies("2.0.0", "^1.2.3"));
    // ^0.2.3 := >=0.2.3, <0.3.0 (0.x special case)
    assert!(semver_satisfies("0.2.9", "^0.2.3"));
    assert!(!semver_satisfies("0.3.0", "^0.2.3"));
}

#[test]
fn semver_x_range_and_and_or() {
    assert!(semver_satisfies("1.4.7", "1.x"));
    assert!(!semver_satisfies("2.0.0", "1.x"));
    assert!(semver_satisfies("1.2.9", "1.2.x"));
    // comma = AND
    assert!(semver_satisfies("1.5.0", ">=1.2.0, <2.0.0"));
    assert!(!semver_satisfies("2.1.0", ">=1.2.0, <2.0.0"));
    // || = OR
    assert!(semver_satisfies("3.0.0", "1.x || 3.x"));
    assert!(!semver_satisfies("2.0.0", "1.x || 3.x"));
}

#[test]
fn max_satisfying_picks_highest_in_range() {
    let versions = vec![
        "1.2.0".to_string(),
        "1.2.5".to_string(),
        "1.3.0".to_string(),
        "2.0.0".to_string(),
    ];
    assert_eq!(
        max_satisfying(&versions, "^1.2.0").as_deref(),
        Some("1.3.0")
    );
    assert_eq!(
        max_satisfying(&versions, "~1.2.0").as_deref(),
        Some("1.2.5")
    );
    assert_eq!(max_satisfying(&versions, ">=3.0.0"), None);
}

// ─── Chart.yaml parsing ─────────────────────────────────────────────────────

const UMBRELLA_CHART: &str = r#"
apiVersion: v2
name: umbrella
version: 1.0.0
dependencies:
  - name: redis
    version: "^17.0.0"
    repository: "https://charts.bitnami.com/bitnami"
    condition: redis.enabled
  - name: postgresql
    version: "~12.1.0"
    repository: "https://charts.bitnami.com/bitnami"
    alias: db
    tags:
      - database
  - name: nginx
    version: "1.x"
    repository: "https://charts.bitnami.com/bitnami"
    condition: nginx.enabled
"#;

#[test]
fn parse_chart_yaml_reads_dependencies() {
    let chart = parse_chart_yaml(UMBRELLA_CHART).expect("parse");
    assert_eq!(chart.name, "umbrella");
    assert_eq!(chart.version, "1.0.0");
    assert_eq!(chart.dependencies.len(), 3);
    let redis = &chart.dependencies[0];
    assert_eq!(redis.name, "redis");
    assert_eq!(redis.version, "^17.0.0");
    assert_eq!(redis.condition.as_deref(), Some("redis.enabled"));
    let db = &chart.dependencies[1];
    assert_eq!(db.alias.as_deref(), Some("db"));
    assert_eq!(db.tags, vec!["database".to_string()]);
}

// ─── resolution against a repo index ────────────────────────────────────────

fn available_index() -> HashMap<String, Vec<String>> {
    let mut idx = HashMap::new();
    idx.insert(
        "redis".to_string(),
        vec!["16.5.0".into(), "17.3.7".into(), "17.11.3".into(), "18.0.0".into()],
    );
    idx.insert(
        "postgresql".to_string(),
        vec!["12.0.0".into(), "12.1.5".into(), "12.2.0".into()],
    );
    idx.insert(
        "nginx".to_string(),
        vec!["1.2.0".into(), "1.9.1".into(), "2.0.0".into()],
    );
    idx
}

#[test]
fn resolve_dependencies_picks_max_satisfying() {
    let chart = parse_chart_yaml(UMBRELLA_CHART).unwrap();
    let locked = resolve_dependencies(&chart, &available_index()).expect("resolve");
    assert_eq!(locked.len(), 3);
    // ^17.0.0 → highest < 18.0.0 == 17.11.3
    assert_eq!(locked[0].name, "redis");
    assert_eq!(locked[0].version, "17.11.3");
    // ~12.1.0 → highest < 12.2.0 == 12.1.5
    assert_eq!(locked[1].name, "postgresql");
    assert_eq!(locked[1].version, "12.1.5");
    // 1.x → highest < 2.0.0 == 1.9.1
    assert_eq!(locked[2].name, "nginx");
    assert_eq!(locked[2].version, "1.9.1");
}

#[test]
fn resolve_dependencies_errors_when_no_match() {
    let chart = parse_chart_yaml(
        r#"
name: u
version: 1.0.0
dependencies:
  - name: redis
    version: ">=99.0.0"
    repository: "https://charts.bitnami.com/bitnami"
"#,
    )
    .unwrap();
    let err = resolve_dependencies(&chart, &available_index());
    assert!(err.is_err(), "no satisfying version must error");
}

#[test]
fn generate_lock_is_deterministic_and_digested() {
    let chart = parse_chart_yaml(UMBRELLA_CHART).unwrap();
    let lock1 = generate_lock(&chart, &available_index(), "2026-05-30T00:00:00Z").unwrap();
    let lock2 = generate_lock(&chart, &available_index(), "2026-05-30T00:00:00Z").unwrap();
    assert_eq!(lock1.dependencies.len(), 3);
    // digest is content-addressed over resolved deps → stable for same input
    assert_eq!(lock1.digest, lock2.digest);
    assert!(lock1.digest.starts_with("sha256:"));
    assert_eq!(lock1.generated, "2026-05-30T00:00:00Z");
}

// ─── condition / tags enabling (Helm processDependencyConditions) ───────────

#[test]
fn enabled_dependencies_respects_condition_path() {
    let chart = parse_chart_yaml(UMBRELLA_CHART).unwrap();
    // redis.enabled=false disables redis; nginx.enabled unset → default enabled
    let values = serde_json::json!({
        "redis": { "enabled": false },
        "nginx": { "enabled": true },
    });
    let enabled = enabled_dependencies(&chart, &values, &HashMap::new());
    let names: Vec<&str> = enabled.iter().map(|d| d.name.as_str()).collect();
    assert!(!names.contains(&"redis"), "redis disabled by condition");
    assert!(names.contains(&"nginx"));
    // postgresql has no condition → enabled by default
    assert!(names.contains(&"postgresql"));
}

#[test]
fn enabled_dependencies_respects_tag_override() {
    let chart = parse_chart_yaml(UMBRELLA_CHART).unwrap();
    // disable the "database" tag → postgresql (tagged database) drops out
    let mut tags = HashMap::new();
    tags.insert("database".to_string(), false);
    let enabled = enabled_dependencies(&chart, &serde_json::json!({}), &tags);
    let names: Vec<&str> = enabled.iter().map(|d| d.name.as_str()).collect();
    assert!(!names.contains(&"postgresql"), "database tag disabled");
    assert!(names.contains(&"redis"));
}

#[test]
fn chart_dependency_builder_roundtrips() {
    let dep = ChartDependency {
        name: "redis".into(),
        version: "^17.0.0".into(),
        repository: "https://example.com".into(),
        condition: Some("redis.enabled".into()),
        tags: vec!["cache".into()],
        alias: None,
        import_values: vec![],
    };
    let chart = Chart {
        name: "u".into(),
        version: "1.0.0".into(),
        dependencies: vec![dep],
    };
    assert_eq!(chart.dependencies[0].name, "redis");
}
