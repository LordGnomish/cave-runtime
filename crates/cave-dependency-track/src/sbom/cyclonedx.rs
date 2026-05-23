// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CycloneDX 1.4 / 1.5 / 1.6 JSON parser.

use crate::error::{Error, Result};
use crate::models::Classifier;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct CycloneDxBom {
    pub spec_version: String,
    pub serial_number: Option<String>,
    pub version: u32,
    pub metadata_component: Option<CdxComponent>,
    pub components: Vec<CdxComponent>,
    pub dependencies: Vec<CdxDependency>,
    pub vulnerabilities: Vec<CdxVuln>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CdxComponent {
    pub bom_ref: Option<String>,
    pub component_type: String,
    pub name: String,
    pub version: Option<String>,
    pub group: Option<String>,
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub hashes: Vec<(String, String)>,
    pub licenses: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CdxDependency {
    pub r#ref: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CdxVuln {
    pub id: String,
    pub source: Option<String>,
    pub affects: Vec<String>,
}

#[derive(Deserialize)]
struct RawDoc {
    #[serde(rename = "bomFormat", default)]
    bom_format: String,
    #[serde(rename = "specVersion", default)]
    spec_version: String,
    #[serde(rename = "serialNumber", default)]
    serial_number: Option<String>,
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    metadata: Option<RawMetadata>,
    #[serde(default)]
    components: Vec<RawComponent>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    vulnerabilities: Vec<RawVuln>,
}

#[derive(Deserialize)]
struct RawMetadata {
    #[serde(default)]
    component: Option<RawComponent>,
}

#[derive(Deserialize)]
struct RawComponent {
    #[serde(rename = "bom-ref", default)]
    bom_ref: Option<String>,
    #[serde(default, rename = "type")]
    component_type: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    purl: Option<String>,
    #[serde(default)]
    cpe: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    hashes: Vec<RawHash>,
    #[serde(default)]
    licenses: Vec<RawLicenseChoice>,
}

#[derive(Deserialize)]
struct RawHash {
    #[serde(default)]
    alg: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawLicenseChoice {
    Expression {
        expression: String,
    },
    License {
        license: RawLicense,
    },
    #[allow(dead_code)]
    Raw(serde_json::Value),
}

#[derive(Deserialize)]
struct RawLicense {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct RawDependency {
    #[serde(default, rename = "ref")]
    r#ref: String,
    #[serde(default, rename = "dependsOn")]
    depends_on: Vec<String>,
}

#[derive(Deserialize)]
struct RawVuln {
    #[serde(default)]
    id: String,
    #[serde(default)]
    source: Option<RawVulnSource>,
    #[serde(default)]
    affects: Vec<RawAffect>,
}

#[derive(Deserialize)]
struct RawVulnSource {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct RawAffect {
    #[serde(default, rename = "ref")]
    r#ref: String,
}

pub fn parse_cyclonedx_json(input: &str) -> Result<CycloneDxBom> {
    let doc: RawDoc =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("cyclonedx: {}", e)))?;
    if !doc.bom_format.is_empty() && doc.bom_format != "CycloneDX" {
        return Err(Error::Parse(format!(
            "expected bomFormat=CycloneDX, got {}",
            doc.bom_format
        )));
    }
    let spec = if doc.spec_version.is_empty() {
        "1.6".to_string()
    } else {
        doc.spec_version
    };
    if !matches!(spec.as_str(), "1.4" | "1.5" | "1.6") {
        return Err(Error::Parse(format!(
            "unsupported CycloneDX spec_version={} (need 1.4|1.5|1.6)",
            spec
        )));
    }
    Ok(CycloneDxBom {
        spec_version: spec,
        serial_number: doc.serial_number,
        version: doc.version.unwrap_or(1),
        metadata_component: doc
            .metadata
            .and_then(|m| m.component)
            .map(convert_component),
        components: doc.components.into_iter().map(convert_component).collect(),
        dependencies: doc
            .dependencies
            .into_iter()
            .map(|d| CdxDependency {
                r#ref: d.r#ref,
                depends_on: d.depends_on,
            })
            .collect(),
        vulnerabilities: doc
            .vulnerabilities
            .into_iter()
            .map(|v| CdxVuln {
                id: v.id,
                source: v.source.and_then(|s| s.name),
                affects: v.affects.into_iter().map(|a| a.r#ref).collect(),
            })
            .collect(),
    })
}

fn convert_component(c: RawComponent) -> CdxComponent {
    let licenses = c
        .licenses
        .into_iter()
        .filter_map(|raw| match raw {
            RawLicenseChoice::Expression { expression } => Some(expression),
            RawLicenseChoice::License { license } => license.id.or(license.name),
            RawLicenseChoice::Raw(_) => None,
        })
        .collect();
    CdxComponent {
        bom_ref: c.bom_ref,
        component_type: c.component_type.unwrap_or_else(|| "library".to_string()),
        name: c.name,
        version: c.version,
        group: c.group,
        purl: c.purl,
        cpe: c.cpe,
        hashes: c
            .hashes
            .into_iter()
            .map(|h| (h.alg, h.content))
            .collect(),
        licenses,
        description: c.description,
    }
}

impl CdxComponent {
    pub fn classifier(&self) -> Classifier {
        match self.component_type.as_str() {
            "application" => Classifier::Application,
            "framework" => Classifier::Framework,
            "library" => Classifier::Library,
            "container" => Classifier::Container,
            "operating-system" => Classifier::OperatingSystem,
            "device" => Classifier::Device,
            "firmware" => Classifier::Firmware,
            "file" => Classifier::File,
            _ => Classifier::Library,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN_1_4: &str = r#"{
        "bomFormat":"CycloneDX","specVersion":"1.4","version":1,
        "metadata":{"component":{"type":"application","name":"cave-runtime","version":"0.1.0"}},
        "components":[
          {"type":"library","name":"serde","version":"1.0","purl":"pkg:cargo/serde@1.0",
           "hashes":[{"alg":"SHA-256","content":"00"}],
           "licenses":[{"license":{"id":"MIT"}}]},
          {"type":"library","name":"tokio","version":"1","licenses":[{"expression":"Apache-2.0 OR MIT"}]}
        ]}"#;

    const MIN_1_6: &str = r#"{
        "bomFormat":"CycloneDX","specVersion":"1.6","version":2,
        "serialNumber":"urn:uuid:11111111-1111-1111-1111-111111111111",
        "components":[{"type":"container","name":"runtime","version":"0.1"}],
        "dependencies":[{"ref":"a","dependsOn":["b","c"]}],
        "vulnerabilities":[{"id":"CVE-2026-0001","source":{"name":"NVD"},"affects":[{"ref":"a"}]}]}"#;

    #[test]
    fn parses_1_4_minimum() {
        let bom = parse_cyclonedx_json(MIN_1_4).unwrap();
        assert_eq!(bom.spec_version, "1.4");
        assert_eq!(bom.version, 1);
        assert_eq!(bom.components.len(), 2);
        let serde_c = &bom.components[0];
        assert_eq!(serde_c.licenses, vec!["MIT"]);
        let tokio_c = &bom.components[1];
        assert_eq!(tokio_c.licenses, vec!["Apache-2.0 OR MIT"]);
    }

    #[test]
    fn parses_1_6_with_serial_and_deps_and_vulns() {
        let bom = parse_cyclonedx_json(MIN_1_6).unwrap();
        assert_eq!(bom.spec_version, "1.6");
        assert_eq!(bom.serial_number.as_deref(), Some("urn:uuid:11111111-1111-1111-1111-111111111111"));
        assert_eq!(bom.dependencies.len(), 1);
        assert_eq!(bom.dependencies[0].depends_on.len(), 2);
        assert_eq!(bom.vulnerabilities.len(), 1);
        assert_eq!(bom.vulnerabilities[0].source.as_deref(), Some("NVD"));
        assert_eq!(bom.components[0].classifier(), Classifier::Container);
    }

    #[test]
    fn rejects_wrong_format() {
        let bad = r#"{"bomFormat":"SPDX","specVersion":"1.6","components":[]}"#;
        assert!(matches!(parse_cyclonedx_json(bad), Err(Error::Parse(_))));
    }

    #[test]
    fn rejects_unsupported_spec() {
        let bad = r#"{"bomFormat":"CycloneDX","specVersion":"0.9","components":[]}"#;
        let err = parse_cyclonedx_json(bad).unwrap_err();
        assert!(format!("{}", err).contains("unsupported"));
    }

    #[test]
    fn metadata_component_extracted() {
        let bom = parse_cyclonedx_json(MIN_1_4).unwrap();
        let meta = bom.metadata_component.unwrap();
        assert_eq!(meta.name, "cave-runtime");
        assert_eq!(meta.classifier(), Classifier::Application);
    }

    #[test]
    fn defaults_spec_to_1_6_when_missing() {
        let bom =
            parse_cyclonedx_json(r#"{"bomFormat":"CycloneDX","components":[]}"#).unwrap();
        assert_eq!(bom.spec_version, "1.6");
    }

    #[test]
    fn malformed_json_errors() {
        assert!(matches!(parse_cyclonedx_json("{"), Err(Error::Parse(_))));
    }

    #[test]
    fn hashes_preserved() {
        let bom = parse_cyclonedx_json(MIN_1_4).unwrap();
        let c = bom.components.iter().find(|c| c.name == "serde").unwrap();
        assert_eq!(c.hashes, vec![("SHA-256".to_string(), "00".to_string())]);
    }
}
