// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scan-parser registry. Each parser converts a scanner's native
//! output to a `Vec<Finding>` ready for dedup + persistence.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/factory.py
//!         (`PARSERS` registry) — each `tools/<name>/parser.py` exposes
//!         `get_scan_types() / get_findings(handle, test) / get_dedupe_fields()`.

use crate::finding::Finding;

pub mod bandit;
pub mod cyclonedx_vex;
pub mod generic;
pub mod nuclei;
pub mod sarif;
pub mod semgrep;
pub mod snyk;
pub mod trivy;
pub mod zap;

#[derive(Debug, thiserror::Error)]
pub enum ParserError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid XML: {0}")]
    Xml(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("unsupported scan type: {0}")]
    UnsupportedScanType(String),
}

/// One parser converts native scanner output → unified Findings.
pub trait ScanParser: Send + Sync {
    /// Scan-type identifier (matches DefectDojo's `get_scan_types()[0]`).
    fn scan_type(&self) -> &'static str;
    /// Field-set used for hash_code dedup (informational; the
    /// authoritative table lives in `dedup::scanner_fields`).
    fn dedupe_fields(&self) -> &'static [&'static str];
    /// Parse a single scan output. `data` is the raw bytes of the
    /// upload (JSON, XML, etc — parser-specific).
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError>;
}

/// Registry of every parser the binary ships with. Lookup by scan_type.
pub fn registry() -> Vec<Box<dyn ScanParser>> {
    vec![
        Box::new(bandit::BanditParser),
        Box::new(trivy::TrivyParser),
        Box::new(zap::ZapParser),
        Box::new(semgrep::SemgrepParser),
        Box::new(sarif::SarifParser),
        Box::new(snyk::SnykParser),
        Box::new(nuclei::NucleiParser),
        Box::new(generic::GenericParser),
    ]
}

/// Find a parser by scan_type — case-sensitive, matches DefectDojo.
pub fn find_parser(scan_type: &str) -> Option<Box<dyn ScanParser>> {
    registry().into_iter().find(|p| p.scan_type() == scan_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_all_seven_parsers() {
        let names: Vec<_> = registry().iter().map(|p| p.scan_type()).collect();
        for expected in [
            "Bandit Scan",
            "Trivy Scan",
            "ZAP Scan",
            "Semgrep JSON Report",
            "SARIF",
            "Snyk Scan",
            "Nuclei Scan",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn find_parser_by_scan_type() {
        assert!(find_parser("Bandit Scan").is_some());
        assert!(find_parser("Nope").is_none());
    }

    #[test]
    fn registry_includes_generic_universal_importer() {
        assert!(find_parser("Generic Findings Import").is_some());
    }
}
