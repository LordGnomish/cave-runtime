// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/ascanrules/.../CommandInjectionScanRule.java
//
//! OS command injection probe — inject shell meta-payloads and look for
//! command-output fingerprints. Mirrors ZAP plugin id 90020.

use super::{ActiveScanRule, PluginId, Probe};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct CommandInjectionRule;

const PAYLOADS: &[&str] = &[
    "; id",
    "| id",
    "&& id",
    "`id`",
    "$(id)",
    "; cat /etc/passwd",
    "| whoami",
    "& whoami",
];

const OUTPUT_FINGERPRINTS: &[&str] = &[
    "uid=",
    "gid=",
    "root:x:0:0:",
    "bin:x:1:1:",
    "/etc/passwd",
    "Microsoft Windows",
];

impl ActiveScanRule for CommandInjectionRule {
    fn id(&self) -> PluginId {
        90020
    }
    fn name(&self) -> &'static str {
        "Remote OS Command Injection"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn cwe_id(&self) -> u32 {
        78
    }
    fn wasc_id(&self) -> u32 {
        31
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
        for fp in OUTPUT_FINGERPRINTS {
            if body.contains(fp) {
                return Some(Alert {
                    name: self.name().to_string(),
                    risk: self.risk(),
                    cwe_id: self.cwe_id(),
                    url: probe.mutated.url.clone(),
                    description: format!(
                        "Response contains '{}' after injecting shell metacharacters.",
                        fp
                    ),
                    solution:
                        "Avoid shelling out to user input. If unavoidable, use a hardened argv-only API and an allow-list."
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
        let r = CommandInjectionRule;
        assert_eq!(r.id(), 90020);
        assert_eq!(r.cwe_id(), 78);
    }

    #[test]
    fn probes_payload_count() {
        let r = CommandInjectionRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/ping?host=a");
        // 8 payloads × 1 param.
        assert_eq!(r.probes(&req).len(), 8);
    }

    #[test]
    fn detect_uid_output() {
        let r = CommandInjectionRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/p?h=a"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/p?h=a;id"),
            plugin_id: r.id(),
            note: "; id".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.body = b"uid=33(www-data) gid=33(www-data) groups=33(www-data)".to_vec();
        assert!(r.check(&probe, &resp).is_some());
    }

    #[test]
    fn no_alert_on_clean_response() {
        let r = CommandInjectionRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/p?h=a"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/p?h=a;id"),
            plugin_id: r.id(),
            note: "; id".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.body = b"PING a (1.2.3.4) 56 data bytes".to_vec();
        assert!(r.check(&probe, &resp).is_none());
    }
}
