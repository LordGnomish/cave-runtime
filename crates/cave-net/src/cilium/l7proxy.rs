// SPDX-License-Identifier: AGPL-3.0-or-later
//! L7 proxy redirect — the in-cave equivalent of Cilium's Envoy proxy hook.
//!
//! Mirrors `pkg/proxy/proxy.go` plus the per-protocol redirect modules
//! (`pkg/proxy/envoy/`, `pkg/proxy/dns/`).
//!
//! For each L4-allowed connection that has an L7 rule, the dataplane
//! redirects to the local L7 proxy on a per-pod port. The proxy:
//!
//! 1. Terminates mTLS (peer cert must be a valid SPIFFE id and start with
//!    the workload's own trust-domain prefix).
//! 2. Runs the configured filter chain in order. The first filter that
//!    returns `FilterDecision::Redirect` short-circuits the chain.
//! 3. Emits a [`RedirectVerdict`] describing whether to forward the request
//!    upstream, drop it, or rewrite it (e.g. DNS lookup → resolved IP set).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum L7ProxyError {
    #[error("peer SPIFFE id {peer} is not in trust domain {trust_domain}")]
    UntrustedPeer { peer: String, trust_domain: String },
    #[error("peer mTLS not present (proxy is configured strict)")]
    NoPeerCert,
    #[error("tenant {tenant} cannot drive a proxy owned by another tenant")]
    TenantDenied { tenant: TenantId },
    #[error("filter chain is empty")]
    NoFilters,
}

/// Per-filter decision. Mirrors Envoy's `FilterStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterDecision {
    /// Continue to the next filter.
    Continue,
    /// Stop the chain and forward the request upstream.
    Forward,
    /// Stop the chain and reject with the given HTTP code.
    Drop(u16),
    /// Stop the chain and rewrite the upstream target host.
    Redirect { upstream: String },
}

/// One filter in the chain.
pub trait L7Filter: std::fmt::Debug {
    fn name(&self) -> &str;
    fn evaluate(&self, req: &mut ProxyRequest) -> FilterDecision;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyRequest {
    pub method: String,
    pub host: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    /// Set by the DNS filter when it rewrites a hostname to an IP.
    pub resolved_upstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedirectVerdict {
    /// Forward the (possibly modified) request to `upstream`.
    Forward { upstream: String },
    /// Drop the request with `http_code`.
    Drop { http_code: u16, reason: String },
}

/// L7 proxy configuration.
pub struct L7Proxy {
    pub tenant: TenantId,
    /// SPIFFE prefix peers must present for mTLS to succeed.
    pub trust_domain: String,
    /// Whether to require a peer cert (`true`) or pass-through (`false`).
    pub strict_mtls: bool,
    /// Filter chain, evaluated in order.
    pub filters: Vec<Box<dyn L7Filter + Send + Sync>>,
}

impl std::fmt::Debug for L7Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.filters.iter().map(|f| f.name()).collect();
        f.debug_struct("L7Proxy")
            .field("tenant", &self.tenant)
            .field("trust_domain", &self.trust_domain)
            .field("strict_mtls", &self.strict_mtls)
            .field("filters", &names)
            .finish()
    }
}

impl L7Proxy {
    pub fn new(tenant: TenantId, trust_domain: impl Into<String>) -> Self {
        Self {
            tenant,
            trust_domain: trust_domain.into(),
            strict_mtls: true,
            filters: Vec::new(),
        }
    }

    pub fn add_filter(&mut self, f: Box<dyn L7Filter + Send + Sync>) {
        self.filters.push(f);
    }

    /// One full request pass: terminate mTLS, then run the filter chain.
    pub fn handle(
        &self,
        tenant: &TenantId,
        peer_spiffe_id: Option<&str>,
        mut req: ProxyRequest,
    ) -> Result<RedirectVerdict, L7ProxyError> {
        if &self.tenant != tenant {
            return Err(L7ProxyError::TenantDenied { tenant: tenant.clone() });
        }
        // mTLS termination.
        match peer_spiffe_id {
            None if self.strict_mtls => return Err(L7ProxyError::NoPeerCert),
            None => {}
            Some(peer) => {
                let prefix = format!("spiffe://{}/", self.trust_domain);
                if !peer.starts_with(&prefix) {
                    return Err(L7ProxyError::UntrustedPeer {
                        peer: peer.into(),
                        trust_domain: self.trust_domain.clone(),
                    });
                }
            }
        }
        if self.filters.is_empty() {
            return Err(L7ProxyError::NoFilters);
        }
        // Run filters in order; first non-Continue wins.
        for f in &self.filters {
            match f.evaluate(&mut req) {
                FilterDecision::Continue => continue,
                FilterDecision::Forward => {
                    let upstream = req.resolved_upstream.unwrap_or_else(|| req.host.clone());
                    return Ok(RedirectVerdict::Forward { upstream });
                }
                FilterDecision::Drop(code) => {
                    return Ok(RedirectVerdict::Drop {
                        http_code: code,
                        reason: format!("filter {} dropped", f.name()),
                    });
                }
                FilterDecision::Redirect { upstream } => {
                    return Ok(RedirectVerdict::Forward { upstream });
                }
            }
        }
        // Chain exhausted without a verdict — default forward.
        let upstream = req.resolved_upstream.unwrap_or(req.host.clone());
        Ok(RedirectVerdict::Forward { upstream })
    }
}

// ── Concrete filters used by the tests (also useful as defaults). ───────────

/// HTTP method allow-list filter. Drops anything not on the list with `405`.
#[derive(Debug)]
pub struct HttpMethodFilter {
    pub allowed: Vec<String>,
}

impl L7Filter for HttpMethodFilter {
    fn name(&self) -> &str {
        "http-method"
    }
    fn evaluate(&self, req: &mut ProxyRequest) -> FilterDecision {
        if self
            .allowed
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&req.method))
        {
            FilterDecision::Continue
        } else {
            FilterDecision::Drop(405)
        }
    }
}

/// DNS filter that rewrites a hostname to an IP using a static lookup table.
#[derive(Debug)]
pub struct DnsRewriteFilter {
    pub table: Vec<(String, String)>,
}

impl L7Filter for DnsRewriteFilter {
    fn name(&self) -> &str {
        "dns-rewrite"
    }
    fn evaluate(&self, req: &mut ProxyRequest) -> FilterDecision {
        if let Some((_, ip)) = self.table.iter().find(|(host, _)| host == &req.host) {
            req.resolved_upstream = Some(ip.clone());
            FilterDecision::Redirect { upstream: ip.clone() }
        } else {
            FilterDecision::Continue
        }
    }
}

/// Trailing forward filter — used at the end of a chain to commit.
#[derive(Debug)]
pub struct AlwaysForward;
impl L7Filter for AlwaysForward {
    fn name(&self) -> &str {
        "always-forward"
    }
    fn evaluate(&self, _req: &mut ProxyRequest) -> FilterDecision {
        FilterDecision::Forward
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/proxy/proxy.go", "Proxy.UpdateOrCreateRedirect");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn req(method: &str, host: &str, path: &str) -> ProxyRequest {
        ProxyRequest {
            method: method.into(),
            host: host.into(),
            path: path.into(),
            headers: vec![],
            resolved_upstream: None,
        }
    }

    fn proxy(tenant: &str, strict: bool) -> L7Proxy {
        let mut p = L7Proxy::new(TenantId::new(tenant).expect("test fixture"), "cluster.local");
        p.strict_mtls = strict;
        p
    }

    #[test]
    fn strict_mtls_refuses_request_without_peer_cert() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/proxy.go",
            "TerminateTLS",
            "tenant-l7p-no-cert"
        );
        let mut p = proxy("tenant-l7p-no-cert", true);
        p.add_filter(Box::new(AlwaysForward));
        let err = p.handle(&tenant, None, req("GET", "web", "/")).unwrap_err();
        assert!(matches!(err, L7ProxyError::NoPeerCert));
    }

    #[test]
    fn permissive_mtls_allows_request_without_peer_cert() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/proxy.go",
            "TerminateTLS",
            "tenant-l7p-permissive"
        );
        let mut p = proxy("tenant-l7p-permissive", false);
        p.add_filter(Box::new(AlwaysForward));
        let v = p.handle(&tenant, None, req("GET", "web", "/")).unwrap();
        assert_eq!(v, RedirectVerdict::Forward { upstream: "web".into() });
    }

    #[test]
    fn peer_outside_trust_domain_is_refused() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/proxy.go",
            "VerifyPeerSpiffe",
            "tenant-l7p-untrusted"
        );
        let mut p = proxy("tenant-l7p-untrusted", true);
        p.add_filter(Box::new(AlwaysForward));
        let err = p
            .handle(&tenant, Some("spiffe://other.local/ns/x/sa/y"), req("GET", "web", "/"))
            .unwrap_err();
        assert!(matches!(err, L7ProxyError::UntrustedPeer { .. }));
    }

    #[test]
    fn cross_tenant_proxy_use_is_refused() {
        let (_cite, attacker) = cilium_test_ctx!(
            "pkg/proxy/proxy.go",
            "tenantCheck",
            "tenant-attacker"
        );
        let mut p = proxy("acme", true);
        p.add_filter(Box::new(AlwaysForward));
        let err = p
            .handle(&attacker, Some("spiffe://cluster.local/ns/acme/sa/web"), req("GET", "web", "/"))
            .unwrap_err();
        assert!(matches!(err, L7ProxyError::TenantDenied { .. }));
    }

    #[test]
    fn empty_filter_chain_is_a_configuration_error() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/proxy.go",
            "buildFilterChain",
            "tenant-l7p-empty-chain"
        );
        let p = proxy("tenant-l7p-empty-chain", true);
        let err = p
            .handle(&tenant, Some("spiffe://cluster.local/ns/x/sa/y"), req("GET", "web", "/"))
            .unwrap_err();
        assert!(matches!(err, L7ProxyError::NoFilters));
    }

    #[test]
    fn http_method_filter_drops_disallowed_method_with_405() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/envoy/http.go",
            "MethodMatcher",
            "tenant-l7p-method"
        );
        let mut p = proxy("tenant-l7p-method", true);
        p.add_filter(Box::new(HttpMethodFilter { allowed: vec!["GET".into()] }));
        p.add_filter(Box::new(AlwaysForward));
        let v = p
            .handle(&tenant, Some("spiffe://cluster.local/ns/x/sa/y"), req("DELETE", "web", "/"))
            .unwrap();
        assert!(matches!(v, RedirectVerdict::Drop { http_code: 405, .. }));
    }

    #[test]
    fn dns_filter_rewrites_upstream_target_when_table_hits() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/dns/dns.go",
            "RewriteUpstream",
            "tenant-l7p-dns"
        );
        let mut p = proxy("tenant-l7p-dns", true);
        p.add_filter(Box::new(DnsRewriteFilter {
            table: vec![("api.acme.local".into(), "10.0.0.7".into())],
        }));
        let v = p
            .handle(
                &tenant,
                Some("spiffe://cluster.local/ns/x/sa/y"),
                req("GET", "api.acme.local", "/"),
            )
            .unwrap();
        assert_eq!(v, RedirectVerdict::Forward { upstream: "10.0.0.7".into() });
    }

    #[test]
    fn first_filter_to_emit_a_verdict_short_circuits_the_chain() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/envoy/filter_chain.go",
            "evaluateChain",
            "tenant-l7p-chain"
        );
        let mut p = proxy("tenant-l7p-chain", true);
        p.add_filter(Box::new(HttpMethodFilter { allowed: vec!["GET".into()] }));
        // Even though there's a filter after the drop, it must not run.
        p.add_filter(Box::new(DnsRewriteFilter {
            table: vec![("web".into(), "10.0.0.99".into())],
        }));
        let v = p
            .handle(
                &tenant,
                Some("spiffe://cluster.local/ns/x/sa/y"),
                req("POST", "web", "/"),
            )
            .unwrap();
        assert!(matches!(v, RedirectVerdict::Drop { http_code: 405, .. }));
    }
}
