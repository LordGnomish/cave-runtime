// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! EPSS (Exploit Prediction Scoring System) CSV parser + join.
//! Mirrors `parser.epss.EpssParser` + `tasks.EpssMirrorTask`.

use crate::error::{Error, Result};
use crate::models::Vulnerability;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct EpssEntry {
    pub cve: String,
    pub epss: f64,
    pub percentile: f64,
}

pub fn parse_epss_csv(input: &str) -> Result<Vec<EpssEntry>> {
    let mut out = Vec::new();
    for (i, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("cve,") {
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 3 {
            return Err(Error::Parse(format!(
                "epss line {}: expected 3 cols, got {}",
                i + 1,
                parts.len()
            )));
        }
        let cve = parts[0].trim().to_string();
        let epss = parts[1]
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::Parse(format!("epss line {}: bad score", i + 1)))?;
        let perc = parts[2]
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::Parse(format!("epss line {}: bad percentile", i + 1)))?;
        if !(0.0..=1.0).contains(&epss) || !(0.0..=1.0).contains(&perc) {
            return Err(Error::Parse(format!(
                "epss line {}: score/percentile out of 0..=1 range",
                i + 1
            )));
        }
        out.push(EpssEntry {
            cve,
            epss,
            percentile: perc,
        });
    }
    Ok(out)
}

/// Join EPSS scores into vulnerabilities in-place (by `vuln_id` == CVE).
pub fn join_epss(vulns: &mut [Vulnerability], epss: &[EpssEntry]) -> usize {
    let map: HashMap<&str, &EpssEntry> = epss.iter().map(|e| (e.cve.as_str(), e)).collect();
    let mut joined = 0;
    for v in vulns.iter_mut() {
        if let Some(e) = map.get(v.vuln_id.as_str()) {
            v.epss_score = Some(e.epss);
            v.epss_percentile = Some(e.percentile);
            joined += 1;
        }
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VulnSource;

    const CSV: &str = "\
#model_version:v2024.04.25,score_date:2026-04-25T00:00:00+0000
cve,epss,percentile
CVE-2026-1,0.12345,0.78901
CVE-2026-2,0.00100,0.05000
";

    #[test]
    fn parses_csv_with_header() {
        let e = parse_epss_csv(CSV).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].cve, "CVE-2026-1");
        assert!((e[0].epss - 0.12345).abs() < 1e-9);
    }

    #[test]
    fn rejects_out_of_range() {
        let csv = "cve,epss,percentile\nCVE-X,1.5,0.5\n";
        assert!(matches!(parse_epss_csv(csv), Err(Error::Parse(_))));
    }

    #[test]
    fn rejects_short_row() {
        let csv = "cve,epss\nCVE-X,0.5\n";
        assert!(matches!(parse_epss_csv(csv), Err(Error::Parse(_))));
    }

    #[test]
    fn join_merges_into_vulns() {
        let mut v = vec![
            Vulnerability::new("CVE-2026-1", VulnSource::Nvd),
            Vulnerability::new("CVE-2026-X", VulnSource::Nvd),
        ];
        let e = parse_epss_csv(CSV).unwrap();
        let joined = join_epss(&mut v, &e);
        assert_eq!(joined, 1);
        assert!(v[0].epss_score.is_some());
        assert!(v[1].epss_score.is_none());
    }

    #[test]
    fn empty_csv_ok() {
        assert!(parse_epss_csv("").unwrap().is_empty());
    }
}
