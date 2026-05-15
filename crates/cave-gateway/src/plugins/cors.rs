// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CORS plugin.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

pub struct CorsPlugin;

fn origins_match(origin: &str, allowed: &[&str]) -> bool {
    if allowed.contains(&"*") {
        return true;
    }
    allowed.contains(&origin)
}

#[async_trait]
impl GatewayPlugin for CorsPlugin {
    fn name(&self) -> &'static str {
        "cors"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let is_preflight = ctx.method == "OPTIONS";

        let origins: Vec<&str> = config["origins"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| vec!["*"]);

        let origin = ctx.headers.get("origin").cloned().unwrap_or_default();

        let origin_ok = origin.is_empty() || origins_match(&origin, &origins);

        let allowed_methods: String = config["methods"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| "GET, HEAD, PUT, PATCH, POST, DELETE, OPTIONS, TRACE, CONNECT".to_string());

        let allowed_headers: String = config["headers"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();

        let exposed_headers: String = config["exposed_headers"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();

        let max_age: String = config["max_age"].as_u64().unwrap_or(0).to_string();
        let credentials = config["credentials"].as_bool().unwrap_or(false);

        let cors_origin = if origins.contains(&"*") && !credentials {
            "*".to_string()
        } else if origin_ok && !origin.is_empty() {
            origin.clone()
        } else {
            "*".to_string()
        };

        ctx.response_headers.insert("Access-Control-Allow-Origin".to_string(), cors_origin);
        if credentials {
            ctx.response_headers.insert("Access-Control-Allow-Credentials".to_string(), "true".to_string());
        }
        if !exposed_headers.is_empty() {
            ctx.response_headers.insert("Access-Control-Expose-Headers".to_string(), exposed_headers);
        }

        if is_preflight {
            ctx.response_headers.insert("Access-Control-Allow-Methods".to_string(), allowed_methods);
            if !allowed_headers.is_empty() {
                ctx.response_headers.insert("Access-Control-Allow-Headers".to_string(), allowed_headers);
            }
            if max_age != "0" {
                ctx.response_headers.insert("Access-Control-Max-Age".to_string(), max_age);
            }

            let preflight_status = config["preflight_continue"].as_bool().unwrap_or(false);
            if !preflight_status {
                return PluginResult::Halt(
                    (StatusCode::NO_CONTENT, "").into_response(),
                );
            }
        }

        PluginResult::Continue
    }
}
