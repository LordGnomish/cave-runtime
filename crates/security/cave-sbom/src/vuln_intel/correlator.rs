// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/tasks/IntegrityAnalysisTask.java
//   src/main/java/org/dependencytrack/parser/{nvd,osv,github}
//
//! Cross-source vulnerability correlator.
//!
//! Each upstream feed (NVD / OSV / GHSA / Snyk) emits advisories with
//! overlapping coverage. The correlator joins them by `vuln_id` and any
//! cross-source aliases (e.g. NVD `CVE-2023-1` ↔ GHSA `GHSA-x-y-z`), picks
//! the authoritative record per priority order, and yields a single
//! `CorrelatedAdvisory` per identifier cluster.

use crate::models::{Severity, VulnIntel, VulnSource};
use std::collections::{BTreeMap, HashMap};

/// Priority order: lower index wins ties. NVD is authoritative for
/// `cvss_v3_base`; OSV for affected ranges; GHSA for ecosystem-specific
/// metadata. Snyk last (license-permitting subset).
fn source_priority(s: &VulnSource) -> u8 {
    match s {
        VulnSource::Nvd => 0,
        VulnSource::Osv => 1,
        VulnSource::Ghsa => 2,
        VulnSource::Snyk => 3,
        VulnSource::VulnDb => 4,
        VulnSource::Github => 5,
        VulnSource::Internal => 6,
    }
}

/// Output of correlation: one record per identifier cluster.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrelatedAdvisory {
    /// Canonical ID (CVE-… preferred; falls back to first ID encountered).
    pub canonical_id: String,
    /// Every aliased identifier across sources.
    pub aliases: Vec<String>,
    /// The source contributing the authoritative record (lowest priority).
    pub authoritative_source: VulnSource,
    /// Authoritative title (from the authoritative source).
    pub title: String,
    /// Highest severity observed across sources (used as the gating value).
    pub severity: Severity,
    /// Highest CVSS-v3 base observed across sources.
    pub max_cvss_v3: Option<f32>,
    /// Sources that contributed records.
    pub sources_present: Vec<VulnSource>,
    /// Aggregated underlying `VulnIntel` records, lowest priority first.
    pub records: Vec<VulnIntel>,
}

#[derive(Debug, Clone, Default)]
pub struct CorrelationStats {
    pub input_records: usize,
    pub correlated_clusters: usize,
    pub multi_source_clusters: usize,
}

/// `aliases` carries known cross-source equivalences. Each tuple
/// `(a, b)` asserts that `a` and `b` are the same vulnerability.
pub fn correlate(
    records: Vec<VulnIntel>,
    aliases: &[(String, String)],
) -> (Vec<CorrelatedAdvisory>, CorrelationStats) {
    let mut union = UnionFind::default();
    for v in &records {
        union.insert(&v.vuln_id);
    }
    for (a, b) in aliases {
        union.union(a, b);
    }

    let mut by_cluster: BTreeMap<String, Vec<VulnIntel>> = BTreeMap::new();
    for v in records.iter() {
        let root = union.find(&v.vuln_id);
        by_cluster.entry(root).or_default().push(v.clone());
    }

    let input_records = records.len();
    let mut multi_source = 0;
    let mut out = Vec::new();
    for (_, mut items) in by_cluster {
        items.sort_by_key(|v| source_priority(&v.source));
        let authoritative = items.first().cloned().expect("non-empty cluster");
        let sources: Vec<VulnSource> = items.iter().map(|v| v.source.clone()).collect();
        let mut unique_sources: Vec<VulnSource> = Vec::new();
        for s in &sources {
            if !unique_sources.iter().any(|u| u == s) {
                unique_sources.push(s.clone());
            }
        }
        if unique_sources.len() > 1 {
            multi_source += 1;
        }
        let max_cvss = items.iter().filter_map(|v| v.cvss_v3_base).fold(None, |acc, x| {
            Some(acc.map(|a: f32| a.max(x)).unwrap_or(x))
        });
        let severity = items
            .iter()
            .map(|v| v.severity)
            .max_by_key(severity_weight)
            .unwrap_or(Severity::Info);
        let aliases: Vec<String> = items.iter().map(|v| v.vuln_id.clone()).collect();
        let canonical_id = aliases
            .iter()
            .find(|id| id.starts_with("CVE-"))
            .cloned()
            .unwrap_or_else(|| aliases.first().cloned().unwrap_or_default());
        out.push(CorrelatedAdvisory {
            canonical_id,
            aliases,
            authoritative_source: authoritative.source.clone(),
            title: authoritative.title.clone(),
            severity,
            max_cvss_v3: max_cvss,
            sources_present: unique_sources,
            records: items,
        });
    }
    let stats = CorrelationStats {
        input_records,
        correlated_clusters: out.len(),
        multi_source_clusters: multi_source,
    };
    (out, stats)
}

fn severity_weight(s: &Severity) -> u8 {
    match s {
        Severity::Critical => 5,
        Severity::High => 4,
        Severity::Medium => 3,
        Severity::Low => 2,
        Severity::Info => 1,
        Severity::Unassigned => 0,
    }
}

#[derive(Default)]
struct UnionFind {
    parent: HashMap<String, String>,
}

impl UnionFind {
    fn insert(&mut self, x: &str) {
        self.parent.entry(x.to_string()).or_insert_with(|| x.to_string());
    }

    fn find(&mut self, x: &str) -> String {
        let mut cur = x.to_string();
        self.parent.entry(cur.clone()).or_insert_with(|| cur.clone());
        loop {
            let p = self.parent.get(&cur).cloned().unwrap_or_else(|| cur.clone());
            if p == cur {
                return cur;
            }
            cur = p;
        }
    }

    fn union(&mut self, a: &str, b: &str) {
        self.insert(a);
        self.insert(b);
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(rb, ra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AnalysisState, Severity, VulnIntel, VulnSource};
    use uuid::Uuid;

    fn mk(id: &str, src: VulnSource, sev: Severity, cvss: Option<f32>) -> VulnIntel {
        let title = format!("{}-{:?}", id, src);
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: id.into(),
            source: src,
            title,
            description: "".into(),
            severity: sev,
            cvss_v3_base: cvss,
            cvss_v3_vector: None,
            cvss_v2_base: None,
            epss_score: None,
            epss_percentile: None,
            cwes: vec![],
            references: vec![],
            affected: vec![],
            published: None,
            modified: None,
            state: AnalysisState::NotSet,
        }
    }

    #[test]
    fn empty_input_yields_empty() {
        let (out, stats) = correlate(vec![], &[]);
        assert!(out.is_empty());
        assert_eq!(stats.input_records, 0);
        assert_eq!(stats.correlated_clusters, 0);
    }

    #[test]
    fn single_advisory_passes_through() {
        let v = mk("CVE-1", VulnSource::Nvd, Severity::High, Some(7.5));
        let (out, _) = correlate(vec![v], &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].canonical_id, "CVE-1");
        assert_eq!(out[0].authoritative_source, VulnSource::Nvd);
        assert_eq!(out[0].sources_present.len(), 1);
    }

    #[test]
    fn nvd_wins_priority_over_osv_for_same_id() {
        let a = mk("CVE-1", VulnSource::Nvd, Severity::Medium, Some(5.0));
        let b = mk("CVE-1", VulnSource::Osv, Severity::High, Some(7.5));
        let (out, _) = correlate(vec![b, a], &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].authoritative_source, VulnSource::Nvd);
    }

    #[test]
    fn max_cvss_is_aggregated_across_sources() {
        let a = mk("CVE-1", VulnSource::Nvd, Severity::Medium, Some(5.0));
        let b = mk("CVE-1", VulnSource::Osv, Severity::High, Some(7.5));
        let (out, _) = correlate(vec![a, b], &[]);
        assert_eq!(out[0].max_cvss_v3, Some(7.5));
    }

    #[test]
    fn severity_is_max_across_sources() {
        let a = mk("CVE-1", VulnSource::Nvd, Severity::Low, None);
        let b = mk("CVE-1", VulnSource::Osv, Severity::Critical, None);
        let (out, _) = correlate(vec![a, b], &[]);
        assert_eq!(out[0].severity, Severity::Critical);
    }

    #[test]
    fn aliases_cluster_cross_source_ids() {
        let a = mk("CVE-2024-1", VulnSource::Nvd, Severity::High, Some(8.0));
        let b = mk("GHSA-x-y-z", VulnSource::Ghsa, Severity::High, Some(7.0));
        let (out, stats) = correlate(
            vec![a, b],
            &[("CVE-2024-1".into(), "GHSA-x-y-z".into())],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].canonical_id, "CVE-2024-1");
        assert!(out[0].aliases.iter().any(|a| a == "GHSA-x-y-z"));
        assert_eq!(stats.multi_source_clusters, 1);
    }

    #[test]
    fn unrelated_ids_remain_separate() {
        let a = mk("CVE-1", VulnSource::Nvd, Severity::Medium, None);
        let b = mk("CVE-2", VulnSource::Osv, Severity::High, None);
        let (out, _) = correlate(vec![a, b], &[]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn canonical_id_prefers_cve_over_ghsa() {
        let a = mk("CVE-2024-2", VulnSource::Nvd, Severity::Low, None);
        let b = mk("GHSA-aaaa", VulnSource::Ghsa, Severity::Low, None);
        let (out, _) = correlate(vec![b, a], &[("GHSA-aaaa".into(), "CVE-2024-2".into())]);
        assert_eq!(out[0].canonical_id, "CVE-2024-2");
    }

    #[test]
    fn stats_count_input_records() {
        let a = mk("CVE-1", VulnSource::Nvd, Severity::Low, None);
        let b = mk("CVE-2", VulnSource::Nvd, Severity::Low, None);
        let (_, stats) = correlate(vec![a, b], &[]);
        assert_eq!(stats.input_records, 2);
        assert_eq!(stats.correlated_clusters, 2);
        assert_eq!(stats.multi_source_clusters, 0);
    }

    #[test]
    fn missing_cvss_does_not_crash() {
        let v = mk("CVE-1", VulnSource::Nvd, Severity::High, None);
        let (out, _) = correlate(vec![v], &[]);
        assert_eq!(out[0].max_cvss_v3, None);
    }

    #[test]
    fn sources_present_dedupes_repeats() {
        let a = mk("CVE-1", VulnSource::Nvd, Severity::High, None);
        let b = mk("CVE-1", VulnSource::Nvd, Severity::High, None);
        let (out, _) = correlate(vec![a, b], &[]);
        assert_eq!(out[0].sources_present.len(), 1);
        assert_eq!(out[0].records.len(), 2);
    }
}
