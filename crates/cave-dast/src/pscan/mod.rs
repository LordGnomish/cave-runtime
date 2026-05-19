// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/pscanrules/.../PluginPassiveScanner.java
//
//! Passive scan plugin framework. Each rule sees a request/response
//! pair the spider or proxy already captured — no extra probes are
//! issued. Rules return zero or more `Alert`s.

pub mod csrf_token;
pub mod info_disclosure;
pub mod insecure_cookie;
pub mod mixed_content;
pub mod security_headers;

use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub type PluginId = u32;

pub trait PassiveScanRule: Send + Sync {
    fn id(&self) -> PluginId;
    fn name(&self) -> &'static str;
    fn risk(&self) -> RiskLevel;
    fn cwe_id(&self) -> u32;
    fn scan(&self, req: &HttpRequest, resp: &HttpResponse) -> Vec<Alert>;
}

pub struct PassiveScanRegistry {
    rules: Vec<Box<dyn PassiveScanRule>>,
}

impl Default for PassiveScanRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PassiveScanRegistry {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn with_baseline() -> Self {
        let mut r = Self::new();
        r.register(Box::new(security_headers::MissingSecurityHeadersRule));
        r.register(Box::new(insecure_cookie::InsecureCookieRule));
        r.register(Box::new(info_disclosure::InfoDisclosureRule));
        r.register(Box::new(mixed_content::MixedContentRule));
        r.register(Box::new(csrf_token::CsrfTokenAbsenceRule));
        r
    }

    pub fn register(&mut self, rule: Box<dyn PassiveScanRule>) {
        self.rules.push(rule);
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn PassiveScanRule> {
        self.rules.iter().map(|b| b.as_ref())
    }

    pub fn run(&self, req: &HttpRequest, resp: &HttpResponse) -> Vec<Alert> {
        let mut alerts = Vec::new();
        for rule in &self.rules {
            alerts.extend(rule.scan(req, resp));
        }
        alerts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_registry_count() {
        let r = PassiveScanRegistry::with_baseline();
        assert_eq!(r.len(), 5);
    }

    #[test]
    fn rule_ids_unique() {
        let r = PassiveScanRegistry::with_baseline();
        let mut ids: Vec<_> = r.iter().map(|r| r.id()).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
