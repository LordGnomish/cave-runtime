//! SBOM generation — CycloneDX 1.4 and SPDX 2.3 (JSON format).

use crate::trivy::{
    lang_pkg::LangPackage,
    os_pkg::OsPackage,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// CycloneDX 1.4
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycloneDxBom {
    pub bom_format: String,
    pub spec_version: String,
    pub version: u32,
    pub serial_number: String,
    pub metadata: CycloneDxMetadata,
    pub components: Vec<CycloneDxComponent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<CycloneDxDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycloneDxMetadata {
    pub timestamp: String,
    pub tools: Vec<CycloneDxTool>,
    pub component: Option<CycloneDxComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycloneDxTool {
    pub vendor: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycloneDxComponent {
    #[serde(rename = "type")]
    pub component_type: String,
    #[serde(rename = "bom-ref")]
    pub bom_ref: String,
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hashes: Vec<CycloneDxHash>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub licenses: Vec<CycloneDxLicenseEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycloneDxHash {
    pub alg: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycloneDxLicenseEntry {
    pub license: CycloneDxLicense,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycloneDxLicense {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycloneDxDependency {
    #[serde(rename = "ref")]
    pub bom_ref: String,
    #[serde(rename = "dependsOn")]
    pub depends_on: Vec<String>,
}

/// Generate a CycloneDX BOM from OS and language packages.
pub fn generate_cyclonedx(
    image_ref: &str,
    os_packages: &[OsPackage],
    lang_packages: &[LangPackage],
) -> CycloneDxBom {
    let mut components = Vec::new();

    for pkg in os_packages {
        let bom_ref = format!("os/{}@{}", pkg.name, pkg.version);
        let purl = format!(
            "pkg:{}/{}@{}",
            pkg.package_manager,
            pkg.name,
            pkg.version
        );
        components.push(CycloneDxComponent {
            component_type: "library".into(),
            bom_ref: bom_ref.clone(),
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            purl: Some(purl),
            hashes: vec![],
            licenses: pkg
                .licenses
                .iter()
                .map(|l| CycloneDxLicenseEntry {
                    license: CycloneDxLicense { id: l.clone() },
                })
                .collect(),
        });
    }

    for pkg in lang_packages {
        let bom_ref = format!("{}/{}@{}", pkg.ecosystem, pkg.name, pkg.version);
        let purl = format!("pkg:{}/{}@{}", pkg.ecosystem, pkg.name, pkg.version);
        let hash = pkg.checksum.as_ref().map(|h| {
            let (alg, content) = if h.starts_with("sha256:") {
                ("SHA-256", h.trim_start_matches("sha256:").to_string())
            } else if h.starts_with("sha1:") {
                ("SHA-1", h.trim_start_matches("sha1:").to_string())
            } else {
                ("SHA-256", h.clone())
            };
            CycloneDxHash { alg: alg.to_string(), content }
        });
        components.push(CycloneDxComponent {
            component_type: "library".into(),
            bom_ref,
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            purl: Some(purl),
            hashes: hash.into_iter().collect(),
            licenses: vec![],
        });
    }

    let root = CycloneDxComponent {
        component_type: "container".into(),
        bom_ref: "root-container".into(),
        name: image_ref.to_string(),
        version: String::new(),
        purl: Some(format!("pkg:docker/{image_ref}")),
        hashes: vec![],
        licenses: vec![],
    };

    CycloneDxBom {
        bom_format: "CycloneDX".into(),
        spec_version: "1.4".into(),
        version: 1,
        serial_number: format!("urn:uuid:{}", Uuid::new_v4()),
        metadata: CycloneDxMetadata {
            timestamp: Utc::now().to_rfc3339(),
            tools: vec![CycloneDxTool {
                vendor: "CAVE Platform".into(),
                name: "cave-security".into(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            }],
            component: Some(root),
        },
        components,
        dependencies: vec![],
    }
}

// ---------------------------------------------------------------------------
// SPDX 2.3
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxDocument {
    pub spdx_version: String,
    pub data_license: String,
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    pub name: String,
    pub document_namespace: String,
    pub creation_info: SpdxCreationInfo,
    pub packages: Vec<SpdxPackage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<SpdxRelationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxCreationInfo {
    pub created: String,
    pub creators: Vec<String>,
    pub license_list_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxPackage {
    pub name: String,
    pub version: String,
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    pub download_location: String,
    pub files_analyzed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_concluded: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_declared: Option<String>,
    pub copyright_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_refs: Option<Vec<SpdxExternalRef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxExternalRef {
    pub reference_category: String,
    pub reference_type: String,
    pub reference_locator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxRelationship {
    pub spdx_element_id: String,
    pub relationship_type: String,
    pub related_spdx_element: String,
}

fn spdx_safe_id(name: &str, version: &str) -> String {
    let safe: String = format!("{name}-{version}")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '-' })
        .collect();
    format!("SPDXRef-{safe}")
}

/// Generate a SPDX 2.3 document.
pub fn generate_spdx(
    image_ref: &str,
    os_packages: &[OsPackage],
    lang_packages: &[LangPackage],
) -> SpdxDocument {
    let doc_namespace = format!(
        "https://cave.platform/spdx/{}/{}",
        image_ref.replace('/', "-"),
        Uuid::new_v4()
    );

    let mut packages = Vec::new();
    let mut relationships = Vec::new();

    for pkg in os_packages {
        let spdx_id = spdx_safe_id(&pkg.name, &pkg.version);
        let purl = format!("pkg:{}/{}@{}", pkg.package_manager, pkg.name, pkg.version);
        packages.push(SpdxPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            spdx_id: spdx_id.clone(),
            download_location: "NOASSERTION".into(),
            files_analyzed: false,
            source_info: pkg.source_name.clone(),
            license_concluded: pkg.licenses.first().cloned(),
            license_declared: pkg.licenses.first().cloned(),
            copyright_text: "NOASSERTION".into(),
            external_refs: Some(vec![SpdxExternalRef {
                reference_category: "PACKAGE-MANAGER".into(),
                reference_type: "purl".into(),
                reference_locator: purl,
            }]),
        });
        relationships.push(SpdxRelationship {
            spdx_element_id: "SPDXRef-DOCUMENT".into(),
            relationship_type: "DESCRIBES".into(),
            related_spdx_element: spdx_id,
        });
    }

    for pkg in lang_packages {
        let spdx_id = spdx_safe_id(&pkg.name, &pkg.version);
        let purl = format!("pkg:{}/{}@{}", pkg.ecosystem, pkg.name, pkg.version);
        packages.push(SpdxPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            spdx_id: spdx_id.clone(),
            download_location: "NOASSERTION".into(),
            files_analyzed: false,
            source_info: None,
            license_concluded: None,
            license_declared: None,
            copyright_text: "NOASSERTION".into(),
            external_refs: Some(vec![SpdxExternalRef {
                reference_category: "PACKAGE-MANAGER".into(),
                reference_type: "purl".into(),
                reference_locator: purl,
            }]),
        });
        relationships.push(SpdxRelationship {
            spdx_element_id: "SPDXRef-DOCUMENT".into(),
            relationship_type: "DESCRIBES".into(),
            related_spdx_element: spdx_id,
        });
    }

    SpdxDocument {
        spdx_version: "SPDX-2.3".into(),
        data_license: "CC0-1.0".into(),
        spdx_id: "SPDXRef-DOCUMENT".into(),
        name: image_ref.to_string(),
        document_namespace: doc_namespace,
        creation_info: SpdxCreationInfo {
            created: Utc::now().to_rfc3339(),
            creators: vec![
                format!("Tool: cave-security-{}", env!("CARGO_PKG_VERSION")),
                "Organization: CAVE Platform".into(),
            ],
            license_list_version: "3.21".into(),
        },
        packages,
        relationships,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trivy::{lang_pkg::Ecosystem, os_pkg::PackageManager};

    fn sample_os_pkg() -> OsPackage {
        OsPackage {
            name: "openssl".into(),
            version: "3.0.9-r0".into(),
            arch: Some("x86_64".into()),
            source_name: None,
            source_version: None,
            licenses: vec!["Apache-2.0".into()],
            maintainer: None,
            package_manager: PackageManager::Apk,
        }
    }

    fn sample_lang_pkg() -> LangPackage {
        LangPackage {
            name: "serde".into(),
            version: "1.0.193".into(),
            ecosystem: Ecosystem::Cargo,
            indirect: false,
            checksum: Some("sha256:deadbeef".into()),
            file_path: "Cargo.lock".into(),
        }
    }

    #[test]
    fn cyclonedx_structure() {
        let bom = generate_cyclonedx(
            "myimage:latest",
            &[sample_os_pkg()],
            &[sample_lang_pkg()],
        );
        assert_eq!(bom.bom_format, "CycloneDX");
        assert_eq!(bom.spec_version, "1.4");
        assert_eq!(bom.components.len(), 2);
        assert!(bom.components.iter().any(|c| c.name == "openssl"));
        assert!(bom.components.iter().any(|c| c.name == "serde"));
    }

    #[test]
    fn cyclonedx_purl() {
        let bom = generate_cyclonedx("img:v1", &[sample_os_pkg()], &[]);
        let c = &bom.components[0];
        assert!(c.purl.as_ref().unwrap().starts_with("pkg:apk"));
    }

    #[test]
    fn spdx_structure() {
        let doc = generate_spdx("myimage:latest", &[sample_os_pkg()], &[sample_lang_pkg()]);
        assert_eq!(doc.spdx_version, "SPDX-2.3");
        assert_eq!(doc.data_license, "CC0-1.0");
        assert_eq!(doc.packages.len(), 2);
    }

    #[test]
    fn spdx_serializes() {
        let doc = generate_spdx("img:v1", &[sample_os_pkg()], &[]);
        let json = serde_json::to_string_pretty(&doc).unwrap();
        assert!(json.contains("SPDX-2.3"));
        assert!(json.contains("openssl"));
    }
}
