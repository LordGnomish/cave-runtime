// SPDX-License-Identifier: AGPL-3.0-or-later
//! `ldap-auth` plugin — RFC 4513 Simple Bind via mock directory.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str, PluginContext};
use crate::proxy::GwResponse;
use base64::Engine;
use serde_json::Value;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let header_name = cfg_str(cfg, "header_name").unwrap_or("authorization");
    let realm = cfg_str(cfg, "realm").unwrap_or("ldap");
    let raw = ctx.request.headers.get(header_name).cloned()
        .ok_or_else(|| AGwError::Unauthorized(format!("realm={realm}")))?;
    let creds = parse_basic(&raw).ok_or_else(|| AGwError::Unauthorized("bad basic auth".into()))?;
    let mock = cfg.get("mock_directory").and_then(|v| v.as_object());
    let ok = mock.and_then(|o| o.get(&creds.username)).and_then(|v| v.as_str())
        .map(|expected| expected == creds.password).unwrap_or(false);
    if !ok { return Err(AGwError::Unauthorized(format!("bind failed for {}", creds.username))); }
    ctx.request.headers.insert("x-ldap-user".into(), creds.username);
    Ok(None)
}

#[derive(Debug)]
pub struct BasicCreds { pub username: String, pub password: String }
pub fn parse_basic(header: &str) -> Option<BasicCreds> {
    let b64 = header.strip_prefix("Basic ")?.trim();
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let s = String::from_utf8(bytes).ok()?;
    let (u, p) = s.split_once(':')?;
    Some(BasicCreds { username: u.into(), password: p.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;
    fn ctx(req: GwRequest) -> PluginContext { PluginContext::new(req, None, Route::new("r")) }
    fn auth(u: &str, p: &str) -> String {
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}")))
    }
    #[test] fn missing_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }
    #[test] fn valid_bind() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", &auth("alice", "secret")));
        assert!(access(&serde_json::json!({ "mock_directory": { "alice": "secret" } }), &mut c).unwrap().is_none());
        assert_eq!(c.request.headers.get("x-ldap-user").map(|s| s.as_str()), Some("alice"));
    }
    #[test] fn invalid_bind() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", &auth("alice", "wrong")));
        assert!(access(&serde_json::json!({ "mock_directory": { "alice": "secret" } }), &mut c).is_err());
    }
    #[test] fn unknown_user() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", &auth("eve", "x")));
        assert!(access(&serde_json::json!({ "mock_directory": { "alice": "secret" } }), &mut c).is_err());
    }
    #[test] fn parse_basic_helper() {
        let c = parse_basic(&auth("u", "p")).unwrap();
        assert_eq!(c.username, "u"); assert_eq!(c.password, "p");
    }
}
