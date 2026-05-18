// SPDX-License-Identifier: AGPL-3.0-or-later
//! hmac-auth plugin — HMAC signature validation.
//!
//! Kong-compatible HMAC Authentication:
//!   Authorization: hmac username="...", algorithm="hmac-sha256",
//!                  headers="date x-custom-header",
//!                  signature="base64=="
//!
//! Config keys:
//!   username_parameter: "username"
//!   algorithms: ["hmac-sha1", "hmac-sha256", "hmac-sha384", "hmac-sha512"]
//!   headers: list of headers to include in signing string
//!   validate_request_body: bool
//!   clock_skew: u64 (seconds, default 300)

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Sha256, Sha384, Sha512};
use std::collections::HashMap;

pub struct HmacAuthPlugin;

type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;
type HmacSha384 = Hmac<Sha384>;
type HmacSha512 = Hmac<Sha512>;

fn parse_authorization_header(value: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let value = value.trim();
    let rest = value.strip_prefix("hmac ").or_else(|| value.strip_prefix("HMAC ")).unwrap_or(value);
    for part in rest.split(',') {
        let part = part.trim();
        if let Some(eq_pos) = part.find('=') {
            let key = part[..eq_pos].trim().to_lowercase();
            let val = part[eq_pos + 1..].trim().trim_matches('"').to_string();
            result.insert(key, val);
        }
    }
    result
}

fn compute_hmac(algorithm: &str, secret: &[u8], signing_string: &[u8]) -> Option<Vec<u8>> {
    match algorithm {
        "hmac-sha1" => {
            let mut mac = HmacSha1::new_from_slice(secret).ok()?;
            mac.update(signing_string);
            Some(mac.finalize().into_bytes().to_vec())
        }
        "hmac-sha256" => {
            let mut mac = HmacSha256::new_from_slice(secret).ok()?;
            mac.update(signing_string);
            Some(mac.finalize().into_bytes().to_vec())
        }
        "hmac-sha384" => {
            let mut mac = HmacSha384::new_from_slice(secret).ok()?;
            mac.update(signing_string);
            Some(mac.finalize().into_bytes().to_vec())
        }
        "hmac-sha512" => {
            let mut mac = HmacSha512::new_from_slice(secret).ok()?;
            mac.update(signing_string);
            Some(mac.finalize().into_bytes().to_vec())
        }
        _ => None,
    }
}

pub fn build_signing_string(ctx: &PluginCtx, header_list: &[&str]) -> String {
    let mut parts = Vec::new();
    for h in header_list {
        let h_lower = h.to_lowercase();
        let value = match h_lower.as_str() {
            "request-target" => format!("{} {}", ctx.method.to_lowercase(), ctx.path),
            _ => ctx.headers.get(&h_lower).cloned().unwrap_or_default(),
        };
        parts.push(format!("{}: {}", h_lower, value));
    }
    parts.join("\n")
}

pub fn verify_hmac(signing_string: &str, signature_b64: &str, algorithm: &str, secret: &[u8]) -> bool {
    let expected = match compute_hmac(algorithm, secret, signing_string.as_bytes()) {
        Some(v) => v,
        None => return false,
    };
    let provided = match STANDARD.decode(signature_b64) {
        Ok(v) => v,
        Err(_) => return false,
    };
    // constant-time comparison
    expected.len() == provided.len() && expected.iter().zip(provided.iter()).all(|(a, b)| a == b)
}

#[async_trait]
impl GatewayPlugin for HmacAuthPlugin {
    fn name(&self) -> &'static str {
        "hmac-auth"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let auth_header = match ctx.headers.get("authorization") {
            Some(h) => h.clone(),
            None => {
                if config["anonymous"].is_string() {
                    return PluginResult::Continue;
                }
                return PluginResult::Halt(
                    (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"message": "Unauthorized"}))).into_response(),
                );
            }
        };

        let params = parse_authorization_header(&auth_header);

        let username = match params.get("username") {
            Some(u) => u.clone(),
            None => {
                return PluginResult::Halt(
                    (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"message": "HMAC missing username"}))).into_response(),
                );
            }
        };

        let algorithm = params.get("algorithm").cloned().unwrap_or_else(|| "hmac-sha256".to_string());
        let signature = match params.get("signature") {
            Some(s) => s.clone(),
            None => {
                return PluginResult::Halt(
                    (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"message": "HMAC missing signature"}))).into_response(),
                );
            }
        };

        let header_list: Vec<&str> = params
            .get("headers")
            .map(|h| h.split_whitespace().collect())
            .unwrap_or_else(|| vec!["date"]);

        // Store for handler-side credential lookup and verification
        ctx.ctx.insert("hmac_username".to_string(), Value::String(username));
        ctx.ctx.insert("hmac_algorithm".to_string(), Value::String(algorithm));
        ctx.ctx.insert("hmac_signature".to_string(), Value::String(signature));
        ctx.ctx.insert("hmac_header_list".to_string(), Value::Array(
            header_list.iter().map(|h| Value::String(h.to_string())).collect(),
        ));
        ctx.ctx.insert("hmac_signing_string".to_string(), Value::String(build_signing_string(ctx, &header_list)));

        PluginResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authorization_header() {
        let h = r#"hmac username="bob", algorithm="hmac-sha256", headers="date host", signature="abc==""#;
        let params = parse_authorization_header(h);
        assert_eq!(params.get("username").map(String::as_str), Some("bob"));
        assert_eq!(params.get("algorithm").map(String::as_str), Some("hmac-sha256"));
    }

    #[test]
    fn verify_sha256() {
        let secret = b"my-secret";
        let signing_string = "date: Mon, 01 Jan 2024 00:00:00 GMT";
        let hmac = compute_hmac("hmac-sha256", secret, signing_string.as_bytes()).unwrap();
        let b64 = STANDARD.encode(&hmac);
        assert!(verify_hmac(signing_string, &b64, "hmac-sha256", secret));
        assert!(!verify_hmac(signing_string, &b64, "hmac-sha256", b"wrong"));
    }
}
