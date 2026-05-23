// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! SBOM scanner — ingest a CycloneDX or SPDX document and correlate against
//! the offline vuln DB.
//!
//! Mirrors trivy's `pkg/scan/artifact/sbom` for the two document shapes
//! cave-trivy emits: CycloneDX 1.5 JSON and SPDX 2.3 JSON. The ingest
//! path normalises components to `Package { ecosystem, name, version,
//! purl }` and feeds them through `scan_image::correlate`.

use crate::error::{TrivyError, TrivyResult};
use crate::models::{Package, Report, ScanResult};
use crate::scan_image::correlate;
use crate::vulndb::VulnDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbomFormat {
    CycloneDx,
    Spdx,
}

pub fn detect_format(text: &str) -> Option<SbomFormat> {
    let head = text
        .chars()
        .take(2048)
        .collect::<String>()
        .to_ascii_lowercase();
    if head.contains("\"bomformat\"") || head.contains("\"cyclonedx\"") {
        return Some(SbomFormat::CycloneDx);
    }
    if head.contains("\"spdxversion\"") || head.contains("\"spdxid\"") {
        return Some(SbomFormat::Spdx);
    }
    None
}

pub fn ingest_cyclonedx(text: &str) -> TrivyResult<Vec<Package>> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| TrivyError::parse(format!("cyclonedx: {}", e)))?;
    let comps = v.get("components").and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for c in &comps {
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let ver = c.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let purl = c.get("purl").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() || ver.is_empty() {
            continue;
        }
        let eco = ecosystem_from_purl(purl);
        let mut p = Package::new(name, ver, &eco);
        if !purl.is_empty() {
            p.purl = Some(purl.to_string());
        }
        out.push(p);
    }
    Ok(out)
}

pub fn ingest_spdx(text: &str) -> TrivyResult<Vec<Package>> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| TrivyError::parse(format!("spdx: {}", e)))?;
    let packages = v.get("packages").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for c in &packages {
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let ver = c.get("versionInfo").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() || ver.is_empty() {
            continue;
        }
        let purl = c
            .get("externalRefs")
            .and_then(|r| r.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|x| {
                    if x.get("referenceType")?.as_str()? == "purl" {
                        x.get("referenceLocator")?.as_str().map(String::from)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();
        let eco = ecosystem_from_purl(&purl);
        let mut p = Package::new(name, ver, &eco);
        if !purl.is_empty() {
            p.purl = Some(purl);
        }
        out.push(p);
    }
    Ok(out)
}

pub fn ecosystem_from_purl(purl: &str) -> String {
    let t = match purl.strip_prefix("pkg:").and_then(|s| s.split('/').next()) {
        Some(t) => t,
        None => return "generic".into(),
    };
    match t {
        "npm" => "npm",
        "pypi" => "pypi",
        "gem" => "gem",
        "golang" => "go",
        "cargo" => "cargo",
        "composer" => "composer",
        "maven" => "maven",
        "pub" => "pub",
        "hex" => "hex",
        "swift" => "swift",
        "apk" => "alpine",
        "deb" => "debian",
        "rpm" => "rhel",
        _ => "generic",
    }
    .into()
}

pub fn scan_sbom(name: &str, text: &str, db: &VulnDb) -> TrivyResult<Report> {
    let fmt = detect_format(text)
        .ok_or_else(|| TrivyError::Sbom("unknown sbom format".into()))?;
    let pkgs = match fmt {
        SbomFormat::CycloneDx => ingest_cyclonedx(text)?,
        SbomFormat::Spdx => ingest_spdx(text)?,
    };
    let mut report = Report::new(name, "sbom");
    let mut r = ScanResult {
        target: name.into(),
        class: "sbom-pkgs".into(),
        ..Default::default()
    };
    correlate(db, &pkgs, &mut r.vulnerabilities);
    report.results.push(r);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cyclonedx() {
        let s = r#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[]}"#;
        assert_eq!(detect_format(s), Some(SbomFormat::CycloneDx));
    }

    #[test]
    fn detects_spdx() {
        let s = r#"{"spdxVersion":"SPDX-2.3","packages":[]}"#;
        assert_eq!(detect_format(s), Some(SbomFormat::Spdx));
    }

    #[test]
    fn detect_unknown() {
        assert!(detect_format("{}").is_none());
    }

    #[test]
    fn ingest_cyclonedx_simple() {
        let s = r#"{"bomFormat":"CycloneDX","components":[
            {"name":"lodash","version":"4.17.20","purl":"pkg:npm/lodash@4.17.20"}
        ]}"#;
        let p = ingest_cyclonedx(s).unwrap();
        assert_eq!(p[0].ecosystem, "npm");
        assert_eq!(p[0].purl.as_deref(), Some("pkg:npm/lodash@4.17.20"));
    }

    #[test]
    fn ingest_spdx_external_purl() {
        let s = r#"{"spdxVersion":"SPDX-2.3","packages":[
            {"name":"openssl","versionInfo":"3.0.0","externalRefs":[
                {"referenceType":"purl","referenceLocator":"pkg:apk/alpine/openssl@3.0.0"}
            ]}
        ]}"#;
        let p = ingest_spdx(s).unwrap();
        assert_eq!(p[0].ecosystem, "alpine");
    }

    #[test]
    fn ecosystem_purl_table() {
        assert_eq!(ecosystem_from_purl("pkg:npm/foo@1"), "npm");
        assert_eq!(ecosystem_from_purl("pkg:apk/alpine/x@1"), "alpine");
        assert_eq!(ecosystem_from_purl("pkg:deb/debian/x@1"), "debian");
        assert_eq!(ecosystem_from_purl("pkg:rpm/x@1"), "rhel");
        assert_eq!(ecosystem_from_purl("pkg:cargo/x@1"), "cargo");
        assert_eq!(ecosystem_from_purl("pkg:weird/x@1"), "generic");
        assert_eq!(ecosystem_from_purl(""), "generic");
    }

    #[test]
    fn scan_sbom_end_to_end() {
        let s = r#"{"bomFormat":"CycloneDX","components":[
            {"name":"openssl-sys","version":"0.9.0","purl":"pkg:cargo/openssl-sys@0.9.0"}
        ]}"#;
        let r = scan_sbom("sbom.json", s, &VulnDb::cave_default()).unwrap();
        assert!(r.results[0]
            .vulnerabilities
            .iter()
            .any(|v| v.id == "CVE-2026-0030"));
    }

    #[test]
    fn scan_sbom_unknown_format() {
        let r = scan_sbom("x", "{}", &VulnDb::cave_default());
        assert!(r.is_err());
    }
}
