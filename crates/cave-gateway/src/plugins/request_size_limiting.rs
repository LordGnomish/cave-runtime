// SPDX-License-Identifier: AGPL-3.0-or-later
//! request-size-limiting plugin.
//!
//! Config: allowed_payload_size (MB, float, default 128)

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

pub struct RequestSizeLimitingPlugin;

#[async_trait]
impl GatewayPlugin for RequestSizeLimitingPlugin {
    fn name(&self) -> &'static str {
        "request-size-limiting"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let max_mb = config["allowed_payload_size"].as_f64().unwrap_or(128.0);
        let max_bytes = (max_mb * 1024.0 * 1024.0) as usize;

        if ctx.body.len() > max_bytes {
            return PluginResult::Halt(
                (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    axum::Json(serde_json::json!({
                        "message": format!("Request size limit exceeded. Allowed: {} MB", max_mb)
                    })),
                )
                    .into_response(),
            );
        }

        // Also check Content-Length header
        if let Some(cl) = ctx.headers.get("content-length") {
            if let Ok(n) = cl.parse::<usize>() {
                if n > max_bytes {
                    return PluginResult::Halt(
                        (StatusCode::PAYLOAD_TOO_LARGE, axum::Json(serde_json::json!({"message": "Request size limit exceeded"}))).into_response(),
                    );
                }
            }
        }

        PluginResult::Continue
    }
}
