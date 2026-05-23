// SPDX-License-Identifier: AGPL-3.0-or-later
//! `jwt` plugin — RFC 7519 token validation.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_bool, cfg_str, cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    #[serde(default)] iss: Option<String>,
    #[serde(default)] sub: Option<String>,
    #[serde(default)] aud: Option<Value>,
    #[serde(default)] exp: Option<u64>,
    #[serde(default)] nbf: Option<u64>,
    #[serde(default)] iat: Option<u64>,
    #[serde(default)] jti: Option<String>,
}

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let header_name = cfg_str(cfg, "header_name").unwrap_or("authorization");
    let scheme = cfg_str(cfg, "scheme").unwrap_or("Bearer");
    let secret = cfg_str(cfg, "secret").unwrap_or("");
    let allowed_iss = cfg_str_array(cfg, "allowed_issuers");
    let claims_to_verify = cfg_str_array(cfg, "claims_to_verify");
    let run_on_preflight = cfg_bool(cfg, "run_on_preflight").unwrap_or(true);
    if !run_on_preflight && ctx.request.method.eq_ignore_ascii_case("OPTIONS") { return Ok(None); }
    let raw = ctx.request.headers.get(header_name)
        .ok_or_else(|| AGwError::Unauthorized("missing auth header".into()))?.clone();
    let token = raw.strip_prefix(&format!("{scheme} "))
        .ok_or_else(|| AGwError::Unauthorized(format!("expected {scheme} scheme")))?.trim().to_string();
    if secret.is_empty() {
        let h = decode_header(&token).map_err(|e| AGwError::Unauthorized(format!("jwt header: {e}")))?;
        check_iss_unverified(&token, &allowed_iss)?;
        ctx.request.headers.insert("x-jwt-alg".into(), format!("{:?}", h.alg));
        return Ok(None);
    }
    let mut v = Validation::new(Algorithm::HS256);
    v.validate_exp = claims_to_verify.contains(&"exp".to_string());
    v.required_spec_claims = claims_to_verify.iter().cloned().collect();
    let data = decode::<Claims>(&token, &DecodingKey::from_secret(secret.as_bytes()), &v)
        .map_err(|e| AGwError::Unauthorized(format!("jwt verify: {e}")))?;
    if !allowed_iss.is_empty() {
        let iss = data.claims.iss.clone().unwrap_or_default();
        if !allowed_iss.contains(&iss) { return Err(AGwError::Forbidden(format!("issuer {iss} not allowed"))); }
    }
    if let Some(s) = data.claims.sub { ctx.request.headers.insert("x-jwt-sub".into(), s); }
    Ok(None)
}

fn check_iss_unverified(token: &str, allowed: &[String]) -> AGwResult<()> {
    if allowed.is_empty() { return Ok(()); }
    let body = token.split('.').nth(1).ok_or_else(|| AGwError::Unauthorized("malformed jwt".into()))?;
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body)
        .map_err(|e| AGwError::Unauthorized(format!("b64 {e}")))?;
    let json: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| AGwError::Unauthorized(format!("json {e}")))?;
    let iss = json.get("iss").and_then(|v| v.as_str()).unwrap_or("");
    if !allowed.iter().any(|a| a == iss) { return Err(AGwError::Forbidden(format!("issuer {iss} not allowed"))); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn issue(secret: &str, claims: Claims) -> String {
        encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
    }
    fn ctx(req: GwRequest) -> PluginContext { PluginContext::new(req, None, Route::new("r")) }

    #[test] fn missing_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }
    #[test] fn hs256_ok() {
        let token = issue("topsecret", Claims { iss: Some("svc".into()), sub: Some("u".into()),
            aud: None, exp: Some(u64::MAX), nbf: None, iat: None, jti: None });
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", &format!("Bearer {token}")));
        assert!(access(&serde_json::json!({ "secret": "topsecret" }), &mut c).unwrap().is_none());
        assert!(c.request.headers.contains_key("x-jwt-sub"));
    }
    #[test] fn wrong_secret_rejected() {
        let token = issue("topsecret", Claims { iss: Some("svc".into()), sub: None,
            aud: None, exp: Some(u64::MAX), nbf: None, iat: None, jti: None });
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", &format!("Bearer {token}")));
        assert!(access(&serde_json::json!({ "secret": "wrong" }), &mut c).is_err());
    }
    #[test] fn issuer_blocked() {
        let token = issue("topsecret", Claims { iss: Some("evil".into()), sub: None,
            aud: None, exp: Some(u64::MAX), nbf: None, iat: None, jti: None });
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", &format!("Bearer {token}")));
        assert!(access(&serde_json::json!({ "secret": "topsecret", "allowed_issuers": ["good"] }), &mut c).is_err());
    }
    #[test] fn options_skipped_when_disabled() {
        let mut c = ctx(GwRequest::new("OPTIONS", "/", "h"));
        assert!(access(&serde_json::json!({ "run_on_preflight": false }), &mut c).unwrap().is_none());
    }
    #[test] fn wrong_scheme_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("authorization", "Basic foo"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }
}
