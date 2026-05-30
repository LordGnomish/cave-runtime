// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Helm dependency resolution — pure-Rust port of ArgoCD reposerver's
//! `helm dependency build` path (`reposerver/repository/repository.go::
//! runHelmDependencyUpdate`), whose resolution logic lives in Helm's
//! `pkg/downloader` + `Masterminds/semver`.
//!
//! Upstream shells out to the `helm` binary; cave-deploy ports the umbrella
//! (Helm-of-Helms) resolution so a `Chart.lock` can be computed in-process
//! without a subprocess:
//!
//!   * [`parse_chart_yaml`]        — read `Chart.yaml` `dependencies:`
//!   * [`semver_satisfies`] /      — Masterminds/semver constraint subset
//!     [`max_satisfying`]
//!   * [`resolve_dependencies`]    — pick the highest matching version per dep
//!   * [`generate_lock`]           — produce a content-digested `Chart.lock`
//!   * [`enabled_dependencies`]    — Helm `processDependencyConditions` /
//!                                   `processDependencyTags`
//!
//! The actual chart download + extraction (network + tar) remains deferred to
//! the Phase 2 `cave-helm-runtime`; this module owns the resolution algebra.

use crate::error::DeployError;
use serde::Deserialize;
use std::collections::HashMap;

// ─── Chart model ────────────────────────────────────────────────────────────

/// A single entry under `Chart.yaml` `dependencies:`.
#[derive(Debug, Clone)]
pub struct ChartDependency {
    pub name: String,
    /// SemVer constraint string (Masterminds syntax).
    pub version: String,
    pub repository: String,
    /// A values path (e.g. `redis.enabled`) gating this dependency.
    pub condition: Option<String>,
    /// Tags this dependency belongs to.
    pub tags: Vec<String>,
    /// Optional alias the subchart is mounted under.
    pub alias: Option<String>,
    /// `import-values` mappings (carried for completeness).
    pub import_values: Vec<String>,
}

/// Parsed `Chart.yaml` (only the fields the resolver needs).
#[derive(Debug, Clone)]
pub struct Chart {
    pub name: String,
    pub version: String,
    pub dependencies: Vec<ChartDependency>,
}

/// One resolved (locked) dependency.
#[derive(Debug, Clone)]
pub struct LockedDependency {
    pub name: String,
    pub repository: String,
    pub version: String,
}

/// Generated `Chart.lock`.
#[derive(Debug, Clone)]
pub struct ChartLock {
    pub dependencies: Vec<LockedDependency>,
    /// `sha256:<hex>` over the resolved dependency set (Helm `Lock.Digest`).
    pub digest: String,
    /// RFC3339 generation timestamp.
    pub generated: String,
}

// ─── Chart.yaml parsing ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RawChart {
    name: String,
    version: String,
    #[serde(default)]
    dependencies: Vec<RawDep>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawDep {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    condition: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    import_values: Vec<String>,
}

/// Parse a `Chart.yaml` document into a [`Chart`].
pub fn parse_chart_yaml(text: &str) -> Result<Chart, DeployError> {
    let raw: RawChart = serde_yaml::from_str(text)?;
    Ok(Chart {
        name: raw.name,
        version: raw.version,
        dependencies: raw
            .dependencies
            .into_iter()
            .map(|d| ChartDependency {
                name: d.name,
                version: d.version,
                repository: d.repository,
                condition: d.condition.filter(|s| !s.is_empty()),
                tags: d.tags,
                alias: d.alias.filter(|s| !s.is_empty()),
                import_values: d.import_values,
            })
            .collect(),
    })
}

// ─── SemVer (Masterminds subset) ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    fn parse(s: &str) -> Option<Version> {
        // strip leading `v`, build metadata (`+...`) and pre-release (`-...`).
        let s = s.trim().trim_start_matches('v');
        let s = s.split('+').next().unwrap_or(s);
        let s = s.split('-').next().unwrap_or(s);
        let mut it = s.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some(Version {
            major,
            minor,
            patch,
        })
    }
}

/// Does `version` satisfy `constraint`?  Supports `||` (OR) of comma/space
/// separated AND groups, each group an exact/`=`/`!=`/`>`/`<`/`>=`/`<=`
/// comparator, a `~`/`^` range, or an `x`/`*` wildcard range.
pub fn semver_satisfies(version: &str, constraint: &str) -> bool {
    let Some(v) = Version::parse(version) else {
        return false;
    };
    let constraint = constraint.trim();
    if constraint.is_empty() || constraint == "*" || constraint == "x" || constraint == "X" {
        return true;
    }
    // OR groups
    constraint
        .split("||")
        .any(|group| and_group_satisfied(v, group))
}

fn and_group_satisfied(v: Version, group: &str) -> bool {
    // split on commas first, then whitespace, for AND terms.
    group
        .split(',')
        .flat_map(|s| s.split_whitespace())
        .filter(|t| !t.is_empty())
        .all(|term| term_satisfied(v, term))
}

fn term_satisfied(v: Version, term: &str) -> bool {
    let term = term.trim();
    if term.is_empty() || term == "*" || term == "x" || term == "X" {
        return true;
    }
    // operator-prefixed comparators
    for (op, len) in [(">=", 2), ("<=", 2), ("!=", 2), (">", 1), ("<", 1), ("=", 1)] {
        if let Some(rest) = term.strip_prefix(op) {
            let _ = len;
            let Some(rv) = Version::parse(rest) else {
                return false;
            };
            return match op {
                ">=" => v >= rv,
                "<=" => v <= rv,
                "!=" => v != rv,
                ">" => v > rv,
                "<" => v < rv,
                "=" => v == rv,
                _ => false,
            };
        }
    }
    // tilde: ~1.2.3 → >=1.2.3 <1.(minor+1).0 ; ~1.2 → >=1.2.0 <1.3.0
    if let Some(rest) = term.strip_prefix('~') {
        if let Some(rv) = Version::parse(rest) {
            let upper = Version {
                major: rv.major,
                minor: rv.minor + 1,
                patch: 0,
            };
            return v >= rv && v < upper;
        }
        return false;
    }
    // caret: ^1.2.3 → >=1.2.3 <2.0.0 ; ^0.2.3 → >=0.2.3 <0.3.0 ; ^0.0.3 → <0.0.4
    if let Some(rest) = term.strip_prefix('^') {
        if let Some(rv) = Version::parse(rest) {
            let upper = if rv.major > 0 {
                Version {
                    major: rv.major + 1,
                    minor: 0,
                    patch: 0,
                }
            } else if rv.minor > 0 {
                Version {
                    major: 0,
                    minor: rv.minor + 1,
                    patch: 0,
                }
            } else {
                Version {
                    major: 0,
                    minor: 0,
                    patch: rv.patch + 1,
                }
            };
            return v >= rv && v < upper;
        }
        return false;
    }
    // x-range: 1.x / 1.2.x / 1.* (wildcard in a component)
    if term.contains('x') || term.contains('X') || term.contains('*') {
        return x_range_satisfied(v, term);
    }
    // bare exact version
    match Version::parse(term) {
        Some(rv) => v == rv,
        None => false,
    }
}

fn x_range_satisfied(v: Version, term: &str) -> bool {
    let parts: Vec<&str> = term.trim_start_matches('v').split('.').collect();
    let is_wild = |p: &str| p == "x" || p == "X" || p == "*";
    // major
    let Some(maj) = parts.first() else {
        return false;
    };
    if is_wild(maj) {
        return true;
    }
    let Ok(maj) = maj.parse::<u64>() else {
        return false;
    };
    if v.major != maj {
        return false;
    }
    match parts.get(1) {
        None => true,
        Some(p) if is_wild(p) => true,
        Some(p) => {
            let Ok(min) = p.parse::<u64>() else {
                return false;
            };
            if v.minor != min {
                return false;
            }
            match parts.get(2) {
                None => true,
                Some(p) if is_wild(p) => true,
                Some(p) => p.parse::<u64>().map(|pt| v.patch == pt).unwrap_or(false),
            }
        }
    }
}

/// Highest version in `versions` that satisfies `constraint`, if any.
pub fn max_satisfying(versions: &[String], constraint: &str) -> Option<String> {
    versions
        .iter()
        .filter(|v| semver_satisfies(v, constraint))
        .filter_map(|v| Version::parse(v).map(|parsed| (parsed, v.clone())))
        .max_by_key(|(parsed, _)| *parsed)
        .map(|(_, raw)| raw)
}

// ─── Resolution ─────────────────────────────────────────────────────────────

/// Resolve every dependency in `chart` against an `available` index mapping
/// chart name → list of published versions.  Mirrors Helm's
/// `resolveRepoNames` + `resolve` (highest satisfying wins).
pub fn resolve_dependencies(
    chart: &Chart,
    available: &HashMap<String, Vec<String>>,
) -> Result<Vec<LockedDependency>, DeployError> {
    let mut locked = Vec::with_capacity(chart.dependencies.len());
    for dep in &chart.dependencies {
        let versions = available.get(&dep.name).ok_or_else(|| {
            DeployError::Invalid(format!(
                "no chart named '{}' found in repository index",
                dep.name
            ))
        })?;
        let chosen = max_satisfying(versions, &dep.version).ok_or_else(|| {
            DeployError::Invalid(format!(
                "can't get a valid version for dependency '{}' ({})",
                dep.name, dep.version
            ))
        })?;
        locked.push(LockedDependency {
            name: dep.name.clone(),
            repository: dep.repository.clone(),
            version: chosen,
        });
    }
    Ok(locked)
}

/// Build a `Chart.lock` from resolved dependencies, with a content digest over
/// the resolved set (Helm's `lock.Digest`).
pub fn generate_lock(
    chart: &Chart,
    available: &HashMap<String, Vec<String>>,
    generated: &str,
) -> Result<ChartLock, DeployError> {
    let deps = resolve_dependencies(chart, available)?;
    let mut canonical = String::new();
    for d in &deps {
        canonical.push_str(&format!("{}|{}|{}\n", d.name, d.repository, d.version));
    }
    Ok(ChartLock {
        digest: format!("sha256:{}", fnv_hex(&canonical)),
        dependencies: deps,
        generated: generated.to_string(),
    })
}

/// Deterministic 64-bit FNV-1a digest rendered as 16-hex (dependency-light
/// stand-in for the sha256 Helm uses; identical input → identical digest).
fn fnv_hex(s: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", hash)
}

// ─── Condition / tag enabling (Helm processDependency*) ─────────────────────

/// Resolve which dependencies are enabled, applying — in Helm's precedence —
/// `condition` value paths first (highest priority), then `tags` overrides.
/// A dependency with neither a satisfied condition nor a matching tag override
/// defaults to enabled.
pub fn enabled_dependencies<'a>(
    chart: &'a Chart,
    values: &serde_json::Value,
    tag_overrides: &HashMap<String, bool>,
) -> Vec<&'a ChartDependency> {
    chart
        .dependencies
        .iter()
        .filter(|dep| dependency_enabled(dep, values, tag_overrides))
        .collect()
}

fn dependency_enabled(
    dep: &ChartDependency,
    values: &serde_json::Value,
    tag_overrides: &HashMap<String, bool>,
) -> bool {
    // condition wins (Helm: first resolvable condition path decides).
    if let Some(cond) = &dep.condition {
        // condition may be a comma list of paths; first resolvable one decides.
        for path in cond.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if let Some(b) = lookup_bool(values, path) {
                return b;
            }
        }
    }
    // tags: enabled if ANY of the dependency's tags is enabled; if every tag
    // is explicitly disabled the dependency drops out.
    if !dep.tags.is_empty() {
        let mut any_explicit = false;
        let mut any_enabled = false;
        for tag in &dep.tags {
            if let Some(&v) = tag_overrides.get(tag) {
                any_explicit = true;
                any_enabled |= v;
            } else {
                // unset tag defaults to enabled
                any_enabled = true;
            }
        }
        if any_explicit {
            return any_enabled;
        }
    }
    // default
    true
}

/// Resolve a dotted `a.b.c` path to a boolean within `values`, if present.
fn lookup_bool(values: &serde_json::Value, path: &str) -> Option<bool> {
    let mut cur = values;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    cur.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_strips_prerelease_and_v() {
        assert_eq!(Version::parse("v1.2.3").unwrap().major, 1);
        assert_eq!(Version::parse("1.2.3-rc.1").unwrap().patch, 3);
        assert_eq!(Version::parse("1.2").unwrap().patch, 0);
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv_hex("redis"), fnv_hex("redis"));
        assert_ne!(fnv_hex("redis"), fnv_hex("nginx"));
    }
}
