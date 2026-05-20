// SPDX-License-Identifier: AGPL-3.0-or-later
//! ProjectDiscovery Nuclei JSON parser.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/nuclei/parser.py
//!         (`class NucleiParser`).
//!
//! Wire format: line-delimited JSON OR JSON array. Each record carries
//! `templateID`, `info: { name, severity, description?, reference?, tags?,
//! classification?: { cve-id?, cwe-id?, cvss-metrics?, cvss-score? } }`,
//! `matched-at`, `host`, `type`, `matcher-name?`.

use super::{ParserError, ScanParser};
use crate::finding::{Finding, FindingSeverity};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub struct NucleiParser;

#[derive(Deserialize)]
struct NuItem {
    #[serde(default, rename = "templateID")]
    template_id: Option<String>,
    #[serde(default, rename = "template-id")]
    template_id_alt: Option<String>,
    info: NuInfo,
    #[serde(default, rename = "matched")]
    matched: Option<String>,
    #[serde(default, rename = "matched-at")]
    matched_at: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    host: Option<String>,
    #[serde(default, rename = "type")]
    item_type: Option<String>,
    #[serde(default, rename = "matcher-name")]
    matcher_name: Option<String>,
    #[serde(default)]
    request: Option<String>,
    #[serde(default)]
    response: Option<String>,
    #[serde(default, rename = "curl-command")]
    curl_command: Option<String>,
    #[serde(default, rename = "extracted-results")]
    extracted: Vec<String>,
}
#[derive(Deserialize)]
struct NuInfo {
    name: String,
    severity: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    reference: Value,
    #[serde(default)]
    #[allow(dead_code)]
    tags: Value,
    #[serde(default)]
    classification: Value,
    #[serde(default)]
    remediation: Option<String>,
}

impl ScanParser for NucleiParser {
    fn scan_type(&self) -> &'static str {
        "Nuclei Scan"
    }
    fn dedupe_fields(&self) -> &'static [&'static str] {
        &["title", "severity", "vuln_id_from_tool"]
    }
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        let text = std::str::from_utf8(data).map_err(|e| ParserError::Xml(e.to_string()))?;
        let trimmed = text.trim_start();
        let items: Vec<NuItem> = if trimmed.starts_with('[') {
            serde_json::from_str(trimmed)?
        } else {
            // JSONL — one record per line
            let mut acc: Vec<NuItem> = Vec::new();
            for line in trimmed.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                acc.push(serde_json::from_str(line)?);
            }
            acc
        };

        let mut dupes: HashMap<String, Finding> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for item in items {
            let template_id = item
                .template_id
                .or(item.template_id_alt)
                .unwrap_or_default();
            let sev = FindingSeverity::parse(&item.info.severity).unwrap_or(FindingSeverity::Low);
            let mut f = Finding::new(item.info.name.clone(), sev);
            f.vuln_id_from_tool = Some(template_id.clone());
            if let Some(d) = item.info.description {
                f.description = d;
            }
            if !item.extracted.is_empty() {
                f.description.push_str("\n**Results:**\n");
                f.description.push_str(&item.extracted.join("\n"));
            }
            // References can be string or array.
            match &item.info.reference {
                Value::Array(a) => {
                    let lines: Vec<String> = a
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    if !lines.is_empty() {
                        f.references = Some(lines.join("\n"));
                    }
                }
                Value::String(s) => f.references = Some(s.clone()),
                _ => {}
            }
            // Classification.
            if let Some(cve_ids) = item
                .info
                .classification
                .get("cve-id")
                .and_then(|v| v.as_array())
            {
                let ids: Vec<String> = cve_ids
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_uppercase()))
                    .collect();
                if let Some(first) = ids.first() {
                    f.cve = Some(first.clone());
                }
                f.vulnerability_ids = ids;
            }
            if let Some(cwe_arr) = item
                .info
                .classification
                .get("cwe-id")
                .and_then(|v| v.as_array())
            {
                if let Some(first) = cwe_arr.first().and_then(|v| v.as_str()) {
                    // Format like "cwe-79"
                    if let Some(num) = first
                        .to_ascii_lowercase()
                        .strip_prefix("cwe-")
                        .and_then(|x| x.parse().ok())
                    {
                        f.cwe = Some(num);
                    }
                }
            }
            if let Some(score) = item
                .info
                .classification
                .get("cvss-score")
                .and_then(|v| v.as_f64())
            {
                f.cvssv3_score = Some(score as f32);
            }
            if let Some(vector) = item
                .info
                .classification
                .get("cvss-metrics")
                .and_then(|v| v.as_str())
            {
                f.cvssv3 = Some(vector.into());
            }
            if let Some(cmd) = item.curl_command {
                f.steps_to_reproduce = Some(format!("curl command to reproduce:\n`{cmd}`"));
            }
            if let Some(r) = item.info.remediation {
                f.mitigation = Some(r);
            }
            if let Some(name) = item.matcher_name.clone() {
                f.component_name = Some(name);
            }
            f.dynamic_finding = true;
            f.found_by_scanner = Some("Nuclei Scan".into());
            let matched = item.matched.or(item.matched_at).unwrap_or_default();
            let dupe_host = matched
                .split("://")
                .nth(1)
                .unwrap_or(&matched)
                .split('/')
                .next()
                .unwrap_or(&matched)
                .to_string();
            let item_type = item.item_type.unwrap_or_default();
            let matcher = item.matcher_name.unwrap_or_default();
            let raw = format!("{template_id}{item_type}{matcher}{dupe_host}");
            let mut h = Sha256::new();
            h.update(raw.as_bytes());
            let dupe_key = format!("{:x}", h.finalize());

            // Stash request/response for context.
            if let Some(req) = item.request {
                f.description.push_str("\n**Request:**\n```\n");
                f.description.push_str(&req);
                f.description.push_str("\n```");
            }
            if let Some(resp) = item.response {
                f.description.push_str("\n**Response:**\n```\n");
                f.description.push_str(&resp);
                f.description.push_str("\n```");
            }
            match dupes.get_mut(&dupe_key) {
                Some(existing) => {
                    existing.nb_occurences += 1;
                }
                None => {
                    order.push(dupe_key.clone());
                    dupes.insert(dupe_key, f);
                }
            }
        }
        Ok(order.into_iter().filter_map(|k| dupes.remove(&k)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_ARRAY: &[u8] = br#"[
      {"templateID":"CVE-2021-44228",
       "info":{"name":"Log4Shell","severity":"critical",
               "description":"RCE in log4j",
               "reference":["https://logging.apache.org"],
               "tags":["cve","rce"],
               "classification":{"cve-id":["cve-2021-44228"],"cwe-id":["cwe-502"],
                                  "cvss-metrics":"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H",
                                  "cvss-score":10.0}},
       "matched-at":"https://app.example.com/login","host":"app.example.com","type":"http",
       "matcher-name":"word-match","curl-command":"curl https://app.example.com/login"},
      {"templateID":"info-disclosure",
       "info":{"name":"Backup file","severity":"medium"},
       "matched-at":"https://app.example.com/backup.zip","host":"app.example.com","type":"http"}
    ]"#;

    #[test]
    fn parses_json_array() {
        let out = NucleiParser.parse(SAMPLE_ARRAY).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn severity_critical_parsed() {
        let out = NucleiParser.parse(SAMPLE_ARRAY).unwrap();
        assert_eq!(out[0].severity, FindingSeverity::Critical);
        assert_eq!(out[1].severity, FindingSeverity::Medium);
    }

    #[test]
    fn classification_carries_cve_and_cwe() {
        let out = NucleiParser.parse(SAMPLE_ARRAY).unwrap();
        assert_eq!(out[0].cve.as_deref(), Some("CVE-2021-44228"));
        assert_eq!(out[0].cwe, Some(502));
    }

    #[test]
    fn cvss_vector_and_score_extracted() {
        let out = NucleiParser.parse(SAMPLE_ARRAY).unwrap();
        assert_eq!(out[0].cvssv3_score, Some(10.0));
        assert!(out[0].cvssv3.as_deref().unwrap().starts_with("CVSS:3.1"));
    }

    #[test]
    fn curl_command_recorded_in_steps() {
        let out = NucleiParser.parse(SAMPLE_ARRAY).unwrap();
        assert!(
            out[0]
                .steps_to_reproduce
                .as_deref()
                .unwrap()
                .contains("curl")
        );
    }

    #[test]
    fn vuln_id_from_tool_is_template_id() {
        let out = NucleiParser.parse(SAMPLE_ARRAY).unwrap();
        assert_eq!(out[0].vuln_id_from_tool.as_deref(), Some("CVE-2021-44228"));
    }

    #[test]
    fn marks_dynamic_finding() {
        let out = NucleiParser.parse(SAMPLE_ARRAY).unwrap();
        assert!(out[0].dynamic_finding);
    }

    #[test]
    fn parses_jsonl_format() {
        let jsonl = b"{\"templateID\":\"x\",\"info\":{\"name\":\"a\",\"severity\":\"low\"},\"matched-at\":\"http://a/\",\"host\":\"a\",\"type\":\"http\"}\n{\"templateID\":\"y\",\"info\":{\"name\":\"b\",\"severity\":\"high\"},\"matched-at\":\"http://b/\",\"host\":\"b\",\"type\":\"http\"}\n";
        let out = NucleiParser.parse(jsonl).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedup_collapses_same_template_same_host() {
        let s = br#"[
            {"templateID":"x","info":{"name":"a","severity":"low"},"matched-at":"http://a/p1","host":"a","type":"http"},
            {"templateID":"x","info":{"name":"a","severity":"low"},"matched-at":"http://a/p2","host":"a","type":"http"}
        ]"#;
        let out = NucleiParser.parse(s).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].nb_occurences, 2);
    }
}
