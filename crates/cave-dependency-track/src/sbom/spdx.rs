// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SPDX 2.3 JSON + tag-value parser.
//!
//! Mirrors `org.dependencytrack.parser.spdx.json.SpdxJsonParser` and the
//! tag-value subset used by the upstream IO importer.

use crate::error::{Error, Result};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct SpdxDocument {
    pub spdx_version: String,
    pub data_license: String,
    pub document_name: String,
    pub document_namespace: Option<String>,
    pub creators: Vec<String>,
    pub packages: Vec<SpdxPackage>,
    pub relationships: Vec<SpdxRelationship>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpdxPackage {
    pub spdx_id: String,
    pub name: String,
    pub version: Option<String>,
    pub supplier: Option<String>,
    pub download_location: Option<String>,
    pub license_concluded: Option<String>,
    pub license_declared: Option<String>,
    pub copyright_text: Option<String>,
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub checksums: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpdxRelationship {
    pub spdx_element_id: String,
    pub related_spdx_element: String,
    pub relationship_type: String,
}

#[derive(Deserialize)]
struct RawJson {
    #[serde(rename = "spdxVersion", default)]
    spdx_version: String,
    #[serde(rename = "dataLicense", default)]
    data_license: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "documentNamespace", default)]
    document_namespace: Option<String>,
    #[serde(rename = "creationInfo", default)]
    creation_info: Option<RawCreationInfo>,
    #[serde(default)]
    packages: Vec<RawPackage>,
    #[serde(default)]
    relationships: Vec<RawRelationship>,
}

#[derive(Deserialize)]
struct RawCreationInfo {
    #[serde(default)]
    creators: Vec<String>,
}

#[derive(Deserialize)]
struct RawPackage {
    #[serde(rename = "SPDXID", default)]
    spdx_id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "versionInfo", default)]
    version: Option<String>,
    #[serde(default)]
    supplier: Option<String>,
    #[serde(rename = "downloadLocation", default)]
    download_location: Option<String>,
    #[serde(rename = "licenseConcluded", default)]
    license_concluded: Option<String>,
    #[serde(rename = "licenseDeclared", default)]
    license_declared: Option<String>,
    #[serde(rename = "copyrightText", default)]
    copyright_text: Option<String>,
    #[serde(rename = "externalRefs", default)]
    external_refs: Vec<RawExternalRef>,
    #[serde(default)]
    checksums: Vec<RawChecksum>,
}

#[derive(Deserialize)]
struct RawExternalRef {
    #[serde(rename = "referenceCategory", default)]
    category: String,
    #[serde(rename = "referenceType", default)]
    ref_type: String,
    #[serde(rename = "referenceLocator", default)]
    locator: String,
}

#[derive(Deserialize)]
struct RawChecksum {
    #[serde(default)]
    algorithm: String,
    #[serde(rename = "checksumValue", default)]
    value: String,
}

#[derive(Deserialize)]
struct RawRelationship {
    #[serde(rename = "spdxElementId", default)]
    spdx_element_id: String,
    #[serde(rename = "relatedSpdxElement", default)]
    related_spdx_element: String,
    #[serde(rename = "relationshipType", default)]
    relationship_type: String,
}

pub fn parse_spdx_json(input: &str) -> Result<SpdxDocument> {
    let raw: RawJson =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("spdx-json: {}", e)))?;
    let version = if raw.spdx_version.is_empty() {
        "SPDX-2.3".to_string()
    } else {
        raw.spdx_version
    };
    if !version.starts_with("SPDX-2.") && version != "SPDX-3.0" {
        return Err(Error::Parse(format!(
            "unsupported SPDX version: {}",
            version
        )));
    }
    let packages = raw
        .packages
        .into_iter()
        .map(|p| {
            let mut pkg = SpdxPackage {
                spdx_id: p.spdx_id,
                name: p.name,
                version: p.version,
                supplier: p.supplier,
                download_location: p.download_location,
                license_concluded: p.license_concluded,
                license_declared: p.license_declared,
                copyright_text: p.copyright_text,
                purl: None,
                cpe: None,
                checksums: p
                    .checksums
                    .into_iter()
                    .map(|c| (c.algorithm, c.value))
                    .collect(),
            };
            for r in p.external_refs {
                if r.category == "PACKAGE-MANAGER" && r.ref_type == "purl" {
                    pkg.purl = Some(r.locator);
                } else if r.category == "SECURITY"
                    && (r.ref_type == "cpe23Type" || r.ref_type == "cpe22Type")
                {
                    pkg.cpe = Some(r.locator);
                }
            }
            pkg
        })
        .collect();
    Ok(SpdxDocument {
        spdx_version: version,
        data_license: if raw.data_license.is_empty() {
            "CC0-1.0".into()
        } else {
            raw.data_license
        },
        document_name: raw.name,
        document_namespace: raw.document_namespace,
        creators: raw.creation_info.map(|c| c.creators).unwrap_or_default(),
        packages,
        relationships: raw
            .relationships
            .into_iter()
            .map(|r| SpdxRelationship {
                spdx_element_id: r.spdx_element_id,
                related_spdx_element: r.related_spdx_element,
                relationship_type: r.relationship_type,
            })
            .collect(),
    })
}

/// Tag-value SPDX 2.3 — `Key: Value` lines per the official spec.
pub fn parse_spdx_tag_value(input: &str) -> Result<SpdxDocument> {
    let mut doc = SpdxDocument {
        spdx_version: String::new(),
        data_license: String::new(),
        document_name: String::new(),
        document_namespace: None,
        creators: Vec::new(),
        packages: Vec::new(),
        relationships: Vec::new(),
    };
    let mut current: Option<SpdxPackage> = None;
    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim() {
            "SPDXVersion" => doc.spdx_version = value,
            "DataLicense" => doc.data_license = value,
            "DocumentName" => doc.document_name = value,
            "DocumentNamespace" => doc.document_namespace = Some(value),
            "Creator" => doc.creators.push(value),
            "PackageName" => {
                if let Some(p) = current.take() {
                    doc.packages.push(p);
                }
                current = Some(SpdxPackage {
                    name: value,
                    ..Default::default()
                });
            }
            "SPDXID" => {
                if let Some(p) = current.as_mut() {
                    p.spdx_id = value;
                }
            }
            "PackageVersion" => {
                if let Some(p) = current.as_mut() {
                    p.version = Some(value);
                }
            }
            "PackageSupplier" => {
                if let Some(p) = current.as_mut() {
                    p.supplier = Some(value);
                }
            }
            "PackageDownloadLocation" => {
                if let Some(p) = current.as_mut() {
                    p.download_location = Some(value);
                }
            }
            "PackageLicenseConcluded" => {
                if let Some(p) = current.as_mut() {
                    p.license_concluded = Some(value);
                }
            }
            "PackageLicenseDeclared" => {
                if let Some(p) = current.as_mut() {
                    p.license_declared = Some(value);
                }
            }
            "PackageCopyrightText" => {
                if let Some(p) = current.as_mut() {
                    p.copyright_text = Some(value);
                }
            }
            "PackageChecksum" => {
                // value = "SHA1: deadbeef"
                if let Some(p) = current.as_mut() {
                    if let Some((alg, hex)) = value.split_once(':') {
                        p.checksums.push((alg.trim().to_string(), hex.trim().to_string()));
                    }
                }
            }
            "ExternalRef" => {
                if let Some(p) = current.as_mut() {
                    // value = "PACKAGE-MANAGER purl pkg:..."
                    let parts: Vec<_> = value.split_whitespace().collect();
                    if parts.len() == 3 {
                        if parts[0] == "PACKAGE-MANAGER" && parts[1] == "purl" {
                            p.purl = Some(parts[2].to_string());
                        }
                        if parts[0] == "SECURITY" && parts[1].starts_with("cpe") {
                            p.cpe = Some(parts[2].to_string());
                        }
                    }
                }
            }
            "Relationship" => {
                let parts: Vec<_> = value.split_whitespace().collect();
                if parts.len() == 3 {
                    doc.relationships.push(SpdxRelationship {
                        spdx_element_id: parts[0].to_string(),
                        relationship_type: parts[1].to_string(),
                        related_spdx_element: parts[2].to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(p) = current.take() {
        doc.packages.push(p);
    }
    if doc.spdx_version.is_empty() {
        return Err(Error::Parse("missing SPDXVersion".into()));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_DOC: &str = r#"{
      "spdxVersion":"SPDX-2.3","dataLicense":"CC0-1.0","name":"cave-bom",
      "documentNamespace":"https://cave.svc/sbom/x",
      "creationInfo":{"creators":["Tool: cave-deptrack"]},
      "packages":[{
        "SPDXID":"SPDXRef-Package-1","name":"openssl","versionInfo":"3.2.0",
        "downloadLocation":"https://openssl.org",
        "licenseConcluded":"OpenSSL-3.0","licenseDeclared":"OpenSSL-3.0",
        "externalRefs":[
          {"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl","referenceLocator":"pkg:generic/openssl@3.2.0"},
          {"referenceCategory":"SECURITY","referenceType":"cpe23Type","referenceLocator":"cpe:2.3:a:openssl:openssl:3.2.0:*:*:*:*:*:*:*"}
        ],
        "checksums":[{"algorithm":"SHA256","checksumValue":"abcd"}]
      }],
      "relationships":[{"spdxElementId":"SPDXRef-DOCUMENT","relatedSpdxElement":"SPDXRef-Package-1","relationshipType":"DESCRIBES"}]
    }"#;

    #[test]
    fn parses_2_3_json() {
        let d = parse_spdx_json(JSON_DOC).unwrap();
        assert_eq!(d.spdx_version, "SPDX-2.3");
        assert_eq!(d.packages.len(), 1);
        let p = &d.packages[0];
        assert_eq!(p.name, "openssl");
        assert_eq!(p.purl.as_deref(), Some("pkg:generic/openssl@3.2.0"));
        assert!(p.cpe.is_some());
        assert_eq!(p.checksums, vec![("SHA256".to_string(), "abcd".to_string())]);
        assert_eq!(d.relationships.len(), 1);
        assert_eq!(d.relationships[0].relationship_type, "DESCRIBES");
    }

    #[test]
    fn accepts_3_0_json() {
        let raw = r#"{"spdxVersion":"SPDX-3.0","packages":[]}"#;
        let d = parse_spdx_json(raw).unwrap();
        assert_eq!(d.spdx_version, "SPDX-3.0");
    }

    #[test]
    fn rejects_old_version() {
        let raw = r#"{"spdxVersion":"SPDX-1.0","packages":[]}"#;
        assert!(matches!(parse_spdx_json(raw), Err(Error::Parse(_))));
    }

    #[test]
    fn parses_tag_value_minimal() {
        let txt = "\
SPDXVersion: SPDX-2.3
DataLicense: CC0-1.0
DocumentName: cave
PackageName: zlib
SPDXID: SPDXRef-Package-zlib
PackageVersion: 1.3.1
PackageLicenseConcluded: Zlib
PackageChecksum: SHA1: deadbeef
ExternalRef: PACKAGE-MANAGER purl pkg:generic/zlib@1.3.1
Relationship: SPDXRef-DOCUMENT DESCRIBES SPDXRef-Package-zlib
";
        let d = parse_spdx_tag_value(txt).unwrap();
        assert_eq!(d.spdx_version, "SPDX-2.3");
        assert_eq!(d.packages.len(), 1);
        assert_eq!(d.packages[0].purl.as_deref(), Some("pkg:generic/zlib@1.3.1"));
        assert_eq!(d.packages[0].checksums, vec![("SHA1".to_string(), "deadbeef".to_string())]);
        assert_eq!(d.relationships.len(), 1);
    }

    #[test]
    fn tag_value_missing_version_errors() {
        assert!(matches!(parse_spdx_tag_value(""), Err(Error::Parse(_))));
    }

    #[test]
    fn tag_value_multiple_packages() {
        let txt = "\
SPDXVersion: SPDX-2.3
PackageName: a
SPDXID: SPDXRef-a
PackageName: b
SPDXID: SPDXRef-b
";
        let d = parse_spdx_tag_value(txt).unwrap();
        assert_eq!(d.packages.len(), 2);
        assert_eq!(d.packages[0].name, "a");
        assert_eq!(d.packages[1].name, "b");
    }
}
