// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/vulnsrc/alpine/alpine.go
//! Alpine secdb feed parser.
//!
//! Alpine's `secdb` JSON shape:
//! ```json
//! {
//!   "distroversion": "v3.19",
//!   "packages": [
//!     { "pkg": { "name": "openssl", "secfixes": { "1.1.1q-r0": ["CVE-2022-2274"] } } }
//!   ]
//! }
//! ```

use crate::{Advisory, DbError, Result, Severity};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct AlpineSecdb {
    pub distroversion: String,
    pub packages: Vec<AlpinePkgWrap>,
}

#[derive(Debug, Deserialize)]
pub struct AlpinePkgWrap {
    pub pkg: AlpinePkg,
}

#[derive(Debug, Deserialize)]
pub struct AlpinePkg {
    pub name: String,
    #[serde(default)]
    pub secfixes: BTreeMap<String, Vec<String>>,
}

/// Parse one Alpine secdb → flat advisory list.
pub fn parse(bytes: &[u8]) -> Result<Vec<Advisory>> {
    let s: AlpineSecdb =
        serde_json::from_slice(bytes).map_err(|e| DbError::InvalidFeed(e.to_string()))?;
    let eco = format!("alpine:{}", s.distroversion.trim_start_matches('v'));
    let mut out = Vec::new();
    for w in s.packages {
        for (fix, cves) in w.pkg.secfixes {
            for cve in cves {
                out.push(Advisory {
                    vulnerability_id: cve,
                    package_name: w.pkg.name.clone(),
                    ecosystem: eco.clone(),
                    fixed_version: fix.clone(),
                    affected_version: String::new(),
                    severity: Severity::Unknown,
                    data_source: "alpine".into(),
                });
            }
        }
    }
    Ok(out)
}
