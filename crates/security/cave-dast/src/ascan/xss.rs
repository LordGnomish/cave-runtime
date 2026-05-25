// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/ascanrules/.../CrossSiteScriptingScanRule.java
//
//! Reflected XSS probe — injects a tagged payload into each parameter
//! and checks whether the payload appears unescaped in the response
//! body. Mirrors ZAP plugin id 40012.

use super::{ActiveScanRule, PluginId, Probe};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct ReflectedXssRule;

/// Canary payload — must reflect verbatim AND in an HTML-active form.
const PAYLOAD: &str = "<script>cave_xss_canary()</script>";
const PAYLOAD_NOQUOTE: &str = "<svg/onload=cave_xss_canary()>";

impl ActiveScanRule for ReflectedXssRule {
    fn id(&self) -> PluginId {
        40012
    }
    fn name(&self) -> &'static str {
        "Cross-Site Scripting (Reflected)"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn cwe_id(&self) -> u32 {
        79
    }
    fn wasc_id(&self) -> u32 {
        8
    }

    fn probes(&self, req: &HttpRequest) -> Vec<Probe> {
        let mut probes = Vec::new();
        for payload in [PAYLOAD, PAYLOAD_NOQUOTE] {
            for mutated in super::per_param_mutations(req, payload) {
                probes.push(Probe {
                    original: req.clone(),
                    mutated,
                    plugin_id: self.id(),
                    note: format!("payload={}", payload),
                });
            }
        }
        probes
    }

    fn check(&self, probe: &Probe, response: &HttpResponse) -> Option<Alert> {
        if !response.is_html() {
            return None; // Only meaningful in HTML context.
        }
        let body = response.body_str().unwrap_or("");
        // The payload appears verbatim — server didn't escape `<` / `>`.
        for payload in [PAYLOAD, PAYLOAD_NOQUOTE] {
            if body.contains(payload) {
                return Some(Alert {
                    name: self.name().to_string(),
                    risk: self.risk(),
                    cwe_id: self.cwe_id(),
                    url: probe.mutated.url.clone(),
                    description:
                        "Injected script tag was reflected verbatim in the response body."
                            .to_string(),
                    solution:
                        "HTML-encode user-controlled data before rendering. Consider a Content-Security-Policy with no 'unsafe-inline'."
                            .to_string(),
                    evidence: Some(payload.to_string()),
                    plugin_id: self.id(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    fn resp_html(body: &str) -> HttpResponse {
        let mut r = HttpResponse::new(200, "OK");
        r.headers.insert("Content-Type", "text/html");
        r.body = body.as_bytes().to_vec();
        r
    }

    #[test]
    fn rule_metadata() {
        let r = ReflectedXssRule;
        assert_eq!(r.id(), 40012);
        assert_eq!(r.cwe_id(), 79);
    }

    #[test]
    fn probes_two_payloads_per_param() {
        let r = ReflectedXssRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/q?term=hello");
        // 2 payloads × 1 param = 2 probes.
        assert_eq!(r.probes(&req).len(), 2);
    }

    #[test]
    fn detect_reflected_payload() {
        let r = ReflectedXssRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/q?t=x"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/q?t=PAYLOAD"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let resp = resp_html(&format!("hello {}", PAYLOAD));
        assert!(r.check(&probe, &resp).is_some());
    }

    #[test]
    fn skip_non_html_response() {
        let r = ReflectedXssRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/q?t=x"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/q?t=PAYLOAD"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.headers.insert("Content-Type", "application/json");
        resp.body = format!("{{\"q\":\"{}\"}}", PAYLOAD).into_bytes();
        assert!(r.check(&probe, &resp).is_none());
    }

    #[test]
    fn no_alert_when_escaped() {
        let r = ReflectedXssRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/q?t=x"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/q?t=PAYLOAD"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let resp = resp_html("hello &lt;script&gt;cave_xss_canary()&lt;/script&gt;");
        assert!(r.check(&probe, &resp).is_none());
    }
}
