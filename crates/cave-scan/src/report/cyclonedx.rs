// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: CycloneDX 1.5 spec — cyclonedx.org/specification/overview/

//! CycloneDX 1.5 SBOM serializer.

use super::Report;
use serde_json::{Value, json};

pub fn to_cyclonedx(report: &Report) -> Value {
    let components: Vec<Value> = report
        .packages
        .iter()
        .map(|p| {
            let mut c = json!({
                "type": "library",
                "name": p.name,
                "version": p.version,
            });
            if let Some(lic) = &p.license {
                c["licenses"] = json!([{ "license": { "id": lic } }]);
            }
            if let Some(purl) = &p.purl {
                c["purl"] = json!(purl);
            }
            c
        })
        .collect();

    let vulns: Vec<Value> = report
        .findings
        .iter()
        .filter(|f| f.cve.is_some() || f.id.starts_with("CVE-"))
        .map(|f| {
            json!({
                "id": f.cve.clone().unwrap_or_else(|| f.id.clone()),
                "ratings": [{ "severity": severity_str(f.severity) }],
                "description": f.message,
            })
        })
        .collect();

    json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "tools": [{ "vendor": "Cave Runtime", "name": "cave-scan", "version": env!("CARGO_PKG_VERSION") }],
            "component": { "type": "application", "name": report.target }
        },
        "components": components,
        "vulnerabilities": vulns
    })
}

fn severity_str(s: super::Severity) -> &'static str {
    match s {
        super::Severity::Critical => "critical",
        super::Severity::High => "high",
        super::Severity::Medium => "medium",
        super::Severity::Low => "low",
        super::Severity::Info => "info",
    }
}

pub fn to_string_pretty(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&to_cyclonedx(report))
}
