// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/epss/EpssParser.java
//   first.org/epss/data_stats (spec reference)
//
//! Exploit Prediction Scoring System (EPSS) join. EPSS publishes a daily
//! `epss_scores-YYYY-MM-DD.csv.gz` file (first.org). The CSV columns are
//! `cve, epss, percentile`. This module parses the CSV and joins by CVE.

use crate::models::VulnIntel;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EpssError {
    #[error("malformed line {0}: {1}")]
    BadLine(usize, String),
    #[error("epss score out of range: {0}")]
    OutOfRange(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpssScore {
    pub cve_id: String,
    pub score: f32,
    pub percentile: f32,
}

/// Parse the EPSS CSV body (decompressed). The first two lines may be
/// `#model_version`, `cve,epss,percentile` headers — they are skipped.
pub fn parse_csv(input: &str) -> Result<Vec<EpssScore>, EpssError> {
    let mut out = Vec::new();
    for (i, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("cve,") {
            // Header.
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 3 {
            return Err(EpssError::BadLine(i + 1, line.into()));
        }
        let score: f32 = parts[1]
            .parse()
            .map_err(|_| EpssError::BadLine(i + 1, parts[1].into()))?;
        let percentile: f32 = parts[2]
            .parse()
            .map_err(|_| EpssError::BadLine(i + 1, parts[2].into()))?;
        if !(0.0..=1.0).contains(&score) {
            return Err(EpssError::OutOfRange(score));
        }
        out.push(EpssScore {
            cve_id: parts[0].to_string(),
            score,
            percentile,
        });
    }
    Ok(out)
}

/// Build an index keyed by `cve_id` for O(1) joins.
pub fn build_index(scores: Vec<EpssScore>) -> HashMap<String, EpssScore> {
    let mut idx = HashMap::with_capacity(scores.len());
    for s in scores {
        idx.insert(s.cve_id.clone(), s);
    }
    idx
}

/// Join EPSS scores onto a slice of `VulnIntel` in place. Returns count joined.
pub fn join_in_place(intels: &mut [VulnIntel], idx: &HashMap<String, EpssScore>) -> usize {
    let mut hits = 0;
    for v in intels.iter_mut() {
        if let Some(s) = idx.get(&v.vuln_id) {
            v.epss_score = Some(s.score);
            v.epss_percentile = Some(s.percentile);
            hits += 1;
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AnalysisState, Severity, VulnSource};
    use uuid::Uuid;

    fn intel(id: &str) -> VulnIntel {
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: id.into(),
            source: VulnSource::Nvd,
            title: "".into(),
            description: "".into(),
            severity: Severity::Medium,
            cvss_v3_base: Some(5.0),
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

    const SAMPLE: &str = "#model_version:v2024.01.01\ncve,epss,percentile\nCVE-2024-1,0.95,0.99\nCVE-2024-2,0.001,0.10\n";

    #[test]
    fn parse_skips_headers_and_collects() {
        let v = parse_csv(SAMPLE).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].cve_id, "CVE-2024-1");
        assert!((v[0].score - 0.95).abs() < 1e-6);
    }

    #[test]
    fn parse_rejects_out_of_range_score() {
        let bad = "CVE-X,1.5,0.5\n";
        assert!(matches!(parse_csv(bad), Err(EpssError::OutOfRange(_))));
    }

    #[test]
    fn parse_rejects_malformed_line() {
        let bad = "CVE-Y,abc\n";
        assert!(matches!(parse_csv(bad), Err(EpssError::BadLine(_, _))));
    }

    #[test]
    fn join_in_place_fills_score_and_percentile() {
        let idx = build_index(parse_csv(SAMPLE).unwrap());
        let mut v = vec![intel("CVE-2024-1"), intel("CVE-2024-2"), intel("CVE-MISS")];
        let hits = join_in_place(&mut v, &idx);
        assert_eq!(hits, 2);
        assert_eq!(v[0].epss_score, Some(0.95));
        assert_eq!(v[0].epss_percentile, Some(0.99));
        assert!(v[2].epss_score.is_none());
    }
}
