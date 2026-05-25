// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! request-termination plugin — always return a fixed response.
//!
//! Config: status_code, message, content_type, body, trigger (optional header check)

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

pub struct RequestTerminationPlugin;

#[async_trait]
impl GatewayPlugin for RequestTerminationPlugin {
    fn name(&self) -> &'static str {
        "request-termination"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        // Optional trigger: only terminate if a specific header/query is present
        if let Some(trigger) = config["trigger"].as_object() {
            let triggered = trigger.iter().any(|(key, expected)| {
                let header_val = ctx.headers.get(&key.to_lowercase());
                match (header_val, expected.as_str()) {
                    (Some(v), Some(e)) => v == e,
                    _ => false,
                }
            });
            if !triggered {
                return PluginResult::Continue;
            }
        }

        let status = config["status_code"].as_u64().unwrap_or(503) as u16;
        let status = StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE);

        let content_type = config["content_type"]
            .as_str()
            .unwrap_or("application/json");

        let body = if let Some(b) = config["body"].as_str() {
            b.to_string()
        } else {
            let msg = config["message"].as_str().unwrap_or("Service unavailable");
            serde_json::json!({"message": msg}).to_string()
        };

        PluginResult::Halt(
            axum::response::Response::builder()
                .status(status)
                .header("content-type", content_type)
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| StatusCode::SERVICE_UNAVAILABLE.into_response()),
        )
    }
}
