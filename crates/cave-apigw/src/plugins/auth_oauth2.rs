// SPDX-License-Identifier: AGPL-3.0-or-later
//! `oauth2` plugin — token introspection (RFC 7662) + scope enforcement.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str, cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::RwLock;

/// Static introspection cache so tests can drive the plugin offline.
pub struct IntrospectionCache {
    entries: RwLock<std::collections::HashMap<String, IntrospectionResponse>>,
}

#[derive(Debug, Clone)]
pub struct IntrospectionResponse {
    pub active: bool,
    pub scopes: HashSet<String>,
    pub sub: Option<String>,
}

impl Default for IntrospectionCache {
    fn default() -> Self {
        Self { entries: RwLock::new(std::collections::HashMap::new()) }
    }
}

impl IntrospectionCache {
    pub fn new() -> Self { Self::default() }
    pub fn put(&self, token: &str, resp: IntrospectionResponse) {
        self.entries.write().unwrap().insert(token.to_string(), resp);
    }
    pub fn get(&self, token: &str) -> Option<IntrospectionResponse> {
        self.entries.read().unwrap().get(token).cloned()
    }
}

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let header_name = cfg_str(cfg, "header_name").unwrap_or("authorization");
    let scheme = cfg_str(cfg, "scheme").unwrap_or("Bearer");
    let required_scopes = cfg_str_array(cfg, "scopes_required");
    let raw = ctx
        .request
        .headers
        .get(header_name)
        .ok_or_else(|| AGwError::Unauthorized("missing auth header".into()))?
        .clone();
    let token = raw
        .strip_prefix(&format!("{scheme} "))
        .ok_or_else(|| AGwError::Unauthorized(format!("expected {scheme} scheme")))?
        .trim()
        .to_string();

    // Static "valid_tokens" mode for offline tests
    let valid = cfg_str_array(cfg, "valid_tokens");
    let valid_scopes = cfg_str_array(cfg, "valid_token_scopes");
    if !valid.is_empty() {
        if !valid.contains(&token) {
            return Err(AGwError::Unauthorized("token not valid".into()));
        }
        let scope_set: HashSet<String> = valid_scopes.into_iter().collect();
        for s in &required_scopes {
            if !scope_set.contains(s) {
                return Err(AGwError::Forbidden(format!("missing scope {s}")));
            }
        }
        ctx.request.headers.insert("x-oauth2-token".into(), token.clone());
        return Ok(None);
    }

    // Otherwise: expect a `mock_introspection` config object (key→response).
    let mock = cfg.get("mock_introspection").and_then(|v| v.as_object());
    let resp = if let Some(obj) = mock {
        let entry = obj.get(&token).ok_or_else(|| AGwError::Unauthorized("token unknown".into()))?;
        IntrospectionResponse {
            active: entry.get("active").and_then(|v| v.as_bool()).unwrap_or(false),
            scopes: entry
                .get("scope")
                .and_then(|v| v.as_str())
                .map(|s| s.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            sub: entry.get("sub").and_then(|v| v.as_str()).map(String::from),
        }
    } else {
        return Err(AGwError::Plugin {
            plugin: "oauth2".into(),
            reason: "introspection endpoint not configured".into(),
        });
    };

    if !resp.active {
        return Err(AGwError::Unauthorized("token inactive".into()));
    }
    for s in &required_scopes {
        if !resp.scopes.contains(s) {
            return Err(AGwError::Forbidden(format!("missing scope {s}")));
        }
    }
    if let Some(sub) = resp.sub {
        ctx.request.headers.insert("x-oauth2-sub".into(), sub);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;

    fn ctx(req: GwRequest) -> PluginContext {
        PluginContext::new(req, None, Route::new("r"))
    }

    #[test]
    fn missing_token_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }

    #[test]
    fn valid_static_token_passes() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer alpha"));
        let r = access(&serde_json::json!({ "valid_tokens": ["alpha"] }), &mut c).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn scope_enforcement_blocks_missing() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer alpha"));
        let r = access(
            &serde_json::json!({
                "valid_tokens": ["alpha"],
                "valid_token_scopes": ["read"],
                "scopes_required": ["write"],
            }),
            &mut c,
        );
        assert!(r.is_err());
    }

    #[test]
    fn mock_introspection_active_token_passes() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer xyz"));
        let cfg = serde_json::json!({
            "mock_introspection": { "xyz": { "active": true, "scope": "read write", "sub": "u-1" } },
        });
        let r = access(&cfg, &mut c).unwrap();
        assert!(r.is_none());
        assert_eq!(c.request.headers.get("x-oauth2-sub").map(|s| s.as_str()), Some("u-1"));
    }

    #[test]
    fn mock_introspection_inactive_token_blocked() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Bearer xyz"));
        let cfg = serde_json::json!({
            "mock_introspection": { "xyz": { "active": false, "scope": "", "sub": "u-1" } },
        });
        assert!(access(&cfg, &mut c).is_err());
    }
}
