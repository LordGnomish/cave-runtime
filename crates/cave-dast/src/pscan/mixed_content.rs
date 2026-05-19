// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/pscanrules/.../MixedContentScanRule.java
//
//! Mixed-content passive rule. Flags an HTTPS page that references
//! `http://...` resources (img, script, link, iframe).

use super::{PassiveScanRule, PluginId};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct MixedContentRule;

impl PassiveScanRule for MixedContentRule {
    fn id(&self) -> PluginId {
        10040
    }
    fn name(&self) -> &'static str {
        "Mixed Content"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::Medium
    }
    fn cwe_id(&self) -> u32 {
        311
    }

    fn scan(&self, req: &HttpRequest, resp: &HttpResponse) -> Vec<Alert> {
        if !resp.is_html() {
            return Vec::new();
        }
        let is_https = req.url.starts_with("https://");
        if !is_https {
            return Vec::new();
        }
        let body = resp.body_str().unwrap_or("");
        let mut findings = Vec::new();
        for marker in [
            "src=\"http://",
            "src='http://",
            "href=\"http://",
            "href='http://",
        ] {
            if let Some(_pos) = body.find(marker) {
                findings.push(marker);
            }
        }
        if findings.is_empty() {
            return Vec::new();
        }
        vec![Alert {
            name: self.name().to_string(),
            risk: self.risk(),
            cwe_id: self.cwe_id(),
            url: req.url.clone(),
            description: format!(
                "HTTPS page references {} plaintext-HTTP resource(s).",
                findings.len()
            ),
            solution:
                "Use protocol-relative or HTTPS-only URLs for every embedded resource. Consider a `Content-Security-Policy: upgrade-insecure-requests`."
                    .to_string(),
            evidence: Some(findings.join(" + ")),
            plugin_id: self.id(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    fn html_with_body(body: &str) -> HttpResponse {
        let mut r = HttpResponse::new(200, "OK");
        r.headers.insert("Content-Type", "text/html");
        r.body = body.as_bytes().to_vec();
        r
    }

    #[test]
    fn flags_http_script_on_https_page() {
        let r = MixedContentRule;
        let req = HttpRequest::new(HttpMethod::Get, "https://x.test/");
        let resp = html_with_body("<script src=\"http://cdn.example/a.js\"></script>");
        assert_eq!(r.scan(&req, &resp).len(), 1);
    }

    #[test]
    fn no_alert_on_http_page() {
        let r = MixedContentRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x.test/");
        let resp = html_with_body("<script src=\"http://cdn.example/a.js\"></script>");
        assert!(r.scan(&req, &resp).is_empty());
    }

    #[test]
    fn no_alert_when_only_https_refs() {
        let r = MixedContentRule;
        let req = HttpRequest::new(HttpMethod::Get, "https://x.test/");
        let resp = html_with_body("<link href=\"https://cdn.example/a.css\">");
        assert!(r.scan(&req, &resp).is_empty());
    }
}
