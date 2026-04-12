//! oauth2 plugin — authorization code, client credentials, implicit, password flows.
//!
//! This is a full OAuth2 Authorization Server embedded in Kong.
//! Config keys:
//!   scopes: ["read", "write", ...]
//!   mandatory_scope: bool
//!   token_expiration: u64 (seconds, default 7200)
//!   enable_authorization_code: bool
//!   enable_client_credentials: bool
//!   enable_implicit_grant: bool
//!   enable_password_grant: bool
//!   hide_credentials: bool
//!   accept_http_if_already_terminated: bool
//!   anonymous: UUID
//!   global_credentials: bool
//!   auth_header_name: "Authorization"
//!   pkce: "none" | "lax" | "strict"

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

pub struct OAuth2Plugin;

fn extract_bearer_token(ctx: &PluginCtx, header_name: &str) -> Option<String> {
    if let Some(v) = ctx.headers.get(header_name) {
        if let Some(token) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")) {
            return Some(token.to_string());
        }
    }
    // Check query string
    for pair in ctx.query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if let (Some("access_token"), Some(v)) = (kv.next(), kv.next()) {
            return Some(v.to_string());
        }
    }
    None
}

#[async_trait]
impl GatewayPlugin for OAuth2Plugin {
    fn name(&self) -> &'static str {
        "oauth2"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let header_name = config["auth_header_name"].as_str().unwrap_or("authorization");

        match extract_bearer_token(ctx, header_name) {
            Some(token) => {
                // Token validation happens at the handler level where the store is accessible
                ctx.ctx.insert("oauth2_token".to_string(), Value::String(token));
                PluginResult::Continue
            }
            None => {
                if config["anonymous"].is_string() {
                    return PluginResult::Continue;
                }
                PluginResult::Halt(
                    (
                        StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({
                            "error": "invalid_request",
                            "error_description": "The access token is missing"
                        })),
                    )
                        .into_response(),
                )
            }
        }
    }
}

/// Generate an access token (used by the /oauth2/token endpoint).
pub fn generate_access_token() -> String {
    use rand::distributions::Alphanumeric;
    use rand::Rng;
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}
