// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! VEX (Vulnerability Exploitability eXchange) — OpenVEX 0.2.0 subset.
//!
//! Mirrors trivy's `pkg/vex`. cave-trivy parses an OpenVEX document and
//! returns a `VexIndex` that the report writer consults: any statement
//! mapping (product, vuln) → `not_affected` or `fixed` suppresses the
//! finding; `affected`/`under_investigation` keep it but flag the
//! justification.

use crate::error::{TrivyError, TrivyResult};
use crate::models::{ScanResult, Vulnerability};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VexStatus {
    NotAffected,
    Affected,
    Fixed,
    UnderInvestigation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VexStatement {
    pub vulnerability: String,
    pub products: Vec<String>,
    pub status: VexStatus,
    #[serde(default)]
    pub justification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VexDocument {
    #[serde(rename = "@context", default)]
    pub context: String,
    pub statements: Vec<VexStatement>,
}

impl VexDocument {
    pub fn parse(json: &str) -> TrivyResult<Self> {
        serde_json::from_str(json).map_err(|e| TrivyError::parse(format!("vex: {}", e)))
    }
}

#[derive(Debug, Clone, Default)]
pub struct VexIndex {
    map: HashMap<(String, String), VexStatus>,
}

impl VexIndex {
    pub fn from_document(doc: &VexDocument) -> Self {
        let mut m = HashMap::new();
        for s in &doc.statements {
            for p in &s.products {
                m.insert((p.clone(), s.vulnerability.clone()), s.status);
            }
        }
        Self { map: m }
    }

    pub fn lookup(&self, product: &str, vuln: &str) -> Option<VexStatus> {
        self.map
            .get(&(product.to_string(), vuln.to_string()))
            .copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Apply a VEX index to a `ScanResult`, returning the suppressed count.
/// Vulnerabilities flagged `NotAffected` or `Fixed` are removed.
pub fn apply(index: &VexIndex, product: &str, result: &mut ScanResult) -> usize {
    if index.is_empty() {
        return 0;
    }
    let before = result.vulnerabilities.len();
    result.vulnerabilities.retain(|v: &Vulnerability| {
        match index.lookup(product, &v.id) {
            Some(VexStatus::NotAffected) | Some(VexStatus::Fixed) => false,
            _ => true,
        }
    });
    before - result.vulnerabilities.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> VexDocument {
        VexDocument {
            context: "https://openvex.dev/ns/v0.2.0".into(),
            statements: vec![
                VexStatement {
                    vulnerability: "CVE-2026-0001".into(),
                    products: vec!["pkg:oci/cave/runtime".into()],
                    status: VexStatus::NotAffected,
                    justification: Some("vulnerable_code_not_in_execute_path".into()),
                },
                VexStatement {
                    vulnerability: "CVE-2026-0010".into(),
                    products: vec!["pkg:oci/cave/runtime".into()],
                    status: VexStatus::Affected,
                    justification: None,
                },
                VexStatement {
                    vulnerability: "CVE-2026-0030".into(),
                    products: vec!["pkg:oci/cave/runtime".into()],
                    status: VexStatus::Fixed,
                    justification: None,
                },
            ],
        }
    }

    #[test]
    fn parses_document() {
        let s = serde_json::to_string(&doc()).unwrap();
        let back = VexDocument::parse(&s).unwrap();
        assert_eq!(back.statements.len(), 3);
    }

    #[test]
    fn parse_bad_json() {
        assert!(VexDocument::parse("not json").is_err());
    }

    #[test]
    fn index_lookup() {
        let idx = VexIndex::from_document(&doc());
        assert_eq!(idx.len(), 3);
        assert_eq!(
            idx.lookup("pkg:oci/cave/runtime", "CVE-2026-0001"),
            Some(VexStatus::NotAffected)
        );
        assert!(idx.lookup("pkg:oci/other", "CVE-2026-0001").is_none());
    }

    #[test]
    fn apply_suppresses_not_affected_and_fixed() {
        let idx = VexIndex::from_document(&doc());
        let mut sr = ScanResult::default();
        sr.vulnerabilities.push(Vulnerability::new(
            "CVE-2026-0001",
            "x",
            "1",
            crate::severity::Severity::Critical,
        ));
        sr.vulnerabilities.push(Vulnerability::new(
            "CVE-2026-0010",
            "x",
            "1",
            crate::severity::Severity::High,
        ));
        sr.vulnerabilities.push(Vulnerability::new(
            "CVE-2026-0030",
            "x",
            "1",
            crate::severity::Severity::Medium,
        ));
        let n = apply(&idx, "pkg:oci/cave/runtime", &mut sr);
        assert_eq!(n, 2);
        assert_eq!(sr.vulnerabilities.len(), 1);
        assert_eq!(sr.vulnerabilities[0].id, "CVE-2026-0010");
    }

    #[test]
    fn empty_index_noop() {
        let idx = VexIndex::default();
        let mut sr = ScanResult::default();
        sr.vulnerabilities.push(Vulnerability::new(
            "CVE-X",
            "p",
            "1",
            crate::severity::Severity::Low,
        ));
        assert_eq!(apply(&idx, "pkg", &mut sr), 0);
    }
}
