// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/ascanrules/.../PathTraversalScanRule.java
//
//! Path traversal probe — inject `../../etc/passwd` style payloads and
//! match against file-system fingerprints. Mirrors ZAP plugin id 6.

use super::{ActiveScanRule, PluginId, Probe};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct PathTraversalRule;

/// File fingerprints — substrings present in well-known sensitive files.
const UNIX_PASSWD_HINT: &str = "root:x:0:0:";
const UNIX_SHADOW_HINT: &str = "root:$";
const WIN_BOOT_HINT: &str = "[boot loader]";
const WIN_HOSTS_HINT: &str = "127.0.0.1 localhost";

const PAYLOADS: &[&str] = &[
    "../../../../etc/passwd",
    "..\\..\\..\\..\\windows\\system.ini",
    "/etc/passwd",
    "C:\\windows\\system.ini",
    "....//....//....//etc/passwd",
];

impl ActiveScanRule for PathTraversalRule {
    fn id(&self) -> PluginId {
        6
    }
    fn name(&self) -> &'static str {
        "Path Traversal"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn cwe_id(&self) -> u32 {
        22
    }
    fn wasc_id(&self) -> u32 {
        33
    }

    fn probes(&self, req: &HttpRequest) -> Vec<Probe> {
        let mut probes = Vec::new();
        for payload in PAYLOADS {
            for mutated in super::per_param_mutations(req, payload) {
                probes.push(Probe {
                    original: req.clone(),
                    mutated,
                    plugin_id: self.id(),
                    note: (*payload).to_string(),
                });
            }
        }
        probes
    }

    fn check(&self, probe: &Probe, response: &HttpResponse) -> Option<Alert> {
        let body = response.body_str().unwrap_or("");
        for fp in [
            UNIX_PASSWD_HINT,
            UNIX_SHADOW_HINT,
            WIN_BOOT_HINT,
            WIN_HOSTS_HINT,
        ] {
            if body.contains(fp) {
                return Some(Alert {
                    name: self.name().to_string(),
                    risk: self.risk(),
                    cwe_id: self.cwe_id(),
                    url: probe.mutated.url.clone(),
                    description: format!(
                        "Response body contains '{}' after path-traversal payload was sent.",
                        fp
                    ),
                    solution:
                        "Canonicalise paths server-side and verify they start with an allowed directory."
                            .to_string(),
                    evidence: Some(fp.to_string()),
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

    #[test]
    fn rule_metadata() {
        let r = PathTraversalRule;
        assert_eq!(r.id(), 6);
        assert_eq!(r.cwe_id(), 22);
    }

    #[test]
    fn probes_payload_count() {
        let r = PathTraversalRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/read?file=a");
        // 5 payloads × 1 param.
        assert_eq!(r.probes(&req).len(), 5);
    }

    #[test]
    fn detect_unix_passwd() {
        let r = PathTraversalRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/r?f=a"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/r?f=../etc/passwd"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.body = b"root:x:0:0:root:/root:/bin/bash".to_vec();
        assert!(r.check(&probe, &resp).is_some());
    }

    #[test]
    fn detect_windows_boot_ini() {
        let r = PathTraversalRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/r?f=a"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/r?f=c:\\boot.ini"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.body = b"[boot loader]\ntimeout=30".to_vec();
        assert!(r.check(&probe, &resp).is_some());
    }

    #[test]
    fn no_alert_on_normal_body() {
        let r = PathTraversalRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/r?f=a"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/r?f=../etc/passwd"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let mut resp = HttpResponse::new(404, "Not Found");
        resp.body = b"file not found".to_vec();
        assert!(r.check(&probe, &resp).is_none());
    }
}
