// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/spdx/json/SpdxDocumentParser.java
//   spdx.github.io/spdx-spec/v2.3 (spec reference)
//
//! SPDX 2.2 / 2.3 parser — JSON + tag-value.
//!
//! Scope: `packages[]`, `relationships[]`, document-level metadata
//! (`spdxVersion`, `documentNamespace`, `name`).

use super::{BomFormat, IngestResult};
use crate::models::{Component, ComponentType};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpdxError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("missing field: {0}")]
    Missing(&'static str),
    #[error("invalid tag-value: {0}")]
    TagValue(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonSpdx {
    #[serde(rename = "SPDXID")]
    spdxid: Option<String>,
    name: Option<String>,
    spdx_version: Option<String>,
    document_namespace: Option<String>,
    #[serde(default)]
    packages: Vec<JsonPackage>,
    #[serde(default)]
    relationships: Vec<JsonRelationship>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonPackage {
    #[serde(rename = "SPDXID")]
    spdxid: String,
    name: Option<String>,
    version_info: Option<String>,
    #[serde(default)]
    external_refs: Vec<JsonExternalRef>,
    license_concluded: Option<String>,
    license_declared: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonExternalRef {
    #[serde(rename = "referenceType")]
    reference_type: Option<String>,
    #[serde(rename = "referenceCategory")]
    reference_category: Option<String>,
    #[serde(rename = "referenceLocator")]
    reference_locator: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonRelationship {
    spdx_element_id: String,
    related_spdx_element: String,
    relationship_type: String,
}

/// Parse SPDX JSON document. Mirrors `SpdxDocumentParser.parse(byte[])`.
pub fn parse_json(input: &[u8]) -> Result<IngestResult, SpdxError> {
    let doc: JsonSpdx = serde_json::from_slice(input)?;
    let project_name = doc.name.clone();
    let project_version = None; // SPDX has no first-class document version.
    let mut components = Vec::with_capacity(doc.packages.len());
    for p in &doc.packages {
        components.push(package_to_component(p));
    }
    // Build dependency edges from DEPENDS_ON / CONTAINS / DEPENDENCY_OF relationships.
    let mut deps: Vec<(String, Vec<String>)> = Vec::new();
    use std::collections::HashMap;
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for r in &doc.relationships {
        match r.relationship_type.as_str() {
            "DEPENDS_ON" | "CONTAINS" => {
                grouped
                    .entry(r.spdx_element_id.clone())
                    .or_default()
                    .push(r.related_spdx_element.clone());
            }
            "DEPENDENCY_OF" => {
                grouped
                    .entry(r.related_spdx_element.clone())
                    .or_default()
                    .push(r.spdx_element_id.clone());
            }
            _ => {}
        }
    }
    for (k, v) in grouped {
        deps.push((k, v));
    }
    Ok(IngestResult {
        format_detected: BomFormat::SpdxJson,
        spec_version: doc.spdx_version,
        serial_number: doc.document_namespace.or(doc.spdxid),
        project_name,
        project_version,
        components,
        dependencies: deps,
    })
}

fn package_to_component(p: &JsonPackage) -> Component {
    let purl = p.external_refs.iter().find_map(|er| {
        if er.reference_type.as_deref() == Some("purl")
            || er.reference_category.as_deref() == Some("PACKAGE-MANAGER")
        {
            er.reference_locator.clone()
        } else {
            None
        }
    });
    let license = p
        .license_concluded
        .clone()
        .filter(|s| !s.is_empty() && s != "NOASSERTION")
        .or_else(|| {
            p.license_declared
                .clone()
                .filter(|s| !s.is_empty() && s != "NOASSERTION")
        });
    Component {
        id: p.spdxid.clone(),
        name: p.name.clone().unwrap_or_default(),
        version: p.version_info.clone().unwrap_or_default(),
        purl,
        license,
        component_type: ComponentType::Library,
        dependencies: vec![],
    }
}

/// Parse SPDX tag-value (`SPDXVersion: SPDX-2.3` style). Mirrors
/// `SpdxToolsTagValueParser.parse(InputStream)`.
pub fn parse_tag_value(input: &[u8]) -> Result<IngestResult, SpdxError> {
    let text = std::str::from_utf8(input).map_err(|_| SpdxError::TagValue("not utf8".into()))?;
    let mut spec_version = None;
    let mut doc_namespace = None;
    let mut project_name = None;
    let mut components: Vec<Component> = Vec::new();
    let mut current_pkg: Option<Component> = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        match k {
            "SPDXVersion" => spec_version = Some(v.to_string()),
            "DocumentName" => project_name = Some(v.to_string()),
            "DocumentNamespace" => doc_namespace = Some(v.to_string()),
            "PackageName" => {
                // Flush previous package.
                if let Some(p) = current_pkg.take() {
                    components.push(p);
                }
                current_pkg = Some(Component {
                    id: String::new(),
                    name: v.to_string(),
                    version: String::new(),
                    purl: None,
                    license: None,
                    component_type: ComponentType::Library,
                    dependencies: vec![],
                });
            }
            "SPDXID" => {
                if let Some(p) = current_pkg.as_mut() {
                    p.id = v.to_string();
                }
            }
            "PackageVersion" => {
                if let Some(p) = current_pkg.as_mut() {
                    p.version = v.to_string();
                }
            }
            "PackageLicenseConcluded" | "PackageLicenseDeclared" => {
                if v != "NOASSERTION" {
                    if let Some(p) = current_pkg.as_mut() {
                        if p.license.is_none() {
                            p.license = Some(v.to_string());
                        }
                    }
                }
            }
            "ExternalRef" => {
                // Format: "ExternalRef: PACKAGE-MANAGER purl pkg:npm/lodash@4.17.21"
                let parts: Vec<&str> = v.split_whitespace().collect();
                if parts.len() >= 3 && parts[1] == "purl" {
                    if let Some(p) = current_pkg.as_mut() {
                        p.purl = Some(parts[2].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(p) = current_pkg {
        components.push(p);
    }
    Ok(IngestResult {
        format_detected: BomFormat::SpdxTagValue,
        spec_version,
        serial_number: doc_namespace,
        project_name,
        project_version: None,
        components,
        dependencies: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON_23: &str = r#"{
      "spdxVersion": "SPDX-2.3",
      "SPDXID": "SPDXRef-DOCUMENT",
      "name": "my-app-0.1.0",
      "documentNamespace": "https://example.com/spdx/my-app",
      "packages": [
        {
          "SPDXID": "SPDXRef-Package-lodash",
          "name": "lodash",
          "versionInfo": "4.17.21",
          "licenseConcluded": "MIT",
          "externalRefs": [
            { "referenceCategory":"PACKAGE-MANAGER", "referenceType":"purl",
              "referenceLocator":"pkg:npm/lodash@4.17.21" }
          ]
        },
        {
          "SPDXID": "SPDXRef-Package-app",
          "name": "my-app",
          "versionInfo": "0.1.0",
          "licenseDeclared": "Apache-2.0"
        }
      ],
      "relationships": [
        { "spdxElementId": "SPDXRef-Package-app",
          "relatedSpdxElement": "SPDXRef-Package-lodash",
          "relationshipType": "DEPENDS_ON" }
      ]
    }"#;

    #[test]
    fn parse_spdx_json_extracts_packages() {
        let r = parse_json(SAMPLE_JSON_23.as_bytes()).unwrap();
        assert_eq!(r.format_detected, BomFormat::SpdxJson);
        assert_eq!(r.spec_version.as_deref(), Some("SPDX-2.3"));
        assert_eq!(r.components.len(), 2);
        let lodash = r.components.iter().find(|c| c.name == "lodash").unwrap();
        assert_eq!(lodash.version, "4.17.21");
        assert_eq!(lodash.license.as_deref(), Some("MIT"));
        assert_eq!(lodash.purl.as_deref(), Some("pkg:npm/lodash@4.17.21"));
    }

    #[test]
    fn parse_spdx_json_extracts_dependencies() {
        let r = parse_json(SAMPLE_JSON_23.as_bytes()).unwrap();
        assert_eq!(r.dependencies.len(), 1);
        let (parent, children) = &r.dependencies[0];
        assert_eq!(parent, "SPDXRef-Package-app");
        assert_eq!(children, &vec!["SPDXRef-Package-lodash".to_string()]);
    }

    #[test]
    fn parse_spdx_json_document_namespace_as_serial() {
        let r = parse_json(SAMPLE_JSON_23.as_bytes()).unwrap();
        assert_eq!(
            r.serial_number.as_deref(),
            Some("https://example.com/spdx/my-app")
        );
    }

    #[test]
    fn parse_spdx_json_handles_noassertion() {
        let blob = br#"{
          "spdxVersion":"SPDX-2.3", "SPDXID":"SPDXRef-DOCUMENT",
          "packages":[{ "SPDXID":"SPDXRef-x","name":"x","versionInfo":"1.0",
                         "licenseConcluded":"NOASSERTION","licenseDeclared":"NOASSERTION" }]
        }"#;
        let r = parse_json(blob).unwrap();
        assert!(r.components[0].license.is_none());
    }

    const SAMPLE_TAGVALUE: &str = "\
SPDXVersion: SPDX-2.3
DataLicense: CC0-1.0
SPDXID: SPDXRef-DOCUMENT
DocumentName: my-app
DocumentNamespace: https://example.com/spdx/my-app

PackageName: lodash
SPDXID: SPDXRef-Package-lodash
PackageVersion: 4.17.21
PackageLicenseConcluded: MIT
ExternalRef: PACKAGE-MANAGER purl pkg:npm/lodash@4.17.21

PackageName: express
SPDXID: SPDXRef-Package-express
PackageVersion: 4.18.0
PackageLicenseConcluded: NOASSERTION
PackageLicenseDeclared: MIT
";

    #[test]
    fn parse_spdx_tagvalue_extracts_packages() {
        let r = parse_tag_value(SAMPLE_TAGVALUE.as_bytes()).unwrap();
        assert_eq!(r.format_detected, BomFormat::SpdxTagValue);
        assert_eq!(r.spec_version.as_deref(), Some("SPDX-2.3"));
        assert_eq!(r.project_name.as_deref(), Some("my-app"));
        assert_eq!(r.components.len(), 2);
        let lodash = r.components.iter().find(|c| c.name == "lodash").unwrap();
        assert_eq!(lodash.purl.as_deref(), Some("pkg:npm/lodash@4.17.21"));
        let express = r.components.iter().find(|c| c.name == "express").unwrap();
        assert_eq!(express.license.as_deref(), Some("MIT"));
    }
}
