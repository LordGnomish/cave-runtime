// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/pscanrules/.../CookieSecureFlagScanRule.java
//         InsecureCookieScanRule.java + CookieHttpOnlyScanRule.java
//
//! Insecure-cookie passive rule. Flags Set-Cookie responses missing
//! the `Secure` or `HttpOnly` attributes, or that lack `SameSite`.

use super::{PassiveScanRule, PluginId};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct InsecureCookieRule;

impl PassiveScanRule for InsecureCookieRule {
    fn id(&self) -> PluginId {
        10011
    }
    fn name(&self) -> &'static str {
        "Insecure Cookie"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::Medium
    }
    fn cwe_id(&self) -> u32 {
        614
    }

    fn scan(&self, _req: &HttpRequest, resp: &HttpResponse) -> Vec<Alert> {
        let mut alerts = Vec::new();
        for cookie in resp.set_cookies() {
            if !cookie.secure {
                alerts.push(Alert {
                    name: format!("Cookie without Secure: {}", cookie.name),
                    risk: RiskLevel::Medium,
                    cwe_id: 614,
                    url: String::new(),
                    description: format!(
                        "Cookie '{}' was set without the Secure attribute — it will be sent over plaintext HTTP.",
                        cookie.name
                    ),
                    solution: "Add `Secure` to every Set-Cookie response over HTTPS.".to_string(),
                    evidence: Some(format!("{}=...", cookie.name)),
                    plugin_id: 10011,
                });
            }
            if !cookie.http_only {
                alerts.push(Alert {
                    name: format!("Cookie without HttpOnly: {}", cookie.name),
                    risk: RiskLevel::Low,
                    cwe_id: 1004,
                    url: String::new(),
                    description: format!(
                        "Cookie '{}' was set without the HttpOnly attribute — it is reachable from document.cookie.",
                        cookie.name
                    ),
                    solution: "Add `HttpOnly` to session cookies to mitigate XSS-driven theft.".to_string(),
                    evidence: Some(format!("{}=...", cookie.name)),
                    plugin_id: 10010,
                });
            }
            if cookie.same_site.is_none() {
                alerts.push(Alert {
                    name: format!("Cookie without SameSite: {}", cookie.name),
                    risk: RiskLevel::Informational,
                    cwe_id: 1275,
                    url: String::new(),
                    description: format!(
                        "Cookie '{}' was set without a SameSite attribute — browsers may infer different defaults.",
                        cookie.name
                    ),
                    solution: "Set `SameSite=Lax` (or `Strict` for sensitive cookies).".to_string(),
                    evidence: Some(format!("{}=...", cookie.name)),
                    plugin_id: 10054,
                });
            }
        }
        alerts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    fn resp_with(cookies: &[&str]) -> HttpResponse {
        let mut r = HttpResponse::new(200, "OK");
        for c in cookies {
            r.headers.insert("Set-Cookie", *c);
        }
        r
    }

    #[test]
    fn flags_all_three_attributes() {
        let r = InsecureCookieRule;
        let req = HttpRequest::new(HttpMethod::Get, "https://x/");
        let resp = resp_with(&["sid=abc"]);
        let alerts = r.scan(&req, &resp);
        assert_eq!(alerts.len(), 3);
    }

    #[test]
    fn fully_hardened_cookie_no_alert() {
        let r = InsecureCookieRule;
        let req = HttpRequest::new(HttpMethod::Get, "https://x/");
        let resp = resp_with(&["sid=abc; Secure; HttpOnly; SameSite=Lax"]);
        assert!(r.scan(&req, &resp).is_empty());
    }

    #[test]
    fn flags_only_missing() {
        let r = InsecureCookieRule;
        let req = HttpRequest::new(HttpMethod::Get, "https://x/");
        let resp = resp_with(&["sid=abc; Secure; HttpOnly"]);
        let alerts = r.scan(&req, &resp);
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].name.contains("SameSite"));
    }
}
