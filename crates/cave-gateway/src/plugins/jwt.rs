// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JWT plugin — RS256, HS256, ES256 validation.
//!
//! Config keys:
//!   uri_param_names: ["jwt"]
//!   cookie_names: []
//!   header_names: ["Authorization"]
//!   claims_to_verify: ["exp", "nbf"]
//!   key_claim_name: "iss"   — claim that identifies the credential (iss → JwtCredential.key)
//!   secret_is_base64: bool
//!   anonymous: UUID
//!   run_on_preflight: bool

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct JwtPlugin;

fn extract_jwt_token(ctx: &PluginCtx, config: &Value) -> Option<String> {
    // 1. Authorization header: "Bearer <token>"
    let header_names: Vec<&str> = config["header_names"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_else(|| vec!["authorization"]);

    for h in &header_names {
        if let Some(v) = ctx.headers.get(*h) {
            if let Some(token) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")) {
                return Some(token.to_string());
            }
        }
    }

    // 2. Query param
    let uri_params: Vec<&str> = config["uri_param_names"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_else(|| vec!["jwt"]);

    for param in &uri_params {
        for pair in ctx.query.split('&') {
            let mut kv = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                if k == *param {
                    return Some(v.to_string());
                }
            }
        }
    }

    // 3. Cookie
    let cookie_names: Vec<&str> = config["cookie_names"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if !cookie_names.is_empty() {
        if let Some(cookie_header) = ctx.headers.get("cookie") {
            for part in cookie_header.split(';') {
                let part = part.trim();
                let mut kv = part.splitn(2, '=');
                if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                    if cookie_names.contains(&k.trim()) {
                        return Some(v.trim().to_string());
                    }
                }
            }
        }
    }

    None
}

#[async_trait]
impl GatewayPlugin for JwtPlugin {
    fn name(&self) -> &'static str {
        "jwt"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let token = match extract_jwt_token(ctx, config) {
            Some(t) => t,
            None => {
                if config["anonymous"].is_string() {
                    return PluginResult::Continue;
                }
                return PluginResult::Halt(
                    (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"message": "Unauthorized"}))).into_response(),
                );
            }
        };

        // Decode header to determine algorithm
        let header = match decode_header(&token) {
            Ok(h) => h,
            Err(_) => {
                return PluginResult::Halt(
                    (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"message": "Bad token; invalid JWT"})))
                        .into_response(),
                );
            }
        };

        // Store decoded unverified claims for downstream key lookup
        // Real validation happens after the key is fetched from store.
        // For now, do a best-effort decode to extract the key_claim (usually "iss").
        // The gateway handler will fetch the JwtCredential and re-verify.
        let key_claim = config["key_claim_name"].as_str().unwrap_or("iss");

        // Perform a decode WITHOUT signature verification to read claims
        let mut no_verify = Validation::new(header.alg);
        no_verify.insecure_disable_signature_validation();
        no_verify.required_spec_claims = std::collections::HashSet::new();
        no_verify.validate_exp = false;

        let unverified = decode::<Value>(&token, &DecodingKey::from_secret(b""), &no_verify);

        match unverified {
            Ok(data) => {
                if let Some(iss) = data.claims.get(key_claim).and_then(|v| v.as_str()) {
                    ctx.ctx.insert("jwt_token".to_string(), Value::String(token));
                    ctx.ctx.insert("jwt_iss".to_string(), Value::String(iss.to_string()));
                    ctx.ctx.insert("jwt_alg".to_string(), Value::String(format!("{:?}", header.alg)));
                    PluginResult::Continue
                } else {
                    PluginResult::Halt(
                        (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"message": "Invalid JWT: missing key claim"})))
                            .into_response(),
                    )
                }
            }
            Err(_) => PluginResult::Halt(
                (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"message": "Invalid JWT"})))
                    .into_response(),
            ),
        }
    }
}

/// Verify a JWT against a specific secret/key (called by the proxy handler).
pub fn verify_jwt_hs256(token: &str, secret: &str, validate_exp: bool) -> bool {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = validate_exp;
    decode::<Value>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation).is_ok()
}

pub fn verify_jwt_rs256(token: &str, public_key_pem: &str, validate_exp: bool) -> bool {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = validate_exp;
    DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
        .ok()
        .and_then(|key| decode::<Value>(token, &key, &validation).ok())
        .is_some()
}

pub fn verify_jwt_es256(token: &str, public_key_pem: &str, validate_exp: bool) -> bool {
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = validate_exp;
    DecodingKey::from_ec_pem(public_key_pem.as_bytes())
        .ok()
        .and_then(|key| decode::<Value>(token, &key, &validation).ok())
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;
    use std::collections::HashMap;

    fn make_token(secret: &str) -> String {
        let claims = json!({"sub": "test", "iss": "my-key", "exp": 9999999999u64});
        encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
    }

    #[tokio::test]
    async fn extracts_bearer_token() {
        let plugin = JwtPlugin;
        let token = make_token("secret");
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {}", token));

        let mut ctx = PluginCtx::new("GET".into(), "/".into(), headers, Bytes::new(), "1.2.3.4".into());
        let config = json!({"key_claim_name": "iss"});
        let result = plugin.access(&mut ctx, &config).await;
        assert!(matches!(result, PluginResult::Continue));
        assert_eq!(ctx.ctx.get("jwt_iss").and_then(|v| v.as_str()), Some("my-key"));
    }

    #[test]
    fn verify_hs256() {
        let token = make_token("mysecret");
        assert!(verify_jwt_hs256(&token, "mysecret", true));
        assert!(!verify_jwt_hs256(&token, "wrongsecret", true));
    }
}
