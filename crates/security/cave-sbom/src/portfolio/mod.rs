// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/PortfolioMetrics.java
//   src/main/java/org/dependencytrack/model/ProjectMetrics.java
//   src/main/java/org/dependencytrack/tasks/metrics/PortfolioMetricsUpdateTask.java
//
//! Portfolio metrics — per-project risk inheritance + time-series snapshots.

use crate::components::ComponentRecord;
use crate::models::{Severity, VulnIntel};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Per-project risk roll-up. Mirrors `org.dependencytrack.model.ProjectMetrics`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProjectRisk {
    pub project_uuid: Uuid,
    pub total_components: u32,
    pub vulnerable_components: u32,
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub info: u32,
    pub unassigned: u32,
    /// Mirror of `InheritedRiskScore` upstream — sum across components.
    /// Weights: critical=10, high=5, medium=3, low=1, unassigned=5.
    pub inherited_risk_score: f64,
}

impl ProjectRisk {
    pub fn compute(
        project_uuid: Uuid,
        components: &[ComponentRecord],
        vulns: &[VulnIntel],
    ) -> Self {
        let mut out = ProjectRisk {
            project_uuid,
            ..Default::default()
        };
        let mut vulnerable_set: HashSet<Uuid> = HashSet::new();
        // Pre-index vulns by lowercase component-name for O(1) lookup.
        let mut by_name: HashMap<String, Vec<&VulnIntel>> = HashMap::new();
        for v in vulns {
            for a in &v.affected {
                by_name
                    .entry(a.name.to_ascii_lowercase())
                    .or_default()
                    .push(v);
            }
        }
        for c in components.iter().filter(|c| c.project_uuid == project_uuid) {
            out.total_components += 1;
            if let Some(matches) = by_name.get(&c.name.to_ascii_lowercase()) {
                if !matches.is_empty() {
                    vulnerable_set.insert(c.uuid);
                }
                for v in matches {
                    match v.severity {
                        Severity::Critical => out.critical += 1,
                        Severity::High => out.high += 1,
                        Severity::Medium => out.medium += 1,
                        Severity::Low => out.low += 1,
                        Severity::Info => out.info += 1,
                        Severity::Unassigned => out.unassigned += 1,
                    }
                }
            }
        }
        out.vulnerable_components = vulnerable_set.len() as u32;
        out.inherited_risk_score = (out.critical as f64 * 10.0)
            + (out.high as f64 * 5.0)
            + (out.medium as f64 * 3.0)
            + (out.low as f64 * 1.0)
            + (out.unassigned as f64 * 5.0);
        out
    }
}

/// Snapshot — captures a `ProjectRisk` at a point in time. Mirrors
/// `PortfolioMetricsUpdateTask` daily snapshot append.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortfolioSnapshot {
    pub taken_at: DateTime<Utc>,
    pub per_project: Vec<ProjectRisk>,
}

impl PortfolioSnapshot {
    pub fn take(
        project_uuids: &[Uuid],
        components: &[ComponentRecord],
        vulns: &[VulnIntel],
        now: DateTime<Utc>,
    ) -> Self {
        let per_project = project_uuids
            .iter()
            .map(|p| ProjectRisk::compute(*p, components, vulns))
            .collect();
        Self {
            taken_at: now,
            per_project,
        }
    }

    pub fn total_vulnerable(&self) -> u32 {
        self.per_project
            .iter()
            .map(|p| p.vulnerable_components)
            .sum()
    }

    pub fn total_critical(&self) -> u32 {
        self.per_project.iter().map(|p| p.critical).sum()
    }
}

/// Trend buckets — given a chronological series of snapshots, return the
/// (timestamp, vulnerable_component_count) pairs.
pub fn vulnerable_trend(series: &[PortfolioSnapshot]) -> Vec<(DateTime<Utc>, u32)> {
    series
        .iter()
        .map(|s| (s.taken_at, s.total_vulnerable()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AffectedRange, AnalysisState, VulnSource};
    use chrono::Duration;

    fn mk_comp(p: Uuid, name: &str) -> ComponentRecord {
        let mut c = ComponentRecord::new(p, name, "1.0.0");
        c.purl = Some(format!("pkg:npm/{}@1.0.0", name));
        c
    }

    fn mk_vuln(name: &str, sev: Severity) -> VulnIntel {
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: format!("CVE-{}", name),
            source: VulnSource::Nvd,
            title: "".into(),
            description: "".into(),
            severity: sev,
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
                name: name.into(),
                vers: "*".into(),
                fixed: None,
            }],
            published: None,
            modified: None,
            state: AnalysisState::NotSet,
        }
    }

    #[test]
    fn project_risk_counts_components() {
        let p = Uuid::new_v4();
        let recs = vec![mk_comp(p, "a"), mk_comp(p, "b"), mk_comp(p, "c")];
        let r = ProjectRisk::compute(p, &recs, &[]);
        assert_eq!(r.total_components, 3);
        assert_eq!(r.vulnerable_components, 0);
        assert_eq!(r.inherited_risk_score, 0.0);
    }

    #[test]
    fn project_risk_buckets_by_severity() {
        let p = Uuid::new_v4();
        let recs = vec![
            mk_comp(p, "openssl"),
            mk_comp(p, "lodash"),
            mk_comp(p, "safe"),
        ];
        let vulns = vec![
            mk_vuln("openssl", Severity::Critical),
            mk_vuln("lodash", Severity::High),
        ];
        let r = ProjectRisk::compute(p, &recs, &vulns);
        assert_eq!(r.vulnerable_components, 2);
        assert_eq!(r.critical, 1);
        assert_eq!(r.high, 1);
    }

    #[test]
    fn inherited_risk_score_weighting() {
        let p = Uuid::new_v4();
        let recs = vec![mk_comp(p, "openssl"), mk_comp(p, "lodash")];
        let vulns = vec![
            mk_vuln("openssl", Severity::Critical),
            mk_vuln("lodash", Severity::Medium),
        ];
        let r = ProjectRisk::compute(p, &recs, &vulns);
        assert_eq!(r.inherited_risk_score, 13.0); // critical*10 + medium*3
    }

    #[test]
    fn project_risk_isolates_by_project() {
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        let recs = vec![mk_comp(p1, "openssl"), mk_comp(p2, "lodash")];
        let vulns = vec![
            mk_vuln("openssl", Severity::Critical),
            mk_vuln("lodash", Severity::High),
        ];
        let r1 = ProjectRisk::compute(p1, &recs, &vulns);
        assert_eq!(r1.total_components, 1);
        assert_eq!(r1.critical, 1);
        assert_eq!(r1.high, 0);
    }

    #[test]
    fn portfolio_snapshot_aggregates() {
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        let recs = vec![mk_comp(p1, "openssl"), mk_comp(p2, "lodash")];
        let vulns = vec![
            mk_vuln("openssl", Severity::Critical),
            mk_vuln("lodash", Severity::High),
        ];
        let s = PortfolioSnapshot::take(&[p1, p2], &recs, &vulns, Utc::now());
        assert_eq!(s.total_vulnerable(), 2);
        assert_eq!(s.total_critical(), 1);
    }

    #[test]
    fn vulnerable_trend_pairs_timestamps_to_counts() {
        let now = Utc::now();
        let s1 = PortfolioSnapshot {
            taken_at: now - Duration::days(7),
            per_project: vec![],
        };
        let s2 = PortfolioSnapshot {
            taken_at: now,
            per_project: vec![ProjectRisk {
                project_uuid: Uuid::new_v4(),
                vulnerable_components: 5,
                ..Default::default()
            }],
        };
        let trend = vulnerable_trend(&[s1, s2]);
        assert_eq!(trend.len(), 2);
        assert_eq!(trend[0].1, 0);
        assert_eq!(trend[1].1, 5);
    }
}
