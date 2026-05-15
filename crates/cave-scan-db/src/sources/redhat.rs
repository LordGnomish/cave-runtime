// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/vulnsrc/redhat/redhat.go
//! Red Hat Security Data API feed parser.
//!
//! Schema modelled on Red Hat's per-CVE JSON. Each record carries an array of
//! `affected_release` + `package_state` entries — we promote each to one
//! Advisory, keyed on `cpe`.

use crate::{Advisory, DbError, Result, Severity};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RedhatCve {
    pub name: String,
    #[serde(default)]
    pub threat_severity: String,
    #[serde(default)]
    pub affected_release: Vec<AffectedRelease>,
    #[serde(default)]
    pub package_state: Vec<PackageState>,
}

#[derive(Debug, Deserialize)]
pub struct AffectedRelease {
    pub product_name: String,
    pub package: String,
    #[serde(default)]
    pub cpe: String,
    #[serde(default)]
    pub release_date: String,
}

#[derive(Debug, Deserialize)]
pub struct PackageState {
    pub product_name: String,
    pub package_name: String,
    pub fix_state: String,
    #[serde(default)]
    pub cpe: String,
}

/// Parse one Red Hat CVE record → advisory list.
pub fn parse(bytes: &[u8]) -> Result<Vec<Advisory>> {
    let r: RedhatCve =
        serde_json::from_slice(bytes).map_err(|e| DbError::InvalidFeed(e.to_string()))?;
    let sev = Severity::parse(&r.threat_severity);
    let mut out = Vec::new();
    for rel in r.affected_release {
        let (pkg_name, fixed) = split_nevra(&rel.package);
        out.push(Advisory {
            vulnerability_id: r.name.clone(),
            package_name: pkg_name,
            ecosystem: cpe_to_ecosystem(&rel.cpe),
            fixed_version: fixed,
            affected_version: String::new(),
            severity: sev,
            data_source: "redhat".into(),
        });
    }
    for st in r.package_state {
        // Track unfixed/won't-fix packages too — affected_version="*".
        out.push(Advisory {
            vulnerability_id: r.name.clone(),
            package_name: st.package_name,
            ecosystem: cpe_to_ecosystem(&st.cpe),
            fixed_version: String::new(),
            affected_version: "*".into(),
            severity: sev,
            data_source: "redhat".into(),
        });
    }
    Ok(out)
}

/// Split an NEVRA-ish package string like `openssl-1.1.1k-7.el8_6` into
/// name + version. Heuristic: split at the first `-<digit>` boundary.
fn split_nevra(s: &str) -> (String, String) {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            return (s[..i].to_string(), s[i + 1..].to_string());
        }
    }
    (s.to_string(), String::new())
}

/// `cpe:/o:redhat:enterprise_linux:8` → `redhat:8`.
fn cpe_to_ecosystem(cpe: &str) -> String {
    let parts: Vec<&str> = cpe.split(':').collect();
    if parts.len() >= 5 && parts[1] == "/o" || parts.first().copied() == Some("cpe") {
        if let Some(ver) = parts.last() {
            return format!("redhat:{ver}");
        }
    }
    "redhat".to_string()
}
