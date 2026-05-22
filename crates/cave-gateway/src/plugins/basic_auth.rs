// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! basic-auth plugin.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

pub struct BasicAuthPlugin;

#[async_trait]
impl GatewayPlugin for BasicAuthPlugin {
    fn name(&self) -> &'static str {
        "basic-auth"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let header = match ctx.headers.get("authorization") {
            Some(h) => h.clone(),
            None => {
                if config["anonymous"].is_string() {
                    return PluginResult::Continue;
                }
                return PluginResult::Halt(
                    (
                        StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({"message": "Unauthorized"})),
                    )
                        .into_response(),
                );
            }
        };

        let encoded = match header
            .strip_prefix("Basic ")
            .or_else(|| header.strip_prefix("basic "))
        {
            Some(e) => e.to_string(),
            None => {
                return PluginResult::Halt(
                    (
                        StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({"message": "Invalid authorization header"})),
                    )
                        .into_response(),
                );
            }
        };

        let decoded = match STANDARD.decode(encoded.trim()) {
            Ok(b) => String::from_utf8_lossy(&b).to_string(),
            Err(_) => {
                return PluginResult::Halt(
                    (
                        StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({"message": "Invalid credentials"})),
                    )
                        .into_response(),
                );
            }
        };

        let mut parts = decoded.splitn(2, ':');
        let username = parts.next().unwrap_or("").to_string();
        let password = parts.next().unwrap_or("").to_string();

        // Store for handler-side store lookup
        ctx.ctx
            .insert("basic_auth_username".to_string(), Value::String(username));
        ctx.ctx
            .insert("basic_auth_password".to_string(), Value::String(password));

        PluginResult::Continue
    }
}
