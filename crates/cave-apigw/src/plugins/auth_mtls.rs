// SPDX-License-Identifier: AGPL-3.0-or-later
//! `mtls` plugin — pin client cert by SHA-256 fingerprint of the leaf.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str, cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let header_name = cfg_str(cfg, "header_name").unwrap_or("x-client-cert-sha256");
    let allowed: Vec<String> = cfg_str_array(cfg, "trusted_fingerprints").into_iter().map(|s| s.to_lowercase()).collect();
    let san_dns_allowed = cfg_str_array(cfg, "trusted_dns_sans");
    let presented = ctx.request.headers.get(header_name).map(|s| s.to_lowercase())
        .ok_or_else(|| AGwError::Unauthorized("missing client cert fingerprint".into()))?;
    if !allowed.is_empty() && !allowed.contains(&presented) {
        return Err(AGwError::Forbidden("client cert not trusted".into()));
    }
    if !san_dns_allowed.is_empty() {
        let dns = ctx.request.headers.get("x-client-cert-dns").cloned().unwrap_or_default();
        if !san_dns_allowed.iter().any(|d| d == &dns) {
            return Err(AGwError::Forbidden(format!("DNS SAN {dns} not allowed")));
        }
    }
    ctx.request.headers.insert("x-mtls-verified".into(), "1".into());
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
    #[test] fn trusted_ok() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("x-client-cert-sha256", "ABCDEF"));
        assert!(access(&serde_json::json!({ "trusted_fingerprints": ["abcdef"] }), &mut c).unwrap().is_none());
    }
    #[test] fn untrusted_blocked() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("x-client-cert-sha256", "BAD"));
        assert!(access(&serde_json::json!({ "trusted_fingerprints": ["good"] }), &mut c).is_err());
    }
    #[test] fn san_dns_blocks_other() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("x-client-cert-sha256", "abc").header("x-client-cert-dns", "evil"));
        assert!(access(&serde_json::json!({ "trusted_fingerprints": ["abc"], "trusted_dns_sans": ["client.example"] }), &mut c).is_err());
    }
    #[test] fn san_dns_allows_match() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("x-client-cert-sha256", "abc").header("x-client-cert-dns", "client.example"));
        assert!(access(&serde_json::json!({ "trusted_fingerprints": ["abc"], "trusted_dns_sans": ["client.example"] }), &mut c).unwrap().is_none());
    }
}
