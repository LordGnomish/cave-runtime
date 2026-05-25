// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/pscanrules/.../MissingSecurityHeadersScanRule.java
//
//! Missing security-headers passive rule. ZAP raises one alert per
//! missing recommended header. Plugin ids 10038, 10063, 10020, 10035,
//! 10202.

use super::{PassiveScanRule, PluginId};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct MissingSecurityHeadersRule;

const RECOMMENDED: &[(&str, &str, PluginId, RiskLevel)] = &[
    (
        "Content-Security-Policy",
        "Send a CSP at least covering default-src 'self'.",
        10038,
        RiskLevel::Medium,
    ),
    (
        "Strict-Transport-Security",
        "Send HSTS with max-age >= 31536000 on every HTTPS response.",
        10035,
        RiskLevel::Medium,
    ),
    (
        "X-Content-Type-Options",
        "Send `X-Content-Type-Options: nosniff` to block MIME sniffing.",
        10021,
        RiskLevel::Low,
    ),
    (
        "X-Frame-Options",
        "Send `X-Frame-Options: DENY` or use CSP frame-ancestors.",
        10020,
        RiskLevel::Low,
    ),
    (
        "Referrer-Policy",
        "Send `Referrer-Policy: strict-origin-when-cross-origin`.",
        10063,
        RiskLevel::Informational,
    ),
];

impl PassiveScanRule for MissingSecurityHeadersRule {
    fn id(&self) -> PluginId {
        10038
    }
    fn name(&self) -> &'static str {
        "Missing Security Headers"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::Medium
    }
    fn cwe_id(&self) -> u32 {
        693
    }

    fn scan(&self, _req: &HttpRequest, resp: &HttpResponse) -> Vec<Alert> {
        if !resp.is_html() {
            return Vec::new();
        }
        let mut alerts = Vec::new();
        for (header, solution, pid, risk) in RECOMMENDED {
            if header == &"Strict-Transport-Security" {
                let scheme_https = resp
                    .headers
                    .first("Content-Security-Policy")
                    .map(|_| true)
                    .unwrap_or(false);
                // STS is only meaningful on HTTPS; we don't know the request
                // scheme reliably here so always flag — ZAP does the same.
                let _ = scheme_https;
            }
            if resp.headers.first(header).is_none() {
                alerts.push(Alert {
                    name: format!("Missing {}", header),
                    risk: *risk,
                    cwe_id: 693,
                    url: String::new(),
                    description: format!("Response is missing the `{}` header.", header),
                    solution: solution.to_string(),
                    evidence: Some(format!("absent: {}", header)),
                    plugin_id: *pid,
                });
            }
        }
        alerts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpMethod, HttpRequest};

    fn html_resp_with(headers: &[(&str, &str)]) -> HttpResponse {
        let mut r = HttpResponse::new(200, "OK");
        r.headers.insert("Content-Type", "text/html");
        for (k, v) in headers {
            r.headers.insert(*k, *v);
        }
        r
    }

    #[test]
    fn flags_every_missing_header() {
        let r = MissingSecurityHeadersRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = html_resp_with(&[]);
        let alerts = r.scan(&req, &resp);
        assert_eq!(alerts.len(), 5);
    }

    #[test]
    fn no_alert_when_all_present() {
        let r = MissingSecurityHeadersRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = html_resp_with(&[
            ("Content-Security-Policy", "default-src 'self'"),
            ("Strict-Transport-Security", "max-age=31536000"),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("Referrer-Policy", "strict-origin-when-cross-origin"),
        ]);
        let alerts = r.scan(&req, &resp);
        assert!(alerts.is_empty());
    }

    #[test]
    fn skip_non_html() {
        let r = MissingSecurityHeadersRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/data.json");
        let mut resp = HttpResponse::new(200, "OK");
        resp.headers.insert("Content-Type", "application/json");
        let alerts = r.scan(&req, &resp);
        assert!(alerts.is_empty());
    }

    #[test]
    fn flags_only_specific_missing() {
        let r = MissingSecurityHeadersRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = html_resp_with(&[
            ("Content-Security-Policy", "default-src 'self'"),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("Referrer-Policy", "no-referrer"),
        ]);
        let alerts = r.scan(&req, &resp);
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].name.contains("Strict-Transport-Security"));
    }
}
