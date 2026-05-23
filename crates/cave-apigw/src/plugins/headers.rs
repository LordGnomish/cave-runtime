// SPDX-License-Identifier: AGPL-3.0-or-later
//! `headers` plugin — security header pack (HSTS / nosniff / XFO / XSS / CSP).

use crate::error::AGwResult;
use crate::plugins::{cfg_bool, cfg_str, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;

#[allow(dead_code)]
pub fn apply(cfg: &Value, resp: &mut GwResponse) -> AGwResult<()> {
    if cfg_bool(cfg, "hsts").unwrap_or(true) {
        let max_age = cfg_str(cfg, "hsts_max_age").unwrap_or("63072000");
        resp.headers.insert("strict-transport-security".into(), format!("max-age={max_age}; includeSubDomains"));
    }
    if cfg_bool(cfg, "frame_deny").unwrap_or(true) {
        resp.headers.insert("x-frame-options".into(), "DENY".into());
    }
    if cfg_bool(cfg, "nosniff").unwrap_or(true) {
        resp.headers.insert("x-content-type-options".into(), "nosniff".into());
    }
    if cfg_bool(cfg, "xss").unwrap_or(true) {
        resp.headers.insert("x-xss-protection".into(), "1; mode=block".into());
    }
    if let Some(csp) = cfg_str(cfg, "csp") {
        resp.headers.insert("content-security-policy".into(), csp.into());
    }
    if let Some(rp) = cfg_str(cfg, "referrer_policy") {
        resp.headers.insert("referrer-policy".into(), rp.into());
    }
    Ok(())
}

#[allow(dead_code)]
pub fn header_filter(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<()> {
    if let Some(r) = ctx.response.as_mut() { apply(cfg, r)?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn hsts_default() {
        let mut r = GwResponse::new(200);
        apply(&serde_json::json!({}), &mut r).unwrap();
        assert!(r.headers.get("strict-transport-security").is_some());
    }
    #[test] fn xfo_deny() {
        let mut r = GwResponse::new(200);
        apply(&serde_json::json!({}), &mut r).unwrap();
        assert_eq!(r.headers.get("x-frame-options").map(|s| s.as_str()), Some("DENY"));
    }
    #[test] fn opt_out_hsts() {
        let mut r = GwResponse::new(200);
        apply(&serde_json::json!({ "hsts": false }), &mut r).unwrap();
        assert!(r.headers.get("strict-transport-security").is_none());
    }
    #[test] fn csp_custom() {
        let mut r = GwResponse::new(200);
        apply(&serde_json::json!({ "csp": "default-src 'self'" }), &mut r).unwrap();
        assert_eq!(r.headers.get("content-security-policy").map(|s| s.as_str()), Some("default-src 'self'"));
    }
    #[test] fn referrer_optional() {
        let mut r = GwResponse::new(200);
        apply(&serde_json::json!({}), &mut r).unwrap();
        assert!(r.headers.get("referrer-policy").is_none());
        apply(&serde_json::json!({ "referrer_policy": "no-referrer" }), &mut r).unwrap();
        assert_eq!(r.headers.get("referrer-policy").map(|s| s.as_str()), Some("no-referrer"));
    }
    #[test] fn nosniff_default() {
        let mut r = GwResponse::new(200);
        apply(&serde_json::json!({}), &mut r).unwrap();
        assert_eq!(r.headers.get("x-content-type-options").map(|s| s.as_str()), Some("nosniff"));
    }
}
