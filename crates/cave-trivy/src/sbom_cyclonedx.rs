// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CycloneDX 1.5 SBOM emitter.
//!
//! Mirrors trivy's `pkg/sbom/cyclonedx`. Given a list of detected
//! `Package`s, emits a CycloneDX JSON document with one `component` per
//! package and a top-level metadata.component for the scan target.

use crate::error::{TrivyError, TrivyResult};
use crate::models::{Package, Report};
use crate::purl::{ecosystem_to_purl_type, PackageUrl};
use serde_json::{json, Value};

pub const SPEC_VERSION: &str = "1.5";

pub fn emit_from_packages(target: &str, pkgs: &[Package]) -> TrivyResult<String> {
    let mut comps = Vec::new();
    for p in pkgs {
        comps.push(component_for(p));
    }
    let doc = json!({
        "bomFormat": "CycloneDX",
        "specVersion": SPEC_VERSION,
        "serialNumber": format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        "version": 1,
        "metadata": {
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "tools": [{ "name": "cave-trivy", "version": crate::UPSTREAM_VERSION }],
            "component": { "type": "container", "name": target, "version": "" }
        },
        "components": comps,
    });
    serde_json::to_string_pretty(&doc).map_err(|e| TrivyError::Sbom(format!("cyclonedx: {}", e)))
}

pub fn emit_from_report(report: &Report) -> TrivyResult<String> {
    let mut comps = Vec::new();
    for r in &report.results {
        for v in &r.vulnerabilities {
            comps.push(json!({
                "type": "library",
                "name": v.pkg_name,
                "version": v.installed_version,
            }));
        }
    }
    emit_from_packages(
        &report.artifact_name,
        &comps
            .iter()
            .filter_map(|c| {
                let name = c.get("name")?.as_str()?.to_string();
                let ver = c.get("version")?.as_str()?.to_string();
                Some(Package::new(&name, &ver, "generic"))
            })
            .collect::<Vec<_>>(),
    )
}

fn component_for(p: &Package) -> Value {
    let pty = ecosystem_to_purl_type(&p.ecosystem);
    let mut purl = PackageUrl::new(pty, &p.name, Some(&p.version));
    if matches!(pty, "apk" | "deb" | "rpm") {
        purl = purl.with_namespace(&p.ecosystem);
    }
    json!({
        "type": "library",
        "bom-ref": format!("pkg:{}/{}@{}", pty, p.name, p.version),
        "name": p.name,
        "version": p.version,
        "purl": purl.to_string_canonical(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_components() {
        let pkgs = vec![
            Package::new("openssl", "3.0.0", "alpine"),
            Package::new("lodash", "4.17.20", "npm"),
        ];
        let s = emit_from_packages("alpine:3.19", &pkgs).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["bomFormat"], "CycloneDX");
        assert_eq!(v["specVersion"], SPEC_VERSION);
        let comps = v["components"].as_array().unwrap();
        assert_eq!(comps.len(), 2);
        assert!(comps[0]["purl"]
            .as_str()
            .unwrap()
            .starts_with("pkg:apk/alpine/openssl@"));
        assert!(comps[1]["purl"]
            .as_str()
            .unwrap()
            .starts_with("pkg:npm/lodash@"));
    }

    #[test]
    fn metadata_present() {
        let s = emit_from_packages("x", &[]).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["metadata"]["component"]["name"], "x");
        assert!(v["metadata"]["timestamp"].as_str().is_some());
        assert!(v["serialNumber"]
            .as_str()
            .unwrap()
            .starts_with("urn:uuid:"));
    }

    #[test]
    fn empty_emits_zero_components() {
        let s = emit_from_packages("x", &[]).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["components"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn round_trip_detect() {
        let s = emit_from_packages("x", &[Package::new("p", "1", "npm")]).unwrap();
        let fmt = crate::scan_sbom::detect_format(&s);
        assert_eq!(fmt, Some(crate::scan_sbom::SbomFormat::CycloneDx));
    }
}
