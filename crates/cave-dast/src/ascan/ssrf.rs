// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/ascanrules/.../SsrfScanRule.java
//
//! Server-Side Request Forgery probe — substitutes URL-typed parameters
//! with internal targets and looks for tell-tale internal responses.
//! Mirrors ZAP plugin id 40046.
//!
//! Partial port: the upstream rule also probes cloud-metadata endpoints
//! (`http://169.254.169.254/...` for AWS IMDSv1, GCP, Azure) and an
//! out-of-band HTTP callback. The IMDS / OOB channel is deferred — we
//! ship the in-body internal-leak path and a flagged 169.254 probe set.

use super::{ActiveScanRule, PluginId, Probe};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct SsrfRule;

const INTERNAL_PAYLOADS: &[&str] = &[
    "http://127.0.0.1/",
    "http://localhost/",
    "http://169.254.169.254/latest/meta-data/", // AWS IMDSv1 — flagged below.
    "http://[::1]/",
    "file:///etc/passwd",
];

const INTERNAL_FINGERPRINTS: &[&str] = &[
    "ami-id",          // AWS IMDS prefix.
    "instance-id",     // AWS IMDS.
    "Metadata-Flavor", // GCP IMDS header echo.
    "root:x:0:0:",     // file:///etc/passwd leak.
    "Apache/",         // internal http server banner often unfiltered.
    "nginx/",
];

impl ActiveScanRule for SsrfRule {
    fn id(&self) -> PluginId {
        40046
    }
    fn name(&self) -> &'static str {
        "Server-Side Request Forgery (SSRF)"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn cwe_id(&self) -> u32 {
        918
    }
    fn wasc_id(&self) -> u32 {
        20
    }

    fn probes(&self, req: &HttpRequest) -> Vec<Probe> {
        // Only probe params whose value looks URL-shaped to keep the
        // request budget bounded. ZAP applies the same heuristic.
        let mut probes = Vec::new();
        let params = req.query_params();
        for (k, v) in &params {
            if !looks_url_shaped(v) {
                continue;
            }
            for payload in INTERNAL_PAYLOADS {
                let mut mutated = req.clone();
                mutated.url = super::mutate_query_param(&req.url, k, payload);
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
        for fp in INTERNAL_FINGERPRINTS {
            if body.contains(fp) {
                return Some(Alert {
                    name: self.name().to_string(),
                    risk: self.risk(),
                    cwe_id: self.cwe_id(),
                    url: probe.mutated.url.clone(),
                    description: format!(
                        "Internal response fingerprint '{}' returned after redirecting URL parameter to an internal target.",
                        fp
                    ),
                    solution:
                        "Validate the URL against an allow-list of external hosts. Deny RFC 1918, link-local, and metadata IP ranges."
                            .to_string(),
                    evidence: Some(fp.to_string()),
                    plugin_id: self.id(),
                });
            }
        }
        None
    }
}

fn looks_url_shaped(v: &str) -> bool {
    v.starts_with("http://") || v.starts_with("https://") || v.starts_with("//")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    #[test]
    fn rule_metadata() {
        let r = SsrfRule;
        assert_eq!(r.id(), 40046);
        assert_eq!(r.cwe_id(), 918);
    }

    #[test]
    fn looks_url_shaped_check() {
        assert!(looks_url_shaped("http://x"));
        assert!(looks_url_shaped("https://x/y"));
        assert!(looks_url_shaped("//x"));
        assert!(!looks_url_shaped("plain"));
    }

    #[test]
    fn probes_only_url_shaped_params() {
        let r = SsrfRule;
        let req = HttpRequest::new(
            HttpMethod::Get,
            "http://x/fetch?dest=http%3A%2F%2Fy&plain=x",
        );
        let probes = r.probes(&req);
        // Only `dest` is url-shaped after decode. 5 payloads × 1 param.
        assert_eq!(probes.len(), 5);
    }

    #[test]
    fn detect_imds_leak() {
        let r = SsrfRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/f?dest=http://y"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/f?dest=http://169.254.169.254/"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.body = b"ami-id\nami-launch-index\ninstance-id".to_vec();
        assert!(r.check(&probe, &resp).is_some());
    }

    #[test]
    fn no_alert_on_clean() {
        let r = SsrfRule;
        let probe = Probe {
            original: HttpRequest::new(HttpMethod::Get, "http://x/f?dest=http://y"),
            mutated: HttpRequest::new(HttpMethod::Get, "http://x/f?dest=http://127.0.0.1/"),
            plugin_id: r.id(),
            note: "p".to_string(),
        };
        let mut resp = HttpResponse::new(400, "Bad Request");
        resp.body = b"invalid host".to_vec();
        assert!(r.check(&probe, &resp).is_none());
    }
}
