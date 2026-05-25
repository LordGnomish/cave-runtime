// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/{cyclonedx,spdx}
//
//! SBOM ingest — CycloneDX (JSON+XML) and SPDX (JSON+tag-value) parsers.

pub mod cyclonedx;
pub mod spdx;
pub mod spdx_expression;
pub mod vex;

use crate::models::Component;

/// Outcome of any SBOM ingest. `format_detected` mirrors Dependency-Track's
/// `BomValidator.detectFormat` return value.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestResult {
    pub format_detected: BomFormat,
    pub spec_version: Option<String>,
    pub serial_number: Option<String>,
    pub project_name: Option<String>,
    pub project_version: Option<String>,
    pub components: Vec<Component>,
    pub dependencies: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BomFormat {
    CycloneDxJson,
    CycloneDxXml,
    SpdxJson,
    SpdxTagValue,
}

/// Quick sniff at the first non-whitespace byte. Mirrors
/// `BomValidator.detectFormat(byte[])`.
pub fn detect_format(input: &[u8]) -> Option<BomFormat> {
    let trimmed = input
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if trimmed.is_empty() {
        return None;
    }
    let head = std::str::from_utf8(&trimmed[..trimmed.len().min(256)]).unwrap_or("");
    if head.starts_with('<') {
        if head.contains("bom") && head.contains("cyclonedx") {
            Some(BomFormat::CycloneDxXml)
        } else if head.contains("bom") {
            // Generic <bom xmlns=...>: assume CycloneDX XML.
            Some(BomFormat::CycloneDxXml)
        } else {
            None
        }
    } else if head.starts_with('{') {
        // CycloneDX json includes "bomFormat":"CycloneDX".
        // SPDX json includes "spdxVersion":"SPDX-2...".
        if head.contains("\"bomFormat\"") || head.contains("CycloneDX") {
            Some(BomFormat::CycloneDxJson)
        } else if head.contains("spdxVersion") || head.contains("SPDXID") {
            Some(BomFormat::SpdxJson)
        } else {
            None
        }
    } else if head.contains("SPDXVersion") {
        Some(BomFormat::SpdxTagValue)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_cyclonedx_json() {
        let bom = br#"{"bomFormat":"CycloneDX","specVersion":"1.5"}"#;
        assert_eq!(detect_format(bom), Some(BomFormat::CycloneDxJson));
    }

    #[test]
    fn detect_cyclonedx_xml() {
        let bom =
            br#"<?xml version="1.0"?><bom xmlns="http://cyclonedx.org/schema/bom/1.5"></bom>"#;
        assert_eq!(detect_format(bom), Some(BomFormat::CycloneDxXml));
    }

    #[test]
    fn detect_spdx_json() {
        let bom = br#"{"spdxVersion":"SPDX-2.3","SPDXID":"SPDXRef-DOCUMENT"}"#;
        assert_eq!(detect_format(bom), Some(BomFormat::SpdxJson));
    }

    #[test]
    fn detect_spdx_tag_value() {
        let bom = b"SPDXVersion: SPDX-2.3\nDataLicense: CC0-1.0\n";
        assert_eq!(detect_format(bom), Some(BomFormat::SpdxTagValue));
    }

    #[test]
    fn detect_unknown() {
        assert_eq!(detect_format(b""), None);
        assert_eq!(detect_format(b"hello world"), None);
    }

    #[test]
    fn detect_skips_leading_whitespace() {
        let bom = b"   \n\t{\"bomFormat\":\"CycloneDX\"}";
        assert_eq!(detect_format(bom), Some(BomFormat::CycloneDxJson));
    }
}
