// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/ascan/AbstractPlugin.java
//
//! Active scan plugin framework.
//!
//! An active scan rule receives an HTTP request and (optionally) the
//! captured response, optionally mutates the request to probe for a
//! vulnerability, and returns zero or more `Alert`s. The framework
//! itself is HTTP-transport agnostic: rules describe the probe by
//! mutating `HttpRequest`; an outer driver is responsible for issuing
//! the request and evaluating the response.
//!
//! This mirrors ZAP's `AbstractPlugin` shape — `getId`, `getName`,
//! `getRisk`, `getCweId`, `scan(HttpMessage, ...)`.

pub mod command_injection;
pub mod path_traversal;
pub mod sqli;
pub mod ssrf;
pub mod xss;
pub mod xxe;

use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

/// ZAP plugin identifier — stable u32 IDs matching ZAP's add-on
/// taxonomy (40018, 40012, …). Keeping them stable lets a Cave-side
/// rule set ride the same dashboards a ZAP shop already uses.
pub type PluginId = u32;

/// A scan probe — the request the rule wants the driver to send, plus
/// the original to compare responses against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub original: HttpRequest,
    pub mutated: HttpRequest,
    pub plugin_id: PluginId,
    pub note: String,
}

/// The trait every active scan rule implements.
pub trait ActiveScanRule: Send + Sync {
    fn id(&self) -> PluginId;
    fn name(&self) -> &'static str;
    fn risk(&self) -> RiskLevel;
    fn cwe_id(&self) -> u32;
    fn wasc_id(&self) -> u32;
    /// Generate probes for a request. Each probe is one mutated request.
    fn probes(&self, req: &HttpRequest) -> Vec<Probe>;
    /// Decide whether a probe's response indicates the vulnerability.
    fn check(&self, probe: &Probe, response: &HttpResponse) -> Option<Alert>;
}

/// Registry of available rules. ZAP's `PluginFactory`.
pub struct ScanPluginRegistry {
    plugins: Vec<Box<dyn ActiveScanRule>>,
}

impl Default for ScanPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanPluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register every baseline rule shipped with cave-dast.
    pub fn with_baseline() -> Self {
        let mut r = Self::new();
        r.register(Box::new(sqli::SqlInjectionRule));
        r.register(Box::new(xss::ReflectedXssRule));
        r.register(Box::new(path_traversal::PathTraversalRule));
        r.register(Box::new(command_injection::CommandInjectionRule));
        r.register(Box::new(xxe::XxeRule));
        r.register(Box::new(ssrf::SsrfRule));
        r
    }

    pub fn register(&mut self, rule: Box<dyn ActiveScanRule>) {
        self.plugins.push(rule);
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn ActiveScanRule> {
        self.plugins.iter().map(|b| b.as_ref())
    }

    /// Drive every registered rule against a single base request, using
    /// the supplied driver closure to issue requests. Returns every
    /// alert raised across all probes.
    pub fn run<F>(&self, req: &HttpRequest, mut driver: F) -> Vec<Alert>
    where
        F: FnMut(&HttpRequest) -> HttpResponse,
    {
        let mut alerts = Vec::new();
        for plugin in &self.plugins {
            for probe in plugin.probes(req) {
                let resp = driver(&probe.mutated);
                if let Some(a) = plugin.check(&probe, &resp) {
                    alerts.push(a);
                }
            }
        }
        alerts
    }
}

/// Mutate the value of a single query parameter, returning a new URL.
pub(crate) fn mutate_query_param(url: &str, key: &str, value: &str) -> String {
    use crate::http::url;
    let Some(mut parsed) = url::parse(url) else {
        return url.to_string();
    };
    let mut params = crate::http::parse_query(&parsed.query);
    params.insert(key.to_string(), value.to_string());
    parsed.query = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");
    parsed.render()
}

/// Mutate every query parameter to the same payload, returning a clone
/// for each. Used by rules that want to probe each parameter slot.
pub(crate) fn per_param_mutations(req: &HttpRequest, payload: &str) -> Vec<HttpRequest> {
    let params = req.query_params();
    let mut out = Vec::with_capacity(params.len());
    for k in params.keys() {
        let mut r = req.clone();
        r.url = mutate_query_param(&req.url, k, payload);
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    #[test]
    fn baseline_registry_count() {
        let r = ScanPluginRegistry::with_baseline();
        assert_eq!(r.len(), 6);
    }

    #[test]
    fn plugin_ids_unique() {
        let r = ScanPluginRegistry::with_baseline();
        let mut ids: Vec<_> = r.iter().map(|p| p.id()).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "plugin IDs must be unique");
    }

    #[test]
    fn mutate_query_param_inserts() {
        let out = mutate_query_param("http://x/api?a=1&b=2", "a", "PAYLOAD");
        assert!(out.contains("a=PAYLOAD"));
        assert!(out.contains("b=2"));
    }

    #[test]
    fn mutate_query_param_adds_if_missing() {
        let out = mutate_query_param("http://x/api?a=1", "c", "z");
        assert!(out.contains("c=z"));
    }

    #[test]
    fn per_param_mutations_count() {
        let req = HttpRequest::new(HttpMethod::Get, "http://x/api?a=1&b=2&c=3");
        let muts = per_param_mutations(&req, "P");
        assert_eq!(muts.len(), 3);
    }
}
