// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ambient L7 policy — `ztunnel/src/state/policy/L7` (waypoint proxy filter chain).
//!
//! Ambient-mode L7 policy lives on the waypoint proxy (not on ztunnel, which
//! is L4-only). Per istio/istio v1.30.0, the waypoint's filter chain is
//! ordered:
//!
//!   1. JWT authentication (RequestAuthentication)
//!   2. AuthZ (AuthorizationPolicy)
//!   3. Rate-limit (local-rate-limit filter)
//!   4. Fault injection (HTTPFaultInjection)
//!   5. Route match + retry budget (VirtualService route)
//!   6. Telemetry tap (access log + metric increment)
//!
//! Each filter can `Allow`, `Deny`, or `Retry`. The first non-Allow outcome
//! terminates the chain. This module ports the ordering machinery + filter
//! trait + a canonical chain builder; concrete filter logic stays in
//! `authz.rs`, `traffic_policy.rs`, `telemetry.rs`.

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOutcome {
    Allow,
    Deny(String),
    Retry { after_ms: u32 },
}

/// A single filter in the waypoint's L7 chain.
pub trait L7Filter: Send + Sync {
    fn name(&self) -> &'static str;
    fn order(&self) -> u8;
    fn evaluate(&self, req: &L7Request) -> FilterOutcome;
}

#[derive(Debug, Clone, Default)]
pub struct L7Request {
    pub method: String,
    pub path: String,
    pub host: String,
    pub peer_spiffe_id: Option<String>,
    pub headers: Vec<(String, String)>,
    pub jwt_principal: Option<String>,
    pub remote_addr: String,
}

/// Canonical filter ordering — index matches `L7Filter::order`.
pub const ORDER_JWT_AUTHN: u8 = 10;
pub const ORDER_AUTHZ: u8 = 20;
pub const ORDER_RATE_LIMIT: u8 = 30;
pub const ORDER_FAULT_INJECTION: u8 = 40;
pub const ORDER_ROUTE_MATCH: u8 = 50;
pub const ORDER_TELEMETRY: u8 = 60;

/// One ordered filter chain attached to a waypoint listener.
pub struct L7FilterChain {
    pub name: String,
    filters: Vec<Arc<dyn L7Filter>>,
}

impl L7FilterChain {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            filters: Vec::new(),
        }
    }

    /// Insert keeping the chain sorted by `order()` (stable).
    pub fn add(&mut self, f: Arc<dyn L7Filter>) {
        self.filters.push(f);
        self.filters.sort_by_key(|f| f.order());
    }

    pub fn len(&self) -> usize {
        self.filters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    pub fn filter_names(&self) -> Vec<&'static str> {
        self.filters.iter().map(|f| f.name()).collect()
    }

    /// Run the chain. Returns `Allow` only if every filter Allows; first
    /// `Deny` or `Retry` short-circuits.
    pub fn evaluate(&self, req: &L7Request) -> FilterOutcome {
        for f in &self.filters {
            match f.evaluate(req) {
                FilterOutcome::Allow => continue,
                other => return other,
            }
        }
        FilterOutcome::Allow
    }
}

/// Canonical waypoint chain: built once per waypoint and reused per request.
pub fn build_default_chain<P, A, R, T>(
    jwt_authn: P,
    authz: A,
    rate_limit: R,
    telemetry: T,
) -> L7FilterChain
where
    P: L7Filter + 'static,
    A: L7Filter + 'static,
    R: L7Filter + 'static,
    T: L7Filter + 'static,
{
    let mut chain = L7FilterChain::new("default-waypoint");
    chain.add(Arc::new(jwt_authn));
    chain.add(Arc::new(authz));
    chain.add(Arc::new(rate_limit));
    chain.add(Arc::new(telemetry));
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AllowFilter(&'static str, u8);
    impl L7Filter for AllowFilter {
        fn name(&self) -> &'static str {
            self.0
        }
        fn order(&self) -> u8 {
            self.1
        }
        fn evaluate(&self, _: &L7Request) -> FilterOutcome {
            FilterOutcome::Allow
        }
    }

    struct DenyFilter(&'static str, u8, &'static str);
    impl L7Filter for DenyFilter {
        fn name(&self) -> &'static str {
            self.0
        }
        fn order(&self) -> u8 {
            self.1
        }
        fn evaluate(&self, _: &L7Request) -> FilterOutcome {
            FilterOutcome::Deny(self.2.into())
        }
    }

    struct RetryFilter(&'static str, u8, u32);
    impl L7Filter for RetryFilter {
        fn name(&self) -> &'static str {
            self.0
        }
        fn order(&self) -> u8 {
            self.1
        }
        fn evaluate(&self, _: &L7Request) -> FilterOutcome {
            FilterOutcome::Retry { after_ms: self.2 }
        }
    }

    fn req() -> L7Request {
        L7Request {
            method: "GET".into(),
            path: "/".into(),
            host: "h".into(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_chain_allows() {
        let c = L7FilterChain::new("c");
        assert_eq!(c.evaluate(&req()), FilterOutcome::Allow);
    }

    #[test]
    fn all_allow_filters_pass() {
        let mut c = L7FilterChain::new("c");
        c.add(Arc::new(AllowFilter("a1", 10)));
        c.add(Arc::new(AllowFilter("a2", 20)));
        c.add(Arc::new(AllowFilter("a3", 30)));
        assert_eq!(c.evaluate(&req()), FilterOutcome::Allow);
    }

    #[test]
    fn deny_short_circuits_chain() {
        let mut c = L7FilterChain::new("c");
        c.add(Arc::new(AllowFilter("a1", 10)));
        c.add(Arc::new(DenyFilter("authz", 20, "no-perms")));
        c.add(Arc::new(AllowFilter("never-runs", 30)));
        assert_eq!(c.evaluate(&req()), FilterOutcome::Deny("no-perms".into()));
    }

    #[test]
    fn retry_short_circuits_chain() {
        let mut c = L7FilterChain::new("c");
        c.add(Arc::new(AllowFilter("a1", 10)));
        c.add(Arc::new(RetryFilter("rl", 30, 250)));
        assert_eq!(c.evaluate(&req()), FilterOutcome::Retry { after_ms: 250 });
    }

    #[test]
    fn filter_order_sorted_ascending_after_add() {
        let mut c = L7FilterChain::new("c");
        c.add(Arc::new(AllowFilter("tel", ORDER_TELEMETRY)));
        c.add(Arc::new(AllowFilter("jwt", ORDER_JWT_AUTHN)));
        c.add(Arc::new(AllowFilter("rl", ORDER_RATE_LIMIT)));
        c.add(Arc::new(AllowFilter("az", ORDER_AUTHZ)));
        // Order must respect canonical priorities.
        let names = c.filter_names();
        assert_eq!(names, vec!["jwt", "az", "rl", "tel"]);
    }

    #[test]
    fn canonical_orders_constant_relationships() {
        assert!(ORDER_JWT_AUTHN < ORDER_AUTHZ);
        assert!(ORDER_AUTHZ < ORDER_RATE_LIMIT);
        assert!(ORDER_RATE_LIMIT < ORDER_FAULT_INJECTION);
        assert!(ORDER_FAULT_INJECTION < ORDER_ROUTE_MATCH);
        assert!(ORDER_ROUTE_MATCH < ORDER_TELEMETRY);
    }

    #[test]
    fn build_default_chain_has_four_filters_in_order() {
        let chain = build_default_chain(
            AllowFilter("jwt", ORDER_JWT_AUTHN),
            AllowFilter("authz", ORDER_AUTHZ),
            AllowFilter("rl", ORDER_RATE_LIMIT),
            AllowFilter("telemetry", ORDER_TELEMETRY),
        );
        assert_eq!(chain.len(), 4);
        assert_eq!(
            chain.filter_names(),
            vec!["jwt", "authz", "rl", "telemetry"]
        );
    }
}
