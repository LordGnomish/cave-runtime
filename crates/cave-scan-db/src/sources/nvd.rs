// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/vulnsrc/nvd/nvd.go
//! NIST NVD (CVE) feed parser.
//!
//! Accepts the upstream NVD JSON 1.1 schema subset — only the fields trivy-db
//! ingests. Each entry has CVE id, descriptions[en], CVSS v3, references.

use crate::{Advisory, CvssV3, DbError, Result, Severity, Vulnerability};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct NvdFeed {
    #[serde(rename = "CVE_Items")]
    pub items: Vec<NvdItem>,
}

#[derive(Debug, Deserialize)]
pub struct NvdItem {
    pub cve: NvdCve,
    #[serde(default)]
    pub impact: Option<NvdImpact>,
}

#[derive(Debug, Deserialize)]
pub struct NvdCve {
    #[serde(rename = "CVE_data_meta")]
    pub meta: NvdMeta,
    pub description: NvdDescriptionWrap,
    #[serde(default)]
    pub references: Option<NvdReferences>,
}

#[derive(Debug, Deserialize)]
pub struct NvdMeta {
    #[serde(rename = "ID")]
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct NvdDescriptionWrap {
    pub description_data: Vec<NvdDescription>,
}

#[derive(Debug, Deserialize)]
pub struct NvdDescription {
    pub lang: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct NvdReferences {
    pub reference_data: Vec<NvdReference>,
}

#[derive(Debug, Deserialize)]
pub struct NvdReference {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct NvdImpact {
    #[serde(rename = "baseMetricV3")]
    pub base_metric_v3: Option<NvdBaseV3>,
}

#[derive(Debug, Deserialize)]
pub struct NvdBaseV3 {
    #[serde(rename = "cvssV3")]
    pub cvss: NvdCvss,
}

#[derive(Debug, Deserialize)]
pub struct NvdCvss {
    #[serde(rename = "vectorString")]
    pub vector_string: String,
    #[serde(rename = "baseScore")]
    pub base_score: f32,
    #[serde(rename = "baseSeverity", default)]
    pub base_severity: String,
}

/// Parse one NVD feed file → list of Vulnerabilities.
///
/// NVD doesn't carry per-package advisories — only CVE metadata. Advisories
/// come from OS / lang sources that reference the CVE id.
pub fn parse(bytes: &[u8]) -> Result<Vec<Vulnerability>> {
    let feed: NvdFeed =
        serde_json::from_slice(bytes).map_err(|e| DbError::InvalidFeed(e.to_string()))?;
    let mut out = Vec::with_capacity(feed.items.len());
    for it in feed.items {
        let desc = it
            .cve
            .description
            .description_data
            .into_iter()
            .find(|d| d.lang == "en")
            .map(|d| d.value)
            .unwrap_or_default();
        let cvss = it.impact.and_then(|i| i.base_metric_v3).map(|b| CvssV3 {
            vector: b.cvss.vector_string,
            score: b.cvss.base_score,
        });
        let sev = cvss
            .as_ref()
            .map(score_to_severity)
            .unwrap_or(Severity::Unknown);
        let refs = it
            .cve
            .references
            .map(|r| r.reference_data.into_iter().map(|x| x.url).collect())
            .unwrap_or_default();
        out.push(Vulnerability {
            id: it.cve.meta.id,
            title: String::new(),
            description: desc,
            severity: sev,
            cwe_ids: Vec::new(),
            references: refs,
            cvss_v3: cvss,
            published_date: None,
            last_modified_date: None,
        });
    }
    Ok(out)
}

/// CVSS v3 → Severity (NVD bucket boundaries from spec §5.1).
pub fn score_to_severity(c: &CvssV3) -> Severity {
    let s = c.score;
    if s >= 9.0 {
        Severity::Critical
    } else if s >= 7.0 {
        Severity::High
    } else if s >= 4.0 {
        Severity::Medium
    } else if s > 0.0 {
        Severity::Low
    } else {
        Severity::Unknown
    }
}

/// Empty placeholder — NVD has no per-package advisories.
pub fn advisories_for(_v: &Vulnerability) -> Vec<Advisory> {
    Vec::new()
}
