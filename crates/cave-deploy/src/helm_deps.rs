// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Helm-of-Helms dependency resolver — `Chart.yaml::dependencies` +
//! `Chart.lock` modeling, version-range matching, multi-source repo
//! collection, topological resolve order.
//!
//! NOTICE: upstream is argoproj/argo-cd (Apache-2.0)
//! `reposerver/repository/repository.go::runHelmDependencyUpdate`,
//! itself wrapping `helm/helm v3` chart-dependency commands. cave-deploy
//! owns the **pure-Rust** dependency-graph + range-check; the
//! tar.gz fetch + cache write live out-of-process per
//! ADR-RUNTIME-SANDBOX-NO-FFI-001 (helm-fetch shells out to `helm`).

use crate::error::DeployError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

type Result<T> = std::result::Result<T, DeployError>;

/// One entry under `Chart.yaml::dependencies`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartDependency {
    pub name: String,
    /// SemVer range — `~1.2.3`, `^1`, `>=1.0,<2`, `1.x`, or exact.
    pub version: String,
    /// Helm repo URL (`https://...`) or `@reponame` alias resolved from
    /// `repositories.yaml`.
    pub repository: String,
    /// Optional alias to import-under (multi-source repos may produce
    /// name collisions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Optional condition expression — `enabled` / `feature.flag`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Optional tag list — `--set tags.foo=true` selects deps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `import-values` — Helm 3 child→parent value pass-through.
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "import-values")]
    pub import_values: Vec<String>,
}

/// `Chart.lock` shape — one entry per *resolved* dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartLockEntry {
    pub name: String,
    /// Concrete version (no range).
    pub version: String,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartLock {
    pub dependencies: Vec<ChartLockEntry>,
    /// Hex SHA-256 digest of the resolved dependency set.
    pub digest: String,
    pub generated: String,
}

/// Parse a `Chart.yaml` body and return only its `dependencies` list.
pub fn parse_chart_yaml_dependencies(body: &str) -> Result<Vec<ChartDependency>> {
    #[derive(Deserialize)]
    struct ChartFile {
        #[serde(default)]
        dependencies: Vec<ChartDependency>,
    }
    let c: ChartFile = serde_yaml::from_str(body)
        .map_err(|e| DeployError::Internal(format!("Chart.yaml: {e}")))?;
    Ok(c.dependencies)
}

pub fn parse_chart_lock(body: &str) -> Result<ChartLock> {
    serde_yaml::from_str(body).map_err(|e| DeployError::Internal(format!("Chart.lock: {e}")))
}

/// Validate a `Chart.lock` against the source `Chart.yaml::dependencies`:
/// every declared dep must appear in the lock and its concrete version
/// must satisfy the declared range.
pub fn validate_lock(deps: &[ChartDependency], lock: &ChartLock) -> Result<()> {
    let lock_by_name: BTreeMap<&str, &ChartLockEntry> =
        lock.dependencies.iter().map(|e| (e.name.as_str(), e)).collect();
    for d in deps {
        let Some(entry) = lock_by_name.get(d.name.as_str()) else {
            return Err(DeployError::Internal(format!(
                "Chart.lock missing dependency '{}' declared in Chart.yaml", d.name
            )));
        };
        if !version_satisfies(&entry.version, &d.version) {
            return Err(DeployError::Internal(format!(
                "Chart.lock '{}' resolved to {} which does not satisfy range '{}'",
                d.name, entry.version, d.version
            )));
        }
        if entry.repository != d.repository {
            return Err(DeployError::Internal(format!(
                "Chart.lock '{}' uses repository '{}' which does not match Chart.yaml repository '{}'",
                d.name, entry.repository, d.repository
            )));
        }
    }
    Ok(())
}

/// Build a topological resolve order over the **declared** chart-name
/// dependencies. Helm-of-Helms umbrella charts may reference subcharts
/// that in turn reference others; here we accept an adjacency map
/// (parent → child names) and return a stable post-order so caching
/// fetchers can resolve leaves first.
///
/// Cycles return Err.
pub fn topo_resolve_order(adjacency: &BTreeMap<String, Vec<String>>) -> Result<Vec<String>> {
    // Kahn's algorithm on the reverse graph (child → parent edges) so
    // we emit leaves first, then their parents, then roots last.
    let mut indeg: BTreeMap<String, usize> = BTreeMap::new();
    let mut reverse: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut all_nodes: BTreeSet<String> = BTreeSet::new();
    for (p, kids) in adjacency {
        all_nodes.insert(p.clone());
        indeg.entry(p.clone()).or_insert(0);
        for k in kids {
            all_nodes.insert(k.clone());
            *indeg.entry(p.clone()).or_insert(0) += 1;
            reverse.entry(k.clone()).or_default().push(p.clone());
        }
    }
    for n in &all_nodes {
        indeg.entry(n.clone()).or_insert(0);
    }

    let mut ready: VecDeque<String> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| n.clone())
        .collect();
    let mut out: Vec<String> = Vec::new();
    while let Some(n) = ready.pop_front() {
        out.push(n.clone());
        if let Some(parents) = reverse.get(&n) {
            for p in parents {
                let entry = indeg.get_mut(p).expect("indeg seeded");
                *entry -= 1;
                if *entry == 0 {
                    ready.push_back(p.clone());
                }
            }
        }
    }
    if out.len() != all_nodes.len() {
        return Err(DeployError::Internal(format!(
            "Chart.yaml dependencies form a cycle: emitted {} of {} nodes",
            out.len(), all_nodes.len()
        )));
    }
    Ok(out)
}

/// Group declared dependencies by repository URL so the fetcher can
/// batch by-repo (one repo `helm dep update` invocation = N charts).
pub fn group_by_repository(deps: &[ChartDependency]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in deps {
        out.entry(d.repository.clone()).or_default().push(d.name.clone());
    }
    out
}

/// Apply tags + conditions to filter the active dependency set.
/// `enabled_tags` may include `*` to enable all.
pub fn select_active<'a>(
    deps: &'a [ChartDependency],
    enabled_tags: &BTreeSet<String>,
    conditions: &BTreeMap<String, bool>,
) -> Vec<&'a ChartDependency> {
    let star = enabled_tags.contains("*");
    deps.iter()
        .filter(|d| {
            // Condition false → exclude. Absent / true → keep.
            if let Some(c) = &d.condition {
                if matches!(conditions.get(c), Some(false)) {
                    return false;
                }
            }
            // No tags → always-active.
            if d.tags.is_empty() {
                return true;
            }
            star || d.tags.iter().any(|t| enabled_tags.contains(t))
        })
        .collect()
}

// ── SemVer subset (Helm-style range) ────────────────────────────────────────
//
// Helm uses the `semver` Go library's range syntax. We implement the
// common cases: exact (`1.2.3`), wildcard suffix (`1.x`, `1.2.x`), caret
// (`^1.2.3`), tilde (`~1.2.3`), and conjunction with `,` (`>=1.0,<2.0`).
// Pre-release tags are kept on equality; release-only comparisons.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Triple(u64, u64, u64);

fn parse_triple(s: &str) -> Option<Triple> {
    // Strip optional `v` prefix and pre-release suffix.
    let s = s.trim().trim_start_matches('v');
    let core = s.split(|c| c == '-' || c == '+').next().unwrap_or(s);
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    Some(Triple(a, b, c))
}

pub fn version_satisfies(version: &str, range: &str) -> bool {
    let Some(v) = parse_triple(version) else { return false };
    let range = range.trim();
    if range.is_empty() { return true; }
    range.split(',').map(str::trim).filter(|s| !s.is_empty()).all(|clause| {
        single_clause_match(v, clause)
    })
}

fn single_clause_match(v: Triple, clause: &str) -> bool {
    let clause = clause.trim();
    // Wildcard `x` / `*` forms — depth-aware (1.x = major-only, 1.2.x = major+minor).
    if let Some(stripped) = clause.strip_suffix(".x")
        .or_else(|| clause.strip_suffix(".*"))
    {
        let dots = stripped.chars().filter(|c| *c == '.').count();
        let prefix = parse_triple(stripped);
        if let Some(p) = prefix {
            return match dots {
                0 => v.0 == p.0,
                _ => v.0 == p.0 && v.1 == p.1,
            };
        }
    }
    if clause == "*" || clause == "x" { return true; }
    if let Some(rest) = clause.strip_prefix('^') {
        let Some(t) = parse_triple(rest) else { return false };
        return v >= t && v.0 == t.0;
    }
    if let Some(rest) = clause.strip_prefix('~') {
        let Some(t) = parse_triple(rest) else { return false };
        return v >= t && v.0 == t.0 && v.1 == t.1;
    }
    if let Some(rest) = clause.strip_prefix(">=") {
        return parse_triple(rest).map(|t| v >= t).unwrap_or(false);
    }
    if let Some(rest) = clause.strip_prefix("<=") {
        return parse_triple(rest).map(|t| v <= t).unwrap_or(false);
    }
    if let Some(rest) = clause.strip_prefix('>') {
        return parse_triple(rest).map(|t| v > t).unwrap_or(false);
    }
    if let Some(rest) = clause.strip_prefix('<') {
        return parse_triple(rest).map(|t| v < t).unwrap_or(false);
    }
    if let Some(rest) = clause.strip_prefix("==") {
        return parse_triple(rest).map(|t| v == t).unwrap_or(false);
    }
    parse_triple(clause).map(|t| v == t).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(name: &str, ver: &str, repo: &str) -> ChartDependency {
        ChartDependency {
            name: name.into(),
            version: ver.into(),
            repository: repo.into(),
            alias: None,
            condition: None,
            tags: vec![],
            import_values: vec![],
        }
    }

    #[test]
    fn parse_chart_yaml_deps_reads_minimal_form() {
        let y = r#"
apiVersion: v2
name: umbrella
version: 1.0.0
dependencies:
  - name: postgres
    version: ~15.0.0
    repository: https://charts.example.org
  - name: redis
    version: ^7
    repository: https://charts.example.org
"#;
        let deps = parse_chart_yaml_dependencies(y).unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "postgres");
        assert_eq!(deps[1].version, "^7");
    }

    #[test]
    fn parse_chart_yaml_missing_deps_returns_empty() {
        let y = "apiVersion: v2\nname: x\nversion: 0\n";
        assert!(parse_chart_yaml_dependencies(y).unwrap().is_empty());
    }

    #[test]
    fn parse_chart_lock_round_trip() {
        let l = ChartLock {
            dependencies: vec![ChartLockEntry { name: "postgres".into(), version: "15.4.2".into(), repository: "https://r".into() }],
            digest: "sha256:abc".into(),
            generated: "2026-05-24T00:00:00Z".into(),
        };
        let y = serde_yaml::to_string(&l).unwrap();
        let r = parse_chart_lock(&y).unwrap();
        assert_eq!(r, l);
    }

    #[test]
    fn validate_lock_missing_dep_errors() {
        let deps = vec![dep("postgres", "^15", "https://r")];
        let lock = ChartLock { dependencies: vec![], digest: "x".into(), generated: "".into() };
        assert!(validate_lock(&deps, &lock).is_err());
    }

    #[test]
    fn validate_lock_version_out_of_range_errors() {
        let deps = vec![dep("postgres", "^15.0.0", "https://r")];
        let lock = ChartLock {
            dependencies: vec![ChartLockEntry { name: "postgres".into(), version: "16.0.0".into(), repository: "https://r".into() }],
            digest: "x".into(), generated: "".into(),
        };
        assert!(validate_lock(&deps, &lock).is_err());
    }

    #[test]
    fn validate_lock_repo_mismatch_errors() {
        let deps = vec![dep("postgres", "^15", "https://r")];
        let lock = ChartLock {
            dependencies: vec![ChartLockEntry { name: "postgres".into(), version: "15.1.0".into(), repository: "https://other".into() }],
            digest: "x".into(), generated: "".into(),
        };
        assert!(validate_lock(&deps, &lock).is_err());
    }

    #[test]
    fn validate_lock_happy_path() {
        let deps = vec![dep("postgres", "^15", "https://r")];
        let lock = ChartLock {
            dependencies: vec![ChartLockEntry { name: "postgres".into(), version: "15.4.2".into(), repository: "https://r".into() }],
            digest: "x".into(), generated: "".into(),
        };
        validate_lock(&deps, &lock).unwrap();
    }

    #[test]
    fn topo_order_leaves_first() {
        let mut g = BTreeMap::new();
        g.insert("umbrella".into(), vec!["pg".into(), "redis".into()]);
        g.insert("pg".into(), vec!["common".into()]);
        g.insert("redis".into(), vec!["common".into()]);
        let order = topo_resolve_order(&g).unwrap();
        let pos = |n: &str| order.iter().position(|s| s == n).unwrap();
        assert!(pos("common") < pos("pg"));
        assert!(pos("common") < pos("redis"));
        assert!(pos("pg") < pos("umbrella"));
    }

    #[test]
    fn topo_order_rejects_cycle() {
        let mut g = BTreeMap::new();
        g.insert("a".into(), vec!["b".into()]);
        g.insert("b".into(), vec!["a".into()]);
        assert!(topo_resolve_order(&g).is_err());
    }

    #[test]
    fn group_by_repository_clusters_deps() {
        let deps = vec![
            dep("pg", "1", "https://a"),
            dep("redis", "1", "https://a"),
            dep("vault", "1", "https://b"),
        ];
        let g = group_by_repository(&deps);
        assert_eq!(g["https://a"].len(), 2);
        assert_eq!(g["https://b"].len(), 1);
    }

    #[test]
    fn select_active_filters_by_condition_and_tag() {
        let mut d_off = dep("off", "1", "r"); d_off.condition = Some("feat.off".into());
        let mut d_tag = dep("tagged", "1", "r"); d_tag.tags = vec!["heavy".into()];
        let mut d_other = dep("other", "1", "r"); d_other.tags = vec!["light".into()];
        let always = dep("always", "1", "r");
        let deps = vec![d_off, d_tag, d_other, always];
        let mut cond = BTreeMap::new();
        cond.insert("feat.off".into(), false);
        let mut tags = BTreeSet::new();
        tags.insert("heavy".into());
        let active = select_active(&deps, &tags, &cond);
        let names: Vec<&str> = active.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["tagged", "always"]);
    }

    #[test]
    fn select_active_star_includes_all_tagged() {
        let mut d1 = dep("a", "1", "r"); d1.tags = vec!["x".into()];
        let mut d2 = dep("b", "1", "r"); d2.tags = vec!["y".into()];
        let deps = vec![d1, d2];
        let mut tags = BTreeSet::new();
        tags.insert("*".into());
        let active = select_active(&deps, &tags, &BTreeMap::new());
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn version_satisfies_exact_match() {
        assert!(version_satisfies("1.2.3", "1.2.3"));
        assert!(!version_satisfies("1.2.4", "1.2.3"));
    }

    #[test]
    fn version_satisfies_caret_minor_bump_ok_major_not() {
        assert!(version_satisfies("1.5.0", "^1.2.3"));
        assert!(version_satisfies("1.2.3", "^1.2.3"));
        assert!(!version_satisfies("2.0.0", "^1.2.3"));
        assert!(!version_satisfies("1.2.2", "^1.2.3"));
    }

    #[test]
    fn version_satisfies_tilde_patch_bump_only() {
        assert!(version_satisfies("1.2.9", "~1.2.3"));
        assert!(!version_satisfies("1.3.0", "~1.2.3"));
    }

    #[test]
    fn version_satisfies_wildcard_x() {
        assert!(version_satisfies("1.4.0", "1.x"));
        assert!(!version_satisfies("2.0.0", "1.x"));
        assert!(version_satisfies("1.2.9", "1.2.x"));
        assert!(!version_satisfies("1.3.0", "1.2.x"));
    }

    #[test]
    fn version_satisfies_conjunction() {
        assert!(version_satisfies("1.5.0", ">=1.0,<2.0"));
        assert!(!version_satisfies("2.0.0", ">=1.0,<2.0"));
    }

    #[test]
    fn version_satisfies_v_prefix_and_pre_release_stripped() {
        assert!(version_satisfies("v1.2.3", "1.2.3"));
        assert!(version_satisfies("1.2.3-rc1", "1.2.3"));
    }
}
