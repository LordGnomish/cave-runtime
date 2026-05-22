// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: SPDX 2.3 spec — spdx.github.io/spdx-spec/

//! SPDX 2.3 SBOM serializer (JSON form).

use super::Report;
use serde_json::{Value, json};

pub fn to_spdx(report: &Report) -> Value {
    let packages: Vec<Value> = report
        .packages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut pkg = json!({
                "SPDXID": format!("SPDXRef-Package-{i}"),
                "name": p.name,
                "versionInfo": p.version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": false,
            });
            if let Some(lic) = &p.license {
                pkg["licenseConcluded"] = json!(lic);
                pkg["licenseDeclared"] = json!(lic);
            } else {
                pkg["licenseConcluded"] = json!("NOASSERTION");
                pkg["licenseDeclared"] = json!("NOASSERTION");
            }
            if let Some(purl) = &p.purl {
                pkg["externalRefs"] = json!([{
                    "referenceCategory": "PACKAGE-MANAGER",
                    "referenceType": "purl",
                    "referenceLocator": purl,
                }]);
            }
            pkg
        })
        .collect();

    let relationships: Vec<Value> = (0..report.packages.len())
        .map(|i| {
            json!({
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relatedSpdxElement": format!("SPDXRef-Package-{i}"),
                "relationshipType": "DESCRIBES",
            })
        })
        .collect();

    json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": report.target,
        "documentNamespace": format!("https://cave-runtime/spdx/{}", report.target),
        "creationInfo": {
            "creators": ["Tool: cave-scan"],
            "created": "1970-01-01T00:00:00Z",
        },
        "packages": packages,
        "relationships": relationships,
    })
}

pub fn to_string_pretty(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&to_spdx(report))
}
