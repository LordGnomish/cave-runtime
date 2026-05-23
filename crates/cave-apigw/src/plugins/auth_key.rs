// SPDX-License-Identifier: AGPL-3.0-or-later
//! `key-auth` plugin — API key in header, query, or form-body.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let names = cfg_str_array(cfg, "key_names");
    let names = if names.is_empty() { vec!["apikey".into()] } else { names };
    let valid = cfg_str_array(cfg, "valid_keys");
    let key_in_body = cfg.get("key_in_body").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut found: Option<String> = None;
    for n in &names {
        let ln = n.to_lowercase();
        if let Some(v) = ctx.request.headers.get(&ln) { found = Some(v.clone()); break; }
        if let Some(q) = extract_query(&ctx.request.uri, n) { found = Some(q); break; }
    }
    if found.is_none() && key_in_body {
        let body = std::str::from_utf8(&ctx.request.body).unwrap_or("");
        for n in &names { if let Some(v) = extract_form(body, n) { found = Some(v); break; } }
    }
    let key = found.ok_or_else(|| AGwError::Unauthorized("missing api key".into()))?;
    if !valid.is_empty() && !valid.contains(&key) {
        return Err(AGwError::Unauthorized("invalid api key".into()));
    }
    ctx.request.headers.insert("x-consumer-credential-type".into(), "key-auth".into());
    Ok(None)
}
fn extract_query(uri: &str, name: &str) -> Option<String> {
    let (_, q) = uri.split_once('?')?;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') { if k == name { return Some(v.into()); } }
    }
    None
}
fn extract_form(body: &str, name: &str) -> Option<String> {
    for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') { if k == name { return Some(v.into()); } }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;
    fn ctx(req: GwRequest) -> PluginContext { PluginContext::new(req, None, Route::new("r")) }
    #[test] fn missing_rejected() { let mut c = ctx(GwRequest::new("GET", "/", "h")); assert!(access(&serde_json::json!({}), &mut c).is_err()); }
    #[test] fn header_ok() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("apikey", "s"));
        assert!(access(&serde_json::json!({ "valid_keys": ["s"] }), &mut c).unwrap().is_none());
    }
    #[test] fn invalid_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("apikey", "x"));
        assert!(access(&serde_json::json!({ "valid_keys": ["y"] }), &mut c).is_err());
    }
    #[test] fn query_ok() {
        let mut c = ctx(GwRequest::new("GET", "/?apikey=s", "h"));
        assert!(access(&serde_json::json!({ "valid_keys": ["s"] }), &mut c).unwrap().is_none());
    }
    #[test] fn custom_name() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("x-api-key", "s"));
        assert!(access(&serde_json::json!({ "key_names": ["x-api-key"], "valid_keys": ["s"] }), &mut c).unwrap().is_none());
    }
    #[test] fn body_form_ok() {
        let r = GwRequest::new("POST", "/", "h").body("apikey=s&x=1".as_bytes().to_vec());
        let mut c = ctx(r);
        assert!(access(&serde_json::json!({ "valid_keys": ["s"], "key_in_body": true }), &mut c).unwrap().is_none());
    }
}
