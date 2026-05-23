// SPDX-License-Identifier: AGPL-3.0-or-later
//! `request-transformer` + `response-transformer`.

use crate::error::AGwResult;
use crate::plugins::PluginContext;
use serde_json::Value;

fn list(cfg: &Value, kind: &str, what: &str) -> Vec<(String, String)> {
    cfg.get(kind).and_then(|c| c.get(what)).and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str())
            .filter_map(|s| s.split_once(':').map(|(k, v)| (k.trim().into(), v.trim().into()))).collect())
        .unwrap_or_default()
}
fn remove_list(cfg: &Value, kind: &str, what: &str) -> Vec<String> {
    cfg.get(kind).and_then(|c| c.get(what)).and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn access_request(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<crate::proxy::GwResponse>> {
    for (k, v) in list(cfg, "add", "headers") { ctx.request.headers.entry(k.to_lowercase()).or_insert(v); }
    for k in remove_list(cfg, "remove", "headers") { ctx.request.headers.remove(&k.to_lowercase()); }
    for (k, v) in list(cfg, "replace", "headers") {
        if ctx.request.headers.contains_key(&k.to_lowercase()) { ctx.request.headers.insert(k.to_lowercase(), v); }
    }
    for (k, v) in list(cfg, "append", "headers") {
        let lk = k.to_lowercase();
        if let Some(e) = ctx.request.headers.get_mut(&lk) { e.push_str(", "); e.push_str(&v); }
        else { ctx.request.headers.insert(lk, v); }
    }
    for (k, v) in list(cfg, "add", "querystring") {
        let sep = if ctx.request.uri.contains('?') { '&' } else { '?' };
        ctx.request.uri.push(sep); ctx.request.uri.push_str(&format!("{k}={v}"));
    }
    Ok(None)
}

pub fn header_filter_response(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<()> {
    let resp = match ctx.response.as_mut() { Some(r) => r, None => return Ok(()) };
    for (k, v) in list(cfg, "add", "headers") { resp.headers.entry(k.to_lowercase()).or_insert(v); }
    for k in remove_list(cfg, "remove", "headers") { resp.headers.remove(&k.to_lowercase()); }
    for (k, v) in list(cfg, "replace", "headers") {
        if resp.headers.contains_key(&k.to_lowercase()) { resp.headers.insert(k.to_lowercase(), v); }
    }
    Ok(())
}

pub fn body_filter_response(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<()> {
    if let Some(prefix) = cfg.get("body_prefix").and_then(|v| v.as_str()) {
        if let Some(r) = ctx.response.as_mut() {
            let mut n = prefix.as_bytes().to_vec(); n.extend_from_slice(&r.body); r.body = n;
        }
    }
    if let Some(suffix) = cfg.get("body_suffix").and_then(|v| v.as_str()) {
        if let Some(r) = ctx.response.as_mut() { r.body.extend_from_slice(suffix.as_bytes()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::{GwRequest, GwResponse};
    fn pc(req: GwRequest) -> PluginContext { PluginContext::new(req, None, Route::new("r")) }
    #[test] fn add_request_header() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        access_request(&serde_json::json!({ "add": { "headers": ["X-Tenant:a"] } }), &mut c).unwrap();
        assert_eq!(c.request.headers.get("x-tenant").map(|s| s.as_str()), Some("a"));
    }
    #[test] fn remove_header() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("x-internal", "yes"));
        access_request(&serde_json::json!({ "remove": { "headers": ["X-Internal"] } }), &mut c).unwrap();
        assert!(!c.request.headers.contains_key("x-internal"));
    }
    #[test] fn replace_only_present() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("x-user", "old"));
        access_request(&serde_json::json!({ "replace": { "headers": ["X-User:new"] } }), &mut c).unwrap();
        assert_eq!(c.request.headers.get("x-user").map(|s| s.as_str()), Some("new"));
    }
    #[test] fn append_combines() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("x-tags", "a"));
        access_request(&serde_json::json!({ "append": { "headers": ["X-Tags:b"] } }), &mut c).unwrap();
        assert_eq!(c.request.headers.get("x-tags").map(|s| s.as_str()), Some("a, b"));
    }
    #[test] fn querystring_add() {
        let mut c = pc(GwRequest::new("GET", "/x", "h"));
        access_request(&serde_json::json!({ "add": { "querystring": ["v=2"] } }), &mut c).unwrap();
        assert!(c.request.uri.contains("v=2"));
    }
    #[test] fn body_wrap() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        c.response = Some(GwResponse::new(200).body(b"core".to_vec()));
        body_filter_response(&serde_json::json!({ "body_prefix": "[", "body_suffix": "]" }), &mut c).unwrap();
        assert_eq!(c.response.unwrap().body, b"[core]");
    }
}
