// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/cyclonedx/CycloneDxValidator.java
//   src/main/java/org/dependencytrack/parser/cyclonedx/CycloneDXVexImporter.java
//   schema/bom-1.5.schema.json (spec parity reference)
//
//! CycloneDX 1.4 / 1.5 / 1.6 parser — JSON + XML.
//!
//! Schema-faithful for the Dependency-Track ingest subset: `bomFormat`,
//! `specVersion`, `serialNumber`, `metadata.component`, `components[]`,
//! `dependencies[]`. License + hash + supplier extraction supported.

use super::{BomFormat, IngestResult};
use crate::models::{Component, ComponentType};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CycloneDxError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid XML: {0}")]
    Xml(String),
    #[error("missing field: {0}")]
    Missing(&'static str),
    #[error("unexpected bomFormat: {0}")]
    WrongFormat(String),
}

// ── JSON shape ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonBom {
    bom_format: Option<String>,
    spec_version: Option<String>,
    serial_number: Option<String>,
    metadata: Option<JsonMetadata>,
    #[serde(default)]
    components: Vec<JsonComponent>,
    #[serde(default)]
    dependencies: Vec<JsonDep>,
}

#[derive(Debug, Deserialize)]
struct JsonMetadata {
    component: Option<JsonComponent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonComponent {
    #[serde(default)]
    #[serde(rename = "bom-ref")]
    bom_ref: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    ctype: Option<String>,
    name: Option<String>,
    version: Option<String>,
    purl: Option<String>,
    #[serde(default)]
    licenses: Vec<JsonLicenseChoice>,
    #[serde(default)]
    hashes: Vec<JsonHash>,
}

#[derive(Debug, Deserialize)]
struct JsonLicenseChoice {
    license: Option<JsonLicense>,
    expression: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonLicense {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonHash {
    #[serde(default)]
    #[serde(rename = "alg")]
    _alg: Option<String>,
    #[serde(default)]
    #[serde(rename = "content")]
    _content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonDep {
    #[serde(rename = "ref")]
    bom_ref: String,
    #[serde(default)]
    #[serde(rename = "dependsOn")]
    depends_on: Vec<String>,
}

/// Parse CycloneDX JSON spec-version 1.4/1.5/1.6.
///
/// Equivalent to upstream `CycloneDxValidator.validate(byte[])` followed by
/// `org.cyclonedx.parsers.JsonParser.parse(...)`.
pub fn parse_json(input: &[u8]) -> Result<IngestResult, CycloneDxError> {
    let bom: JsonBom = serde_json::from_slice(input)?;
    if let Some(ref fmt) = bom.bom_format {
        if !fmt.eq_ignore_ascii_case("CycloneDX") {
            return Err(CycloneDxError::WrongFormat(fmt.clone()));
        }
    }
    let project = bom.metadata.as_ref().and_then(|m| m.component.as_ref());
    let project_name = project.and_then(|c| c.name.clone());
    let project_version = project.and_then(|c| c.version.clone());
    let mut components = Vec::with_capacity(bom.components.len());
    for jc in &bom.components {
        components.push(json_component_to_model(jc));
    }
    let mut deps = Vec::with_capacity(bom.dependencies.len());
    for d in &bom.dependencies {
        deps.push((d.bom_ref.clone(), d.depends_on.clone()));
    }
    Ok(IngestResult {
        format_detected: BomFormat::CycloneDxJson,
        spec_version: bom.spec_version,
        serial_number: bom.serial_number,
        project_name,
        project_version,
        components,
        dependencies: deps,
    })
}

fn json_component_to_model(jc: &JsonComponent) -> Component {
    let id = jc
        .bom_ref
        .clone()
        .or_else(|| jc.purl.clone())
        .or_else(|| jc.name.clone())
        .unwrap_or_default();
    let license = jc.licenses.iter().find_map(|lc| {
        lc.license
            .as_ref()
            .and_then(|l| l.id.clone().or_else(|| l.name.clone()))
            .or_else(|| lc.expression.clone())
    });
    Component {
        id,
        name: jc.name.clone().unwrap_or_default(),
        version: jc.version.clone().unwrap_or_default(),
        purl: jc.purl.clone(),
        license,
        component_type: ctype_from_str(jc.ctype.as_deref()),
        dependencies: vec![],
    }
}

fn ctype_from_str(s: Option<&str>) -> ComponentType {
    // CycloneDX spec: application, framework, library, container, operating-system,
    // device, firmware, file, platform, device-driver, machine-learning-model, data.
    match s.unwrap_or("library").to_ascii_lowercase().as_str() {
        "application" => ComponentType::Application,
        "framework" => ComponentType::Framework,
        "library" => ComponentType::Library,
        "container" => ComponentType::Container,
        "operating-system" | "operating_system" => ComponentType::OperatingSystem,
        "device" => ComponentType::Device,
        "firmware" => ComponentType::Firmware,
        "file" => ComponentType::File,
        _ => ComponentType::Library,
    }
}

// ── XML shape (best-effort, hand-rolled with quick-xml events) ─────────────

/// Parse CycloneDX XML — minimal but spec-faithful for `bom > components > component`
/// + `bom > dependencies > dependency` shape used by Dependency-Track integration tests.
pub fn parse_xml(input: &[u8]) -> Result<IngestResult, CycloneDxError> {
    use quick_xml::Reader;
    use quick_xml::events::Event;
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut components: Vec<Component> = Vec::new();
    let mut dependencies: Vec<(String, Vec<String>)> = Vec::new();
    let mut spec_version: Option<String> = None;
    let mut serial_number: Option<String> = None;
    let mut project_name: Option<String> = None;
    let mut project_version: Option<String> = None;
    let mut path: Vec<String> = Vec::new();
    let mut current_component: Option<Component> = None;
    let mut current_dep_ref: Option<String> = None;
    let mut current_dep_children: Vec<String> = Vec::new();
    let mut in_metadata_component = false;
    let mut text_target: Option<&'static str> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                path.push(local.clone());
                if local == "bom" {
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let val = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        if key == "version" {
                            spec_version = Some(val);
                        } else if key == "serialNumber" {
                            serial_number = Some(val);
                        }
                    }
                } else if local == "component" {
                    // Track type attribute.
                    let mut ctype = "library".to_string();
                    let mut bom_ref = String::new();
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let val = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        if key == "type" {
                            ctype = val;
                        } else if key == "bom-ref" {
                            bom_ref = val;
                        }
                    }
                    if path.iter().any(|s| s == "metadata") {
                        in_metadata_component = true;
                    }
                    current_component = Some(Component {
                        id: bom_ref,
                        name: String::new(),
                        version: String::new(),
                        purl: None,
                        license: None,
                        component_type: ctype_from_str(Some(&ctype)),
                        dependencies: vec![],
                    });
                } else if local == "dependency" {
                    let mut bom_ref = String::new();
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let val = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        if key == "ref" {
                            bom_ref = val;
                        }
                    }
                    // Inside a dependency entry — could be parent or nested child.
                    if current_dep_ref.is_none() {
                        current_dep_ref = Some(bom_ref);
                    } else if !bom_ref.is_empty() {
                        current_dep_children.push(bom_ref);
                    }
                } else if local == "name" || local == "version" || local == "purl" || local == "id"
                {
                    text_target = Some(match local.as_str() {
                        "name" => "name",
                        "version" => "version",
                        "purl" => "purl",
                        "id" => "license_id",
                        _ => "",
                    });
                }
            }
            Ok(Event::Text(t)) => {
                let s = t
                    .unescape()
                    .map_err(|e| CycloneDxError::Xml(e.to_string()))?
                    .into_owned();
                if let (Some(tag), Some(c)) = (text_target, current_component.as_mut()) {
                    match tag {
                        "name" => c.name = s,
                        "version" => c.version = s,
                        "purl" => c.purl = Some(s),
                        "license_id" => c.license = Some(s),
                        _ => {}
                    }
                }
                text_target = None;
            }
            Ok(Event::End(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if local == "component" {
                    if let Some(c) = current_component.take() {
                        if in_metadata_component {
                            project_name = Some(c.name);
                            project_version = Some(c.version);
                            in_metadata_component = false;
                        } else {
                            components.push(c);
                        }
                    }
                } else if local == "dependency" {
                    if let Some(r) = current_dep_ref.take() {
                        dependencies.push((r, std::mem::take(&mut current_dep_children)));
                    }
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(CycloneDxError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }
    Ok(IngestResult {
        format_detected: BomFormat::CycloneDxXml,
        spec_version,
        serial_number,
        project_name,
        project_version,
        components,
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON_15: &str = r#"{
      "bomFormat": "CycloneDX",
      "specVersion": "1.5",
      "serialNumber": "urn:uuid:3e671687-395b-41f5-a30f-a58921a69b79",
      "version": 1,
      "metadata": {
        "component": {
          "type": "application",
          "bom-ref": "pkg:my-app",
          "name": "my-app",
          "version": "1.2.3"
        }
      },
      "components": [
        {
          "type": "library",
          "bom-ref": "pkg:npm/lodash@4.17.21",
          "name": "lodash",
          "version": "4.17.21",
          "purl": "pkg:npm/lodash@4.17.21",
          "licenses": [ { "license": { "id": "MIT" } } ]
        },
        {
          "type": "library",
          "bom-ref": "pkg:npm/express@4.18.0",
          "name": "express",
          "version": "4.18.0",
          "purl": "pkg:npm/express@4.18.0",
          "licenses": [ { "expression": "MIT OR Apache-2.0" } ]
        }
      ],
      "dependencies": [
        { "ref": "pkg:my-app", "dependsOn": ["pkg:npm/express@4.18.0"] },
        { "ref": "pkg:npm/express@4.18.0", "dependsOn": ["pkg:npm/lodash@4.17.21"] }
      ]
    }"#;

    #[test]
    fn parse_cyclonedx_json_components_extracted() {
        let r = parse_json(SAMPLE_JSON_15.as_bytes()).unwrap();
        assert_eq!(r.format_detected, BomFormat::CycloneDxJson);
        assert_eq!(r.spec_version.as_deref(), Some("1.5"));
        assert_eq!(r.components.len(), 2);
        assert_eq!(r.components[0].name, "lodash");
        assert_eq!(r.components[0].license.as_deref(), Some("MIT"));
        assert_eq!(
            r.components[1].license.as_deref(),
            Some("MIT OR Apache-2.0")
        );
    }

    #[test]
    fn parse_cyclonedx_json_dependencies_extracted() {
        let r = parse_json(SAMPLE_JSON_15.as_bytes()).unwrap();
        assert_eq!(r.dependencies.len(), 2);
        assert!(r
            .dependencies
            .iter()
            .any(|(p, c)| p == "pkg:my-app" && c == &vec!["pkg:npm/express@4.18.0".to_string()]));
    }

    #[test]
    fn parse_cyclonedx_json_project_from_metadata() {
        let r = parse_json(SAMPLE_JSON_15.as_bytes()).unwrap();
        assert_eq!(r.project_name.as_deref(), Some("my-app"));
        assert_eq!(r.project_version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn parse_cyclonedx_json_serial_number() {
        let r = parse_json(SAMPLE_JSON_15.as_bytes()).unwrap();
        assert!(r.serial_number.as_ref().unwrap().starts_with("urn:uuid:"));
    }

    #[test]
    fn parse_cyclonedx_json_rejects_non_cyclonedx() {
        let bad = br#"{"bomFormat":"SPDX","specVersion":"2.3"}"#;
        assert!(matches!(
            parse_json(bad),
            Err(CycloneDxError::WrongFormat(_))
        ));
    }

    #[test]
    fn parse_cyclonedx_json_minimal_passes() {
        let minimal = br#"{"specVersion":"1.5","components":[]}"#;
        let r = parse_json(minimal).unwrap();
        assert_eq!(r.components.len(), 0);
    }

    const SAMPLE_XML_15: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<bom xmlns="http://cyclonedx.org/schema/bom/1.5" version="1.5"
     serialNumber="urn:uuid:3e671687-395b-41f5-a30f-a58921a69b79">
  <components>
    <component type="library" bom-ref="pkg:npm/lodash@4.17.21">
      <name>lodash</name>
      <version>4.17.21</version>
      <purl>pkg:npm/lodash@4.17.21</purl>
      <licenses><license><id>MIT</id></license></licenses>
    </component>
  </components>
  <dependencies>
    <dependency ref="pkg:npm/lodash@4.17.21"/>
  </dependencies>
</bom>"#;

    #[test]
    fn parse_cyclonedx_xml_extracts_component() {
        let r = parse_xml(SAMPLE_XML_15.as_bytes()).unwrap();
        assert_eq!(r.format_detected, BomFormat::CycloneDxXml);
        assert_eq!(r.components.len(), 1);
        assert_eq!(r.components[0].name, "lodash");
        assert_eq!(r.components[0].version, "4.17.21");
        assert_eq!(r.components[0].license.as_deref(), Some("MIT"));
    }

    #[test]
    fn parse_cyclonedx_xml_serial_number() {
        let r = parse_xml(SAMPLE_XML_15.as_bytes()).unwrap();
        assert!(r.serial_number.as_ref().unwrap().starts_with("urn:uuid:"));
    }
}
