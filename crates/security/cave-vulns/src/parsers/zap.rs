// SPDX-License-Identifier: AGPL-3.0-or-later
//! OWASP ZAP XML report parser.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/zap/parser.py
//!         (`class ZapParser`).
//!
//! Wire format: ZAP XML report — `<OWASPZAPReport><site>*<alerts>
//! <alertitem>+</alertitem></alerts></site></OWASPZAPReport>`.

use super::{ParserError, ScanParser};
use crate::finding::{Finding, FindingSeverity};

pub struct ZapParser;

// Source: ZapParser.MAPPING_SEVERITY in upstream parser.py:57
fn riskcode_to_severity(rc: &str) -> FindingSeverity {
    match rc {
        "0" => FindingSeverity::Info,
        "1" => FindingSeverity::Low,
        "2" => FindingSeverity::Medium,
        "3" => FindingSeverity::High,
        _ => FindingSeverity::Info,
    }
}

/// Minimal pull-style XML parser tailored to ZAP report shape.
/// We avoid pulling a full XML lib by walking `<alertitem>` tags.
fn extract_tag<'a>(s: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        if let Some(j) = after.find(&close) {
            out.push(&after[..j]);
            rest = &after[j + close.len()..];
        } else {
            break;
        }
    }
    out
}

fn xml_decode(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

impl ScanParser for ZapParser {
    fn scan_type(&self) -> &'static str {
        "ZAP Scan"
    }
    fn dedupe_fields(&self) -> &'static [&'static str] {
        // Source: ZapParser.get_dedupe_fields, upstream parser.py:42
        &["title", "cwe", "severity"]
    }
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        let text = std::str::from_utf8(data).map_err(|e| ParserError::Xml(e.to_string()))?;
        let mut out = Vec::new();
        for item in extract_tag(text, "alertitem") {
            let alert = extract_tag(item, "alert").first().copied().unwrap_or("");
            let desc = extract_tag(item, "desc").first().copied().unwrap_or("");
            let solution = extract_tag(item, "solution").first().copied().unwrap_or("");
            let reference = extract_tag(item, "reference")
                .first()
                .copied()
                .unwrap_or("");
            let riskcode = extract_tag(item, "riskcode")
                .first()
                .copied()
                .unwrap_or("0");
            let cweid = extract_tag(item, "cweid").first().copied().unwrap_or("");
            let pluginid = extract_tag(item, "pluginid").first().copied().unwrap_or("");

            let sev = riskcode_to_severity(riskcode);
            let mut f = Finding::new(xml_decode(alert), sev);
            f.description = xml_decode(desc);
            f.mitigation = Some(xml_decode(solution)).filter(|s| !s.is_empty());
            f.references = Some(xml_decode(reference)).filter(|s| !s.is_empty());
            if let Ok(n) = cweid.parse::<u32>() {
                f.cwe = Some(n);
            }
            f.vuln_id_from_tool = Some(pluginid.into());
            f.dynamic_finding = true;
            f.static_finding = false;
            f.found_by_scanner = Some("ZAP Scan".into());
            out.push(f);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<OWASPZAPReport version="2.14.0">
      <site name="https://app.example.com">
        <alerts>
          <alertitem>
            <pluginid>40012</pluginid>
            <alert>Cross Site Scripting (Reflected)</alert>
            <riskcode>3</riskcode>
            <desc>&lt;p&gt;XSS detected&lt;/p&gt;</desc>
            <solution>Encode HTML</solution>
            <reference>https://owasp.org/xss</reference>
            <cweid>79</cweid>
          </alertitem>
          <alertitem>
            <pluginid>10038</pluginid>
            <alert>Missing CSP Header</alert>
            <riskcode>1</riskcode>
            <desc>No CSP set</desc>
            <solution>Set Content-Security-Policy</solution>
            <reference></reference>
            <cweid>693</cweid>
          </alertitem>
        </alerts>
      </site>
    </OWASPZAPReport>"#;

    #[test]
    fn parses_two_alertitems() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn riskcode_3_maps_to_high() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].severity, FindingSeverity::High);
    }

    #[test]
    fn riskcode_1_maps_to_low() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert_eq!(out[1].severity, FindingSeverity::Low);
    }

    #[test]
    fn cwe_extracted_from_cweid() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].cwe, Some(79));
        assert_eq!(out[1].cwe, Some(693));
    }

    #[test]
    fn pluginid_lands_on_vuln_id_from_tool() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].vuln_id_from_tool.as_deref(), Some("40012"));
    }

    #[test]
    fn marks_as_dynamic_finding() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert!(out[0].dynamic_finding);
        assert!(!out[0].static_finding);
    }

    #[test]
    fn decodes_html_entities_in_desc() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert!(out[0].description.contains("<p>"));
    }

    #[test]
    fn empty_reference_yields_none() {
        let out = ZapParser.parse(SAMPLE).unwrap();
        assert!(out[1].references.is_none());
    }

    #[test]
    fn empty_xml_returns_empty() {
        let out = ZapParser
            .parse(b"<OWASPZAPReport></OWASPZAPReport>")
            .unwrap();
        assert!(out.is_empty());
    }
}
