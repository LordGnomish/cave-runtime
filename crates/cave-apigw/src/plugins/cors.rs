// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cors` plugin — CORS preflight handling + Access-Control headers.

use crate::error::AGwResult;
use crate::plugins::{cfg_bool, cfg_str_array, cfg_u64, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    if !ctx.request.method.eq_ignore_ascii_case("OPTIONS") {
        return Ok(None);
    }
    let preflight_continue = cfg_bool(cfg, "preflight_continue").unwrap_or(false);
    if preflight_continue {
        return Ok(None);
    }
    let mut resp = GwResponse::new(204);
    apply_cors_headers(cfg, &ctx.request.headers, &mut resp.headers);
    resp.headers.insert("access-control-max-age".into(), format!("{}", cfg_u64(cfg, "max_age").unwrap_or(3600)));
    Ok(Some(resp))
}

pub fn header_filter(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<()> {
    let resp = match ctx.response.as_mut() {
        Some(r) => r,
        None => return Ok(()),
    };
    apply_cors_headers(cfg, &ctx.request.headers, &mut resp.headers);
    Ok(())
}

fn apply_cors_headers(
    cfg: &Value,
    request_headers: &std::collections::HashMap<String, String>,
    out: &mut std::collections::HashMap<String, String>,
) {
    let allow_origins = cfg_str_array(cfg, "origins");
    let origin = request_headers.get("origin").cloned().unwrap_or_default();
    let allow_value = if allow_origins.is_empty() || allow_origins.iter().any(|o| o == "*") {
        "*".to_string()
    } else if allow_origins.contains(&origin) {
        origin
    } else if !origin.is_empty() && allow_origins.iter().any(|o| o.starts_with("https://*.")
        && origin.ends_with(&o[10..]))
    {
        origin
    } else {
        // No match — emit explicit null to signal disallow (Envoy behaviour).
        "null".to_string()
    };
    out.insert("access-control-allow-origin".into(), allow_value);
    let methods = cfg_str_array(cfg, "methods");
    if !methods.is_empty() {
        out.insert("access-control-allow-methods".into(), methods.join(", "));
    } else {
        out.insert(
            "access-control-allow-methods".into(),
            "GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD".into(),
        );
    }
    let headers = cfg_str_array(cfg, "headers");
    if !headers.is_empty() {
        out.insert("access-control-allow-headers".into(), headers.join(", "));
    } else {
        out.insert("access-control-allow-headers".into(), "Content-Type, Authorization".into());
    }
    let exposed = cfg_str_array(cfg, "exposed_headers");
    if !exposed.is_empty() {
        out.insert("access-control-expose-headers".into(), exposed.join(", "));
    }
    if cfg_bool(cfg, "credentials").unwrap_or(false) {
        out.insert("access-control-allow-credentials".into(), "true".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;

    fn pc(req: GwRequest) -> PluginContext {
        PluginContext::new(req, None, Route::new("r"))
    }

    #[test]
    fn options_short_circuits_to_204() {
        let mut c = pc(GwRequest::new("OPTIONS", "/", "h").header("origin", "https://x"));
        let r = access(&serde_json::json!({ "origins": ["*"] }), &mut c).unwrap().unwrap();
        assert_eq!(r.status, 204);
        assert_eq!(r.headers.get("access-control-allow-origin").map(|s| s.as_str()), Some("*"));
    }

    #[test]
    fn options_preflight_continue_passes_through() {
        let mut c = pc(GwRequest::new("OPTIONS", "/", "h"));
        let r = access(&serde_json::json!({ "preflight_continue": true }), &mut c).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn non_options_passes_through_access() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        let r = access(&serde_json::json!({}), &mut c).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn explicit_origin_match() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("origin", "https://app.example"));
        c.response = Some(GwResponse::new(200));
        header_filter(&serde_json::json!({ "origins": ["https://app.example"] }), &mut c).unwrap();
        let r = c.response.unwrap();
        assert_eq!(r.headers.get("access-control-allow-origin").map(|s| s.as_str()), Some("https://app.example"));
    }

    #[test]
    fn unknown_origin_returns_null() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("origin", "https://evil"));
        c.response = Some(GwResponse::new(200));
        header_filter(&serde_json::json!({ "origins": ["https://good"] }), &mut c).unwrap();
        let r = c.response.unwrap();
        assert_eq!(r.headers.get("access-control-allow-origin").map(|s| s.as_str()), Some("null"));
    }

    #[test]
    fn credentials_flag_emitted() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        c.response = Some(GwResponse::new(200));
        header_filter(&serde_json::json!({ "origins": ["*"], "credentials": true }), &mut c).unwrap();
        assert_eq!(c.response.unwrap().headers.get("access-control-allow-credentials").map(|s| s.as_str()), Some("true"));
    }
}
