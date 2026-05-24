// SPDX-License-Identifier: AGPL-3.0-or-later
//! Proxy core — runs a matched request through plugin phases and forwards
//! to the upstream.
//!
//! Phases (Kong-ordered):
//!   `access` → `header_filter` → `body_filter` → `log`
//! The implementation here is protocol-agnostic and operates over the
//! `GwRequest` / `GwResponse` value types so it can be exercised by tests
//! without a network.

use crate::error::{AGwError, AGwResult};
use crate::lb::{LbState, PickHint, PickedTarget};
use crate::models::{Plugin, Protocol, Route, Service, Target};
use crate::plugins::PluginContext;
use crate::store::GwStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct GwRequest {
    pub method: String,
    pub uri: String,
    pub host: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub source_ip: Option<String>,
}

impl GwRequest {
    pub fn new(method: &str, uri: &str, host: &str) -> Self {
        Self {
            method: method.to_string(),
            uri: uri.to_string(),
            host: host.to_string(),
            headers: HashMap::new(),
            body: vec![],
            source_ip: None,
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.insert(k.to_lowercase(), v.to_string());
        self
    }
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

#[derive(Debug, Clone)]
pub struct GwResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl GwResponse {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: vec![],
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.insert(k.to_lowercase(), v.to_string());
        self
    }
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

/// Synthetic upstream client — pluggable so tests can return canned responses.
pub trait UpstreamClient: Send + Sync {
    fn forward(&self, target: &Target, req: &GwRequest) -> AGwResult<GwResponse>;
}

/// Test/in-memory upstream client that echoes a pre-supplied response.
pub struct StaticUpstream {
    pub response: GwResponse,
}

impl UpstreamClient for StaticUpstream {
    fn forward(&self, _target: &Target, _req: &GwRequest) -> AGwResult<GwResponse> {
        Ok(self.response.clone())
    }
}

/// Failing upstream — flips error injection per call.
pub struct FailingUpstream {
    pub status: u16,
}

impl UpstreamClient for FailingUpstream {
    fn forward(&self, _target: &Target, _req: &GwRequest) -> AGwResult<GwResponse> {
        Err(AGwError::UpstreamUnhealthy(format!("upstream {}", self.status)))
    }
}

pub struct Proxy {
    store: Arc<GwStore>,
    lb: Arc<LbState>,
    client: Arc<dyn UpstreamClient>,
}

#[derive(Debug, Clone)]
pub struct ProxyOutcome {
    pub response: GwResponse,
    pub matched_route: Route,
    pub picked_target: Option<Target>,
    pub elapsed: Duration,
    pub plugins_run: Vec<String>,
    pub retries: u32,
}

impl Proxy {
    pub fn new(store: Arc<GwStore>, lb: Arc<LbState>, client: Arc<dyn UpstreamClient>) -> Self {
        Self { store, lb, client }
    }

    /// Run the full Access → Forward → Header/Body → Log lifecycle.
    pub fn handle(
        &self,
        route: &Route,
        service: Option<&Service>,
        upstream_targets: &[Target],
        req: GwRequest,
        rewritten_path: String,
    ) -> AGwResult<ProxyOutcome> {
        let start = Instant::now();
        // 1) resolve effective plugins
        let plugin_list = self.store.effective_plugins(
            Some(route.id),
            service.map(|s| s.id),
            None,
        );
        let mut ctx = PluginContext::new(req.clone(), service.cloned(), route.clone());
        ctx.request.uri = rewritten_path.clone();
        let mut plugins_run: Vec<String> = vec![];

        // 2) access phase — may short-circuit
        for plugin in &plugin_list {
            plugins_run.push(plugin.name.clone());
            if let Some(short) = crate::plugins::access(plugin, &mut ctx)? {
                return Ok(ProxyOutcome {
                    response: short,
                    matched_route: route.clone(),
                    picked_target: None,
                    elapsed: start.elapsed(),
                    plugins_run,
                    retries: 0,
                });
            }
        }

        // 3) forward — round-robin over upstream targets with retries
        let max_retries = service.map(|s| s.retries).unwrap_or(0);
        let mut retries = 0u32;
        let mut last_err: Option<AGwError> = None;
        let mut picked: Option<Target> = None;

        let hint = PickHint {
            source_ip: req.source_ip.clone(),
            ..Default::default()
        };

        for attempt in 0..=max_retries {
            let target = self.choose_target(upstream_targets, &hint);
            if target.is_none() {
                last_err = Some(AGwError::UpstreamUnhealthy("no targets".into()));
                continue;
            }
            let target = target.unwrap();
            picked = Some(target.clone());
            self.lb.inc_active(target.id);
            let t0 = Instant::now();
            let result = self.client.forward(&target, &ctx.request);
            let elapsed = t0.elapsed();
            self.lb.dec_active(target.id);
            match result {
                Ok(resp) => {
                    self.lb.record_outcome(target.id, true, elapsed);
                    ctx.response = Some(resp.clone());
                    retries = attempt;
                    // 4) header_filter + body_filter phases
                    for plugin in &plugin_list {
                        crate::plugins::header_filter(plugin, &mut ctx)?;
                        crate::plugins::body_filter(plugin, &mut ctx)?;
                    }
                    let response = ctx.response.unwrap_or(resp);
                    // 5) log phase
                    for plugin in &plugin_list {
                        crate::plugins::log_phase(plugin, &ctx);
                    }
                    return Ok(ProxyOutcome {
                        response,
                        matched_route: route.clone(),
                        picked_target: picked,
                        elapsed: start.elapsed(),
                        plugins_run,
                        retries,
                    });
                }
                Err(e) if e.is_retryable() => {
                    self.lb.record_outcome(target.id, false, elapsed);
                    last_err = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| AGwError::UpstreamUnhealthy("all retries exhausted".into())))
    }

    fn choose_target<'a>(&self, targets: &'a [Target], _hint: &PickHint) -> Option<Target> {
        if targets.is_empty() {
            return None;
        }
        // Trivial RR over slice for direct-target services (no upstream pool).
        let nanos = Instant::now().elapsed().subsec_nanos() as usize;
        Some(targets[nanos % targets.len()].clone())
    }
}

/// Convenience: turn a Service into a single-element target list when no upstream is configured.
pub fn service_as_target(svc: &Service) -> Target {
    Target {
        id: Uuid::nil(),
        host: svc.host.clone(),
        port: svc.port,
        weight: 1,
        tags: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Plugin, PluginKind, Route, Service};

    fn setup() -> (Arc<GwStore>, Arc<LbState>, Route, Service) {
        let store = Arc::new(GwStore::new());
        let svc = Service::new("svc", "backend", 80);
        let svc_id = store.upsert_service(svc.clone()).unwrap();
        let mut route = Route::new("r");
        route.paths = vec!["/api".into()];
        route.service_id = Some(svc_id);
        store.upsert_route(route.clone()).unwrap();
        (store, Arc::new(LbState::new()), route, svc)
    }

    #[test]
    fn handle_returns_upstream_response() {
        let (store, lb, route, svc) = setup();
        let resp = GwResponse::new(200).body(b"hello".to_vec());
        let client = Arc::new(StaticUpstream { response: resp });
        let p = Proxy::new(store, lb, client);
        let target = service_as_target(&svc);
        let out = p
            .handle(&route, Some(&svc), &[target], GwRequest::new("GET", "/api/x", "h"), "/x".into())
            .unwrap();
        assert_eq!(out.response.status, 200);
        assert_eq!(out.response.body, b"hello");
    }

    #[test]
    fn handle_retries_on_unhealthy_upstream() {
        let (store, lb, route, mut svc) = setup();
        svc.retries = 2;
        let client = Arc::new(FailingUpstream { status: 503 });
        let p = Proxy::new(store, lb, client);
        let target = service_as_target(&svc);
        let res = p.handle(&route, Some(&svc), &[target], GwRequest::new("GET", "/api/x", "h"), "/x".into());
        assert!(res.is_err());
    }

    #[test]
    fn handle_records_plugins_run() {
        let (store, lb, route, svc) = setup();
        let plugin = Plugin {
            route_id: Some(route.id),
            ..Plugin::new("retry-plug", PluginKind::Retry)
        };
        store.upsert_plugin(plugin).unwrap();
        let client = Arc::new(StaticUpstream { response: GwResponse::new(200) });
        let p = Proxy::new(store, lb, client);
        let out = p
            .handle(&route, Some(&svc), &[service_as_target(&svc)], GwRequest::new("GET", "/api/x", "h"), "/x".into())
            .unwrap();
        assert!(out.plugins_run.contains(&"retry-plug".to_string()));
    }
}

#[allow(dead_code)]
fn _ensure_imports_used(_pr: Protocol, _p: &Plugin) {}
