// SPDX-License-Identifier: AGPL-3.0-or-later
//! `oauth2` plugin — RFC 7662 token introspection + scope enforcement.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str, cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use std::collections::HashSet;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let header_name = cfg_str(cfg, "header_name").unwrap_or("authorization");
    let scheme = cfg_str(cfg, "scheme").unwrap_or("Bearer");
    let required = cfg_str_array(cfg, "scopes_required");
    let raw = ctx.request.headers.get(header_name)
        .ok_or_else(|| AGwError::Unauthorized("missing auth header".into()))?.clone();
    let token = raw.strip_prefix(&format!("{scheme} "))
        .ok_or_else(|| AGwError::Unauthorized(format!("expected {scheme} scheme")))?.trim().to_string();
    let valid = cfg_str_array(cfg, "valid_tokens");
    let valid_scopes = cfg_str_array(cfg, "valid_token_scopes");
    if !valid.is_empty() {
        if !valid.contains(&token) { return Err(AGwError::Unauthorized("token not valid".into())); }
        let scope_set: HashSet<String> = valid_scopes.into_iter().collect();
        for s in &required {
            if !scope_set.contains(s) { return Err(AGwError::Forbidden(format!("missing scope {s}"))); }
        }
        ctx.request.headers.insert("x-oauth2-token".into(), token);
        return Ok(None);
    }
    let mock = cfg.get("mock_introspection").and_then(|v| v.as_object());
    let resp = mock.and_then(|o| o.get(&token))
        .ok_or_else(|| AGwError::Unauthorized("token unknown".into()))?;
    let active = resp.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
    if !active { return Err(AGwError::Unauthorized("token inactive".into())); }
    let scopes: HashSet<String> = resp.get("scope").and_then(|v| v.as_str())
        .map(|s| s.split_whitespace().map(String::from).collect()).unwrap_or_default();
    for s in &required {
        if !scopes.contains(s) { return Err(AGwError::Forbidden(format!("missing scope {s}"))); }
    }
    if let Some(sub) = resp.get("sub").and_then(|v| v.as_str()) {
        ctx.request.headers.insert("x-oauth2-sub".into(), sub.into());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;
    fn ctx(req: GwRequest) -> PluginContext { PluginContext::new(req, None, Route::new("r")) }
    #[test] fn missing_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }
    #[test] fn static_ok() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer a"));
        assert!(access(&serde_json::json!({ "valid_tokens": ["a"] }), &mut c).unwrap().is_none());
    }
    #[test] fn scope_missing() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer a"));
        assert!(access(&serde_json::json!({ "valid_tokens": ["a"], "valid_token_scopes": ["read"], "scopes_required": ["write"] }), &mut c).is_err());
    }
    #[test] fn intro_active_ok() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer x"));
        let cfg = serde_json::json!({ "mock_introspection": { "x": { "active": true, "scope": "read", "sub": "u" } } });
        assert!(access(&cfg, &mut c).unwrap().is_none());
        assert_eq!(c.request.headers.get("x-oauth2-sub").map(|s| s.as_str()), Some("u"));
    }
    #[test] fn intro_inactive_blocked() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer x"));
        let cfg = serde_json::json!({ "mock_introspection": { "x": { "active": false } } });
        assert!(access(&cfg, &mut c).is_err());
    }
}
