// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cors` plugin — preflight + Access-Control headers.

use crate::error::AGwResult;
use crate::plugins::{cfg_bool, cfg_str_array, cfg_u64, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    if !ctx.request.method.eq_ignore_ascii_case("OPTIONS") { return Ok(None); }
    if cfg_bool(cfg, "preflight_continue").unwrap_or(false) { return Ok(None); }
    let mut resp = GwResponse::new(204);
    apply(cfg, &ctx.request.headers, &mut resp.headers);
    resp.headers.insert("access-control-max-age".into(), format!("{}", cfg_u64(cfg, "max_age").unwrap_or(3600)));
    Ok(Some(resp))
}

pub fn header_filter(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<()> {
    let resp = match ctx.response.as_mut() { Some(r) => r, None => return Ok(()) };
    apply(cfg, &ctx.request.headers, &mut resp.headers);
    Ok(())
}

fn apply(cfg: &Value, req: &std::collections::HashMap<String, String>, out: &mut std::collections::HashMap<String, String>) {
    let allow = cfg_str_array(cfg, "origins");
    let origin = req.get("origin").cloned().unwrap_or_default();
    let v = if allow.is_empty() || allow.iter().any(|o| o == "*") { "*".into() }
        else if allow.contains(&origin) { origin.clone() }
        else if !origin.is_empty() && allow.iter().any(|o| o.starts_with("https://*.") && origin.ends_with(&o[10..])) { origin.clone() }
        else { "null".into() };
    out.insert("access-control-allow-origin".into(), v);
    let methods = cfg_str_array(cfg, "methods");
    out.insert("access-control-allow-methods".into(),
        if methods.is_empty() { "GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD".into() } else { methods.join(", ") });
    let hdrs = cfg_str_array(cfg, "headers");
    out.insert("access-control-allow-headers".into(),
        if hdrs.is_empty() { "Content-Type, Authorization".into() } else { hdrs.join(", ") });
    let exposed = cfg_str_array(cfg, "exposed_headers");
    if !exposed.is_empty() { out.insert("access-control-expose-headers".into(), exposed.join(", ")); }
    if cfg_bool(cfg, "credentials").unwrap_or(false) {
        out.insert("access-control-allow-credentials".into(), "true".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;
    fn pc(req: GwRequest) -> PluginContext { PluginContext::new(req, None, Route::new("r")) }
    #[test] fn options_204() {
        let mut c = pc(GwRequest::new("OPTIONS", "/", "h").header("origin", "https://x"));
        let r = access(&serde_json::json!({ "origins": ["*"] }), &mut c).unwrap().unwrap();
        assert_eq!(r.status, 204);
        assert_eq!(r.headers.get("access-control-allow-origin").map(|s| s.as_str()), Some("*"));
    }
    #[test] fn preflight_continue_passes() {
        let mut c = pc(GwRequest::new("OPTIONS", "/", "h"));
        assert!(access(&serde_json::json!({ "preflight_continue": true }), &mut c).unwrap().is_none());
    }
    #[test] fn non_options_no_access() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        assert!(access(&serde_json::json!({}), &mut c).unwrap().is_none());
    }
    #[test] fn explicit_origin_match() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("origin", "https://app.example"));
        c.response = Some(GwResponse::new(200));
        header_filter(&serde_json::json!({ "origins": ["https://app.example"] }), &mut c).unwrap();
        assert_eq!(c.response.unwrap().headers.get("access-control-allow-origin").map(|s| s.as_str()), Some("https://app.example"));
    }
    #[test] fn unknown_origin_null() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("origin", "https://evil"));
        c.response = Some(GwResponse::new(200));
        header_filter(&serde_json::json!({ "origins": ["https://good"] }), &mut c).unwrap();
        assert_eq!(c.response.unwrap().headers.get("access-control-allow-origin").map(|s| s.as_str()), Some("null"));
    }
    #[test] fn credentials_emitted() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        c.response = Some(GwResponse::new(200));
        header_filter(&serde_json::json!({ "origins": ["*"], "credentials": true }), &mut c).unwrap();
        assert_eq!(c.response.unwrap().headers.get("access-control-allow-credentials").map(|s| s.as_str()), Some("true"));
    }
}
