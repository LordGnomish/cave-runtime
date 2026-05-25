// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/pscanrules/.../InformationDisclosureServerErrorScanRule.java
//         InformationDisclosureSuspiciousCommentsScanRule.java
//         InformationDisclosureInUrlScanRule.java
//
//! Information disclosure passive rule. Flags `Server:` /
//! `X-Powered-By:` banners and suspicious developer comments.

use super::{PassiveScanRule, PluginId};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct InfoDisclosureRule;

const SUSPICIOUS_COMMENT_FRAGMENTS: &[&str] = &[
    "TODO",
    "FIXME",
    "XXX",
    "DEBUG",
    "password",
    "secret",
    "api_key",
    "api-key",
    "private_key",
];

impl PassiveScanRule for InfoDisclosureRule {
    fn id(&self) -> PluginId {
        10037
    }
    fn name(&self) -> &'static str {
        "Information Disclosure"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::Low
    }
    fn cwe_id(&self) -> u32 {
        200
    }

    fn scan(&self, _req: &HttpRequest, resp: &HttpResponse) -> Vec<Alert> {
        let mut alerts = Vec::new();
        if let Some(server) = resp.headers.first("Server") {
            if has_version(server) {
                alerts.push(Alert {
                    name: "Server Banner Disclosure".to_string(),
                    risk: RiskLevel::Informational,
                    cwe_id: 200,
                    url: String::new(),
                    description: format!("Response advertises server: {}", server),
                    solution: "Strip the Server header or drop the version suffix.".to_string(),
                    evidence: Some(server.to_string()),
                    plugin_id: 10036,
                });
            }
        }
        if let Some(xp) = resp.headers.first("X-Powered-By") {
            alerts.push(Alert {
                name: "X-Powered-By Disclosure".to_string(),
                risk: RiskLevel::Informational,
                cwe_id: 200,
                url: String::new(),
                description: format!("Response advertises framework: {}", xp),
                solution: "Remove the X-Powered-By header.".to_string(),
                evidence: Some(xp.to_string()),
                plugin_id: 10037,
            });
        }
        // Suspicious comments — only meaningful in HTML/JS responses.
        let body = resp.body_str().unwrap_or("");
        let scan_text = body.contains("<!--") || body.contains("//");
        if scan_text {
            for frag in SUSPICIOUS_COMMENT_FRAGMENTS {
                if body_contains_comment_fragment(body, frag) {
                    alerts.push(Alert {
                        name: "Suspicious Comment".to_string(),
                        risk: RiskLevel::Informational,
                        cwe_id: 200,
                        url: String::new(),
                        description: format!(
                            "Response body contains a developer comment that mentions '{}'.",
                            frag
                        ),
                        solution: "Strip developer comments at deploy time (minifier, build step)."
                            .to_string(),
                        evidence: Some((*frag).to_string()),
                        plugin_id: 10027,
                    });
                }
            }
        }
        alerts
    }
}

fn has_version(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit()) && s.contains(['/', '.'])
}

/// Cheap "is this fragment inside a comment-like span?" check. We look
/// for the fragment AND a `<!--` / `//` / `/*` marker on the same line.
fn body_contains_comment_fragment(body: &str, frag: &str) -> bool {
    if !body.contains(frag) {
        return false;
    }
    for line in body.lines() {
        if line.contains(frag)
            && (line.contains("<!--") || line.contains("//") || line.contains("/*"))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    #[test]
    fn server_version_flagged() {
        let r = InfoDisclosureRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let mut resp = HttpResponse::new(200, "OK");
        resp.headers.insert("Server", "Apache/2.4.41 (Ubuntu)");
        let alerts = r.scan(&req, &resp);
        assert!(alerts.iter().any(|a| a.name.contains("Server Banner")));
    }

    #[test]
    fn server_no_version_skipped() {
        let r = InfoDisclosureRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let mut resp = HttpResponse::new(200, "OK");
        resp.headers.insert("Server", "nginx");
        let alerts = r.scan(&req, &resp);
        assert!(!alerts.iter().any(|a| a.name.contains("Server Banner")));
    }

    #[test]
    fn x_powered_by_flagged() {
        let r = InfoDisclosureRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let mut resp = HttpResponse::new(200, "OK");
        resp.headers.insert("X-Powered-By", "Express");
        let alerts = r.scan(&req, &resp);
        assert!(alerts.iter().any(|a| a.name == "X-Powered-By Disclosure"));
    }

    #[test]
    fn suspicious_comment_in_html() {
        let r = InfoDisclosureRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let mut resp = HttpResponse::new(200, "OK");
        resp.body = b"<html><!-- TODO: fix password handling --></html>".to_vec();
        let alerts = r.scan(&req, &resp);
        assert!(alerts.iter().any(|a| a.name == "Suspicious Comment"));
    }

    #[test]
    fn no_alert_on_clean_response() {
        let r = InfoDisclosureRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = HttpResponse::new(200, "OK");
        assert!(r.scan(&req, &resp).is_empty());
    }
}
