// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/resources/v1/SearchResource.java
//   src/main/java/org/dependencytrack/search/FuzzyVulnerableSoftwareSearchManager.java
//
//! Cross-entity keyword search — Dependency-Track `SearchResource` parity.
//!
//! Performs a case-insensitive substring search across projects, components,
//! and vulnerabilities.  Mirrors the `GET /api/v1/search` endpoint which
//! fans out to each Lucene index.  We back this with in-memory iteration
//! (no embedded Lucene / tantivy required); the interface is identical.

use crate::components::{ComponentRecord, Project};
use crate::models::VulnIntel;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The type of entity this search result refers to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SearchResultKind {
    Project,
    Component,
    Vulnerability,
    License,
}

/// A single item returned from the cross-entity search.
///
/// Mirrors `org.dependencytrack.resources.v1.vo.SearchResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// UUID of the matched entity.
    pub uuid: Uuid,
    /// Human-readable label (typically `name` or `vuln_id`).
    pub label: String,
    /// Secondary detail (version, severity, etc.).
    pub detail: Option<String>,
    /// Which entity class this result belongs to.
    pub kind: SearchResultKind,
}

/// Maximum number of results returned per search call.
///
/// Mirrors the Dependency-Track default page size guard on search.
pub const MAX_SEARCH_RESULTS: usize = 100;

/// Perform a cross-entity keyword search.
///
/// `query` is matched case-insensitively as a substring against:
/// - `Project.name`, `Project.version`, `Project.purl`
/// - `ComponentRecord.name`, `ComponentRecord.version`, `ComponentRecord.purl`,
///   `ComponentRecord.license`
/// - `VulnIntel.vuln_id`, `VulnIntel.title`, `VulnIntel.description`
///
/// An empty `query` returns an empty list (mirrors upstream 400 guard).
/// Results are capped at [`MAX_SEARCH_RESULTS`].
pub fn search_all(
    query: &str,
    projects: &[Project],
    components: &[ComponentRecord],
    vulns: &[VulnIntel],
) -> Vec<SearchResult> {
    if query.is_empty() {
        return Vec::new();
    }

    let q = query.to_lowercase();
    let mut results: Vec<SearchResult> = Vec::new();

    // ── Projects ────────────────────────────────────────────────────────────
    for p in projects {
        if matches_project(&q, p) {
            results.push(SearchResult {
                uuid: p.uuid,
                label: p.name.clone(),
                detail: p.version.clone(),
                kind: SearchResultKind::Project,
            });
        }
        if results.len() >= MAX_SEARCH_RESULTS {
            return results;
        }
    }

    // ── Components ──────────────────────────────────────────────────────────
    for c in components {
        if matches_component(&q, c) {
            results.push(SearchResult {
                uuid: c.uuid,
                label: c.name.clone(),
                detail: Some(c.version.clone()),
                kind: SearchResultKind::Component,
            });
        }
        if results.len() >= MAX_SEARCH_RESULTS {
            return results;
        }
    }

    // ── Vulnerabilities ─────────────────────────────────────────────────────
    for v in vulns {
        if matches_vuln(&q, v) {
            results.push(SearchResult {
                uuid: v.id,
                label: v.vuln_id.clone(),
                detail: Some(v.title.clone()),
                kind: SearchResultKind::Vulnerability,
            });
        }
        if results.len() >= MAX_SEARCH_RESULTS {
            return results;
        }
    }

    results
}

fn matches_project(q: &str, p: &Project) -> bool {
    p.name.to_lowercase().contains(q)
        || p.version
            .as_deref()
            .map(|v| v.to_lowercase().contains(q))
            .unwrap_or(false)
        || p.purl
            .as_deref()
            .map(|v| v.to_lowercase().contains(q))
            .unwrap_or(false)
        || p.group
            .as_deref()
            .map(|v| v.to_lowercase().contains(q))
            .unwrap_or(false)
}

fn matches_component(q: &str, c: &ComponentRecord) -> bool {
    c.name.to_lowercase().contains(q)
        || c.version.to_lowercase().contains(q)
        || c.purl
            .as_deref()
            .map(|v| v.to_lowercase().contains(q))
            .unwrap_or(false)
        || c.license
            .as_deref()
            .map(|v| v.to_lowercase().contains(q))
            .unwrap_or(false)
        || c.group
            .as_deref()
            .map(|v| v.to_lowercase().contains(q))
            .unwrap_or(false)
}

fn matches_vuln(q: &str, v: &VulnIntel) -> bool {
    v.vuln_id.to_lowercase().contains(q)
        || v.title.to_lowercase().contains(q)
        || v.description.to_lowercase().contains(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AffectedRange, AnalysisState, Severity, VulnSource};

    fn mk_project(name: &str) -> Project {
        Project::new(name, Some("1.0.0".into()))
    }

    fn mk_comp(name: &str) -> ComponentRecord {
        let mut c = ComponentRecord::new(Uuid::new_v4(), name, "1.0");
        c.purl = Some(format!("pkg:npm/{}@1.0", name));
        c
    }

    fn mk_vuln(id: &str, title: &str) -> VulnIntel {
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: id.into(),
            source: VulnSource::Nvd,
            title: title.into(),
            description: "desc".into(),
            severity: Severity::High,
            cvss_v3_base: Some(7.5),
            cvss_v3_vector: None,
            cvss_v2_base: None,
            epss_score: None,
            epss_percentile: None,
            cwes: vec![],
            references: vec![],
            affected: vec![AffectedRange {
                purl_type: "npm".into(),
                namespace: None,
                name: "x".into(),
                vers: "*".into(),
                fixed: None,
            }],
            published: None,
            modified: None,
            state: AnalysisState::NotSet,
        }
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(search_all("", &[mk_project("x")], &[], &[]).is_empty());
    }

    #[test]
    fn project_found_by_name() {
        let r = search_all("my-app", &[mk_project("my-app")], &[], &[]);
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0].kind, SearchResultKind::Project));
    }

    #[test]
    fn component_found_by_name() {
        let r = search_all("lodash", &[], &[mk_comp("lodash")], &[]);
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0].kind, SearchResultKind::Component));
    }

    #[test]
    fn vuln_found_by_id() {
        let r = search_all(
            "CVE-2021-44228",
            &[],
            &[],
            &[mk_vuln("CVE-2021-44228", "Log4Shell")],
        );
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0].kind, SearchResultKind::Vulnerability));
    }

    #[test]
    fn no_match_is_empty() {
        let r = search_all("xyzzy", &[mk_project("other")], &[], &[]);
        assert!(r.is_empty());
    }

    #[test]
    fn results_capped_at_max() {
        let comps: Vec<_> = (0..200).map(|i| mk_comp(&format!("lib-{}", i))).collect();
        let r = search_all("lib-", &[], &comps, &[]);
        assert!(r.len() <= MAX_SEARCH_RESULTS);
    }

    #[test]
    fn case_insensitive_match() {
        let r = search_all("LODASH", &[], &[mk_comp("lodash")], &[]);
        assert_eq!(r.len(), 1);
    }
}
