// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/api/src/main/java/org/apache/kafka/connect/rest/ConnectRestExtension.java
//   connect/api/src/main/java/org/apache/kafka/connect/rest/ConnectRestExtensionContext.java
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/rest/RestServer.java

//! Connect REST extension API — pluggable filter chain that runs
//! around every Connect REST request.
//!
//! Mirrors upstream's `ConnectRestExtension` (KIP-285). Upstream
//! relies on Jersey `ContainerRequestFilter` / `ContainerResponseFilter`
//! plumbing; cave-streams keeps the same chain shape but expresses it
//! as a Rust trait so it can sit in front of our axum router or
//! be exercised in tests without spinning up HTTP.
//!
//! ## How registration works
//!
//! cave-streams avoids the classpath-discovery / `ServiceLoader`
//! dance upstream uses. Each extension is a Rust `static`+`fn`
//! pair registered by name at boot. The Rust analogue of the
//! `META-INF/services/...ConnectRestExtension` resource is the
//! [`RestExtensionRegistry::register`] call site.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::error::StreamsResult;

/// One filter step. Returns `Ok(Continue(_))` to keep the chain
/// going, `Ok(ShortCircuit(_))` to return early (e.g. an auth filter
/// that rejects the request).
#[derive(Debug, Clone)]
pub enum FilterDecision {
    /// Pass through with the (possibly mutated) request context.
    Continue(RestRequestCtx),
    /// Short-circuit with a response body + status. Subsequent
    /// filters are skipped.
    ShortCircuit { status: u16, body: String },
}

/// Per-request mutable context. Filters can stash data here for
/// downstream filters or the final handler to read.
#[derive(Debug, Clone, Default)]
pub struct RestRequestCtx {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
    /// Filter-set attributes. Connect upstream uses
    /// `ContainerRequest.setProperty` for the same purpose.
    pub attributes: BTreeMap<String, String>,
}

impl RestRequestCtx {
    pub fn new(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            ..Default::default()
        }
    }

    pub fn with_header(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.headers.insert(k.into(), v.into());
        self
    }

    pub fn with_attribute(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.attributes.insert(k.into(), v.into());
        self
    }
}

/// Per-extension config + cluster context handed to `register`.
///
/// Upstream is `ConnectRestExtensionContext`; cave-streams carries
/// the same fields used by the in-tree extensions.
#[derive(Debug, Clone, Default)]
pub struct ExtensionContext {
    pub worker_id: String,
    pub group_id: String,
    pub config: BTreeMap<String, String>,
}

/// A REST request filter. Multiple filters chain in registration
/// order.
pub trait RestExtensionFilter: Send + Sync + std::fmt::Debug + 'static {
    fn name(&self) -> &'static str;
    fn filter(&self, ctx: RestRequestCtx) -> StreamsResult<FilterDecision>;
}

/// The Connect REST extension itself. Upstream is a
/// `ConnectRestExtension` Java SPI; here it is a trait that gets a
/// chance to install filters at boot.
pub trait ConnectRestExtension: Send + Sync + std::fmt::Debug + 'static {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    /// Called once per worker. Implementors register filters via
    /// `chain.push(...)` and may stash config from `ctx`.
    fn register(&self, ctx: &ExtensionContext, chain: &mut RestFilterChain) -> StreamsResult<()>;
}

/// Ordered filter chain. Built up by extension `register` calls,
/// then exercised on each request.
#[derive(Debug, Clone, Default)]
pub struct RestFilterChain {
    filters: Vec<Arc<dyn RestExtensionFilter>>,
}

impl RestFilterChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, f: Arc<dyn RestExtensionFilter>) -> &mut Self {
        self.filters.push(f);
        self
    }

    pub fn len(&self) -> usize {
        self.filters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.filters.iter().map(|f| f.name()).collect()
    }

    /// Run the chain. Returns the final continuation context (when
    /// every filter said `Continue`) or the short-circuit response.
    pub fn execute(&self, mut ctx: RestRequestCtx) -> StreamsResult<FilterDecision> {
        for f in &self.filters {
            match f.filter(ctx)? {
                FilterDecision::Continue(next) => ctx = next,
                short @ FilterDecision::ShortCircuit { .. } => return Ok(short),
            }
        }
        Ok(FilterDecision::Continue(ctx))
    }
}

/// Registry of extensions. Built once at boot; each entry has a name
/// and an instance.
#[derive(Default)]
pub struct RestExtensionRegistry {
    extensions: RwLock<Vec<Arc<dyn ConnectRestExtension>>>,
}

impl RestExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, ext: Arc<dyn ConnectRestExtension>) {
        let mut g = self.extensions.write().expect("poisoned");
        g.push(ext);
    }

    pub fn len(&self) -> usize {
        self.extensions.read().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.extensions.read().expect("poisoned").is_empty()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.extensions
            .read()
            .expect("poisoned")
            .iter()
            .map(|e| e.name())
            .collect()
    }

    /// Walk every extension and let it install filters. Returns
    /// the assembled chain.
    pub fn assemble_chain(&self, ctx: &ExtensionContext) -> StreamsResult<RestFilterChain> {
        let mut chain = RestFilterChain::new();
        for ext in self.extensions.read().expect("poisoned").iter() {
            ext.register(ctx, &mut chain)?;
        }
        Ok(chain)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Reference in-tree filters — mirror the ones Connect ships under
// `connect/runtime/src/main/java/.../runtime/rest/extension/*`.
// ────────────────────────────────────────────────────────────────────────────

/// Authentication filter that rejects any request missing the
/// configured header. Mirrors the shape upstream's basic-auth
/// extension uses.
#[derive(Debug, Clone)]
pub struct RequireHeaderFilter {
    pub header: String,
    pub expected: String,
}

impl RestExtensionFilter for RequireHeaderFilter {
    fn name(&self) -> &'static str {
        "RequireHeaderFilter"
    }
    fn filter(&self, ctx: RestRequestCtx) -> StreamsResult<FilterDecision> {
        match ctx.headers.get(&self.header) {
            Some(v) if v == &self.expected => Ok(FilterDecision::Continue(ctx)),
            _ => Ok(FilterDecision::ShortCircuit {
                status: 401,
                body: format!("missing or wrong '{}' header", self.header),
            }),
        }
    }
}

/// Filter that stamps an attribute onto every request — handy in
/// tests as a positive-control filter.
#[derive(Debug, Clone)]
pub struct StampAttributeFilter {
    pub key: String,
    pub value: String,
}

impl RestExtensionFilter for StampAttributeFilter {
    fn name(&self) -> &'static str {
        "StampAttributeFilter"
    }
    fn filter(&self, mut ctx: RestRequestCtx) -> StreamsResult<FilterDecision> {
        ctx.attributes.insert(self.key.clone(), self.value.clone());
        Ok(FilterDecision::Continue(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct AuthExtension {
        header: String,
        expected: String,
    }

    impl ConnectRestExtension for AuthExtension {
        fn name(&self) -> &'static str {
            "test.AuthExtension"
        }
        fn register(
            &self,
            _ctx: &ExtensionContext,
            chain: &mut RestFilterChain,
        ) -> StreamsResult<()> {
            chain.push(Arc::new(RequireHeaderFilter {
                header: self.header.clone(),
                expected: self.expected.clone(),
            }));
            Ok(())
        }
    }

    #[derive(Debug)]
    struct StampExtension;
    impl ConnectRestExtension for StampExtension {
        fn name(&self) -> &'static str {
            "test.StampExtension"
        }
        fn register(
            &self,
            _ctx: &ExtensionContext,
            chain: &mut RestFilterChain,
        ) -> StreamsResult<()> {
            chain.push(Arc::new(StampAttributeFilter {
                key: "ext.stamp".into(),
                value: "ok".into(),
            }));
            Ok(())
        }
    }

    #[test]
    fn empty_chain_passes_through() {
        let chain = RestFilterChain::new();
        let ctx = RestRequestCtx::new("GET", "/connectors");
        let out = chain.execute(ctx).unwrap();
        match out {
            FilterDecision::Continue(c) => assert_eq!(c.path, "/connectors"),
            _ => panic!("empty chain should not short-circuit"),
        }
    }

    #[test]
    fn require_header_filter_short_circuits_when_missing() {
        let mut chain = RestFilterChain::new();
        chain.push(Arc::new(RequireHeaderFilter {
            header: "Authorization".into(),
            expected: "Bearer X".into(),
        }));
        let out = chain.execute(RestRequestCtx::new("GET", "/")).unwrap();
        match out {
            FilterDecision::ShortCircuit { status, .. } => assert_eq!(status, 401),
            _ => panic!("expected short-circuit"),
        }
    }

    #[test]
    fn require_header_filter_continues_on_match() {
        let mut chain = RestFilterChain::new();
        chain.push(Arc::new(RequireHeaderFilter {
            header: "Authorization".into(),
            expected: "Bearer X".into(),
        }));
        let ctx = RestRequestCtx::new("GET", "/").with_header("Authorization", "Bearer X");
        match chain.execute(ctx).unwrap() {
            FilterDecision::Continue(_) => {}
            _ => panic!("expected continue"),
        }
    }

    #[test]
    fn registry_assembles_chain_from_extensions() {
        let r = RestExtensionRegistry::new();
        r.register(Arc::new(AuthExtension {
            header: "X-Auth".into(),
            expected: "ok".into(),
        }));
        r.register(Arc::new(StampExtension));
        let chain = r.assemble_chain(&ExtensionContext::default()).unwrap();
        assert_eq!(chain.len(), 2);
    }

    #[test]
    fn extensions_register_in_order() {
        let r = RestExtensionRegistry::new();
        r.register(Arc::new(StampExtension));
        r.register(Arc::new(AuthExtension {
            header: "X-Auth".into(),
            expected: "ok".into(),
        }));
        let chain = r.assemble_chain(&ExtensionContext::default()).unwrap();
        assert_eq!(
            chain.names(),
            vec!["StampAttributeFilter", "RequireHeaderFilter"]
        );
    }

    #[test]
    fn chain_short_circuit_skips_subsequent_filters() {
        let mut chain = RestFilterChain::new();
        chain.push(Arc::new(RequireHeaderFilter {
            header: "X-Auth".into(),
            expected: "ok".into(),
        }));
        chain.push(Arc::new(StampAttributeFilter {
            key: "later".into(),
            value: "v".into(),
        }));
        let out = chain.execute(RestRequestCtx::new("GET", "/")).unwrap();
        // Must short-circuit before the stamp runs.
        assert!(matches!(out, FilterDecision::ShortCircuit { .. }));
    }

    #[test]
    fn stamp_filter_attaches_attribute() {
        let mut chain = RestFilterChain::new();
        chain.push(Arc::new(StampAttributeFilter {
            key: "x".into(),
            value: "y".into(),
        }));
        let out = chain.execute(RestRequestCtx::new("GET", "/")).unwrap();
        match out {
            FilterDecision::Continue(c) => assert_eq!(c.attributes.get("x"), Some(&"y".into())),
            _ => panic!(),
        }
    }

    #[test]
    fn registry_lists_extension_names() {
        let r = RestExtensionRegistry::new();
        r.register(Arc::new(StampExtension));
        assert_eq!(r.names(), vec!["test.StampExtension"]);
    }
}
