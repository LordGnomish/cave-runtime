// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! SPDX 2.3 JSON SBOM emitter.
//!
//! Mirrors trivy's `pkg/sbom/spdx`. cave-trivy emits the minimum-viable
//! SPDX-JSON envelope: `spdxVersion`, `SPDXID`, `documentNamespace`,
//! `creationInfo`, and a `packages` array with `externalRefs` carrying
//! the purl.

use crate::error::{TrivyError, TrivyResult};
use crate::models::Package;
use crate::purl::{ecosystem_to_purl_type, PackageUrl};
use serde_json::{json, Value};

pub const SPDX_VERSION: &str = "SPDX-2.3";

pub fn emit(target: &str, pkgs: &[Package]) -> TrivyResult<String> {
    let mut packages = Vec::new();
    for (i, p) in pkgs.iter().enumerate() {
        let pty = ecosystem_to_purl_type(&p.ecosystem);
        let mut purl = PackageUrl::new(pty, &p.name, Some(&p.version));
        if matches!(pty, "apk" | "deb" | "rpm") {
            purl = purl.with_namespace(&p.ecosystem);
        }
        packages.push(json!({
            "SPDXID": format!("SPDXRef-Package-{}", i + 1),
            "name": p.name,
            "versionInfo": p.version,
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": false,
            "externalRefs": [{
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": purl.to_string_canonical(),
            }],
        }));
    }
    let doc = json!({
        "spdxVersion": SPDX_VERSION,
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": target,
        "documentNamespace": format!("https://cave.runtime/sbom/{}/{}", target, uuid::Uuid::new_v4()),
        "creationInfo": {
            "created": chrono::Utc::now().to_rfc3339(),
            "creators": [format!("Tool: cave-trivy-{}", crate::UPSTREAM_VERSION)],
            "licenseListVersion": "3.24"
        },
        "packages": packages,
    });
    serde_json::to_string_pretty(&doc).map_err(|e| TrivyError::Sbom(format!("spdx: {}", e)))
}

pub fn parse(text: &str) -> TrivyResult<Value> {
    serde_json::from_str(text).map_err(|e| TrivyError::parse(format!("spdx: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_spdx_envelope() {
        let pkgs = vec![Package::new("openssl", "3.0.0", "alpine")];
        let s = emit("alpine:3.19", &pkgs).unwrap();
        let v = parse(&s).unwrap();
        assert_eq!(v["spdxVersion"], SPDX_VERSION);
        assert_eq!(v["SPDXID"], "SPDXRef-DOCUMENT");
        assert_eq!(v["dataLicense"], "CC0-1.0");
        let pkgs = v["packages"].as_array().unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0]["name"], "openssl");
    }

    #[test]
    fn empty_packages() {
        let s = emit("x", &[]).unwrap();
        let v = parse(&s).unwrap();
        assert_eq!(v["packages"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn purl_in_external_refs() {
        let pkgs = vec![Package::new("lodash", "4.17.21", "npm")];
        let s = emit("x", &pkgs).unwrap();
        let v = parse(&s).unwrap();
        let ext = &v["packages"][0]["externalRefs"][0];
        assert_eq!(ext["referenceType"], "purl");
        assert!(ext["referenceLocator"]
            .as_str()
            .unwrap()
            .starts_with("pkg:npm/lodash@"));
    }

    #[test]
    fn round_trip_detect() {
        let s = emit("x", &[Package::new("p", "1", "npm")]).unwrap();
        assert_eq!(
            crate::scan_sbom::detect_format(&s),
            Some(crate::scan_sbom::SbomFormat::Spdx)
        );
    }

    #[test]
    fn parse_bad() {
        assert!(parse("not json").is_err());
    }
}
