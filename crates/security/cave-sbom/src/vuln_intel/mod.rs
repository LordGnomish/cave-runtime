// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/{nvd,osv,github,snyk,epss}
//
//! Vulnerability intelligence — multi-source advisory ingestion.

pub mod correlator;
pub mod epss;
pub mod ghsa;
pub mod nvd;
pub mod osv;
pub mod snyk;

use crate::models::VulnIntel;

/// Deduplicating merge: when multiple sources publish advisories under the
/// same `vuln_id`, prefer the one with the highest CVSS v3 base.
/// Mirrors `org.dependencytrack.tasks.IntegrityAnalysisTask` merge logic.
pub fn merge_advisories(inputs: Vec<VulnIntel>) -> Vec<VulnIntel> {
    use std::collections::BTreeMap;
    let mut by_id: BTreeMap<String, VulnIntel> = BTreeMap::new();
    for v in inputs {
        match by_id.get(&v.vuln_id) {
            Some(existing) => {
                let cur = existing.cvss_v3_base.unwrap_or(0.0);
                let new = v.cvss_v3_base.unwrap_or(0.0);
                if new > cur {
                    by_id.insert(v.vuln_id.clone(), v);
                }
            }
            None => {
                by_id.insert(v.vuln_id.clone(), v);
            }
        }
    }
    by_id.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AnalysisState, Severity, VulnSource};
    use uuid::Uuid;

    fn make(id: &str, src: VulnSource, base: Option<f32>) -> VulnIntel {
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: id.into(),
            source: src,
            title: id.into(),
            description: "".into(),
            severity: Severity::Medium,
            cvss_v3_base: base,
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
    fn merge_keeps_highest_cvss() {
        let merged = merge_advisories(vec![
            make("CVE-1", VulnSource::Nvd, Some(5.0)),
            make("CVE-1", VulnSource::Osv, Some(7.5)),
            make("CVE-1", VulnSource::Ghsa, Some(6.0)),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, VulnSource::Osv);
        assert_eq!(merged[0].cvss_v3_base, Some(7.5));
    }

    #[test]
    fn merge_independent_ids_preserved() {
        let merged = merge_advisories(vec![
            make("CVE-1", VulnSource::Nvd, Some(5.0)),
            make("CVE-2", VulnSource::Osv, Some(7.5)),
        ]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_missing_cvss_loses_to_present() {
        let merged = merge_advisories(vec![
            make("CVE-1", VulnSource::Nvd, None),
            make("CVE-1", VulnSource::Osv, Some(1.0)),
        ]);
        assert_eq!(merged[0].source, VulnSource::Osv);
    }
}
