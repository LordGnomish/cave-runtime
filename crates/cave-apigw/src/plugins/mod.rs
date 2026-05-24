// SPDX-License-Identifier: AGPL-3.0-or-later
//! Plugin runtime — phase dispatch + per-kind handlers.
//!
//! Kong plugins implement up to four phases (`access` / `header_filter` /
//! `body_filter` / `log`); this module dispatches on `PluginKind`.

use crate::error::{AGwError, AGwResult};
use crate::models::{Plugin, PluginKind, Route, Service};
use crate::proxy::{GwRequest, GwResponse};
use serde_json::Value;

pub mod auth_key;
pub mod auth_jwt;
pub mod auth_oauth2;
pub mod auth_mtls;
pub mod auth_ldap;
pub mod rate_limit;
pub mod cache;
pub mod transform;
pub mod cors;
pub mod bot_detection;
pub mod ip_restrict;
pub mod circuit_breaker;
pub mod retry;
pub mod headers;

#[derive(Debug, Clone)]
pub struct PluginContext {
    pub request: GwRequest,
    pub response: Option<GwResponse>,
    pub route: Route,
    pub service: Option<Service>,
}

impl PluginContext {
    pub fn new(request: GwRequest, service: Option<Service>, route: Route) -> Self {
        Self { request, response: None, route, service }
    }
}

/// Run a plugin's `access` phase. Returning `Some(resp)` short-circuits forwarding.
pub fn access(plugin: &Plugin, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    if !plugin.enabled {
        return Ok(None);
    }
    let cfg = &plugin.config;
    match plugin.kind {
        PluginKind::KeyAuth => auth_key::access(cfg, ctx),
        PluginKind::Jwt => auth_jwt::access(cfg, ctx),
        PluginKind::Oauth2 => auth_oauth2::access(cfg, ctx),
        PluginKind::Mtls => auth_mtls::access(cfg, ctx),
        PluginKind::Ldap => auth_ldap::access(cfg, ctx),
        PluginKind::RateLimiting => rate_limit::access(cfg, ctx),
        PluginKind::ProxyCache => cache::access(cfg, ctx),
        PluginKind::RequestTransformer => transform::access_request(cfg, ctx),
        PluginKind::ResponseTransformer => Ok(None),
        PluginKind::Cors => cors::access(cfg, ctx),
        PluginKind::BotDetection => bot_detection::access(cfg, ctx),
        PluginKind::IpRestriction => ip_restrict::access(cfg, ctx),
        PluginKind::CircuitBreaker => circuit_breaker::access(cfg, ctx),
        PluginKind::Retry => retry::access(cfg, ctx),
        PluginKind::RequestTermination => Ok(Some(GwResponse::new(503))),
    }
}

pub fn header_filter(plugin: &Plugin, ctx: &mut PluginContext) -> AGwResult<()> {
    if !plugin.enabled {
        return Ok(());
    }
    let cfg = &plugin.config;
    match plugin.kind {
        PluginKind::Cors => cors::header_filter(cfg, ctx),
        PluginKind::ResponseTransformer => transform::header_filter_response(cfg, ctx),
        PluginKind::ProxyCache => cache::header_filter(cfg, ctx),
        PluginKind::RequestTransformer => Ok(()),
        PluginKind::RequestTermination => Ok(()),
        _ => Ok(()),
    }
}

pub fn body_filter(plugin: &Plugin, ctx: &mut PluginContext) -> AGwResult<()> {
    if !plugin.enabled {
        return Ok(());
    }
    let cfg = &plugin.config;
    match plugin.kind {
        PluginKind::ResponseTransformer => transform::body_filter_response(cfg, ctx),
        _ => Ok(()),
    }
}

pub fn log_phase(plugin: &Plugin, _ctx: &PluginContext) {
    if !plugin.enabled {
        return;
    }
    tracing::trace!(plugin = %plugin.name, kind = ?plugin.kind, "plugin log phase");
}

/// Helper for plugin configs to read a string field with a default.
pub(crate) fn cfg_str<'a>(cfg: &'a Value, key: &str) -> Option<&'a str> {
    cfg.get(key).and_then(|v| v.as_str())
}

pub(crate) fn cfg_u64(cfg: &Value, key: &str) -> Option<u64> {
    cfg.get(key).and_then(|v| v.as_u64())
}

pub(crate) fn cfg_bool(cfg: &Value, key: &str) -> Option<bool> {
    cfg.get(key).and_then(|v| v.as_bool())
}

pub(crate) fn cfg_str_array(cfg: &Value, key: &str) -> Vec<String> {
    cfg.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub(crate) fn bad_config(msg: impl Into<String>) -> AGwError {
    AGwError::Plugin { plugin: "config".into(), reason: msg.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Plugin;

    #[test]
    fn disabled_plugin_skips_access() {
        let mut ctx = PluginContext::new(GwRequest::new("GET", "/", "h"), None, Route::new("r"));
        let mut p = Plugin::new("p", PluginKind::KeyAuth);
        p.enabled = false;
        assert!(access(&p, &mut ctx).unwrap().is_none());
    }

    #[test]
    fn request_termination_short_circuits() {
        let mut ctx = PluginContext::new(GwRequest::new("GET", "/", "h"), None, Route::new("r"));
        let p = Plugin::new("term", PluginKind::RequestTermination);
        let out = access(&p, &mut ctx).unwrap().unwrap();
        assert_eq!(out.status, 503);
    }

    #[test]
    fn header_filter_dispatches_per_kind() {
        let mut ctx = PluginContext::new(GwRequest::new("GET", "/", "h"), None, Route::new("r"));
        ctx.response = Some(GwResponse::new(200));
        let mut p = Plugin::new("cors", PluginKind::Cors);
        p.config = serde_json::json!({ "origins": ["https://app.example"] });
        header_filter(&p, &mut ctx).unwrap();
        let r = ctx.response.unwrap();
        assert!(r.headers.contains_key("access-control-allow-origin"));
    }
}
