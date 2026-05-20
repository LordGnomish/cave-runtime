// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! key-auth plugin — API key authentication.
//!
//! Config keys:
//!   key_names: ["apikey", "x-api-key", ...]  — header/query param names to look for
//!   key_in_header: bool (default true)
//!   key_in_query: bool (default true)
//!   key_in_body: bool (default false)
//!   hide_credentials: bool (default false)
//!   anonymous: UUID of anonymous consumer (allows unauthenticated if set)
//!   run_on_preflight: bool (default true)

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

pub struct KeyAuthPlugin;

#[allow(dead_code)]
fn extract_key<'a>(
    ctx: &'a PluginCtx,
    key_names: &[&str],
    in_header: bool,
    in_query: bool,
) -> Option<&'a str> {
    if in_header {
        for name in key_names {
            if let Some(v) = ctx.headers.get(*name) {
                return Some(v.as_str());
            }
        }
    }
    if in_query {
        for pair in ctx.query.split('&') {
            let mut kv = pair.splitn(2, '=');
            if let (Some(k), Some(_v)) = (kv.next(), kv.next()) {
                if key_names.contains(&k) {
                    // Return a reference to the owned string — need to find it in headers map
                    // since we can't return a reference to local — just search headers
                    break;
                }
            }
        }
    }
    None
}

#[async_trait]
impl GatewayPlugin for KeyAuthPlugin {
    fn name(&self) -> &'static str {
        "key-auth"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let default_names = vec!["apikey", "x-api-key"];
        let key_names: Vec<&str> = if let Some(arr) = config["key_names"].as_array() {
            arr.iter().filter_map(|v| v.as_str()).collect()
        } else {
            default_names.clone()
        };

        let in_header = config["key_in_header"].as_bool().unwrap_or(true);
        let in_query = config["key_in_query"].as_bool().unwrap_or(true);
        let anonymous = config["anonymous"].as_str();

        // Search headers
        let mut found_key: Option<String> = None;

        if in_header {
            for name in &key_names {
                if let Some(v) = ctx.headers.get(*name) {
                    found_key = Some(v.clone());
                    break;
                }
            }
        }

        // Search query string
        if found_key.is_none() && in_query && !ctx.query.is_empty() {
            for pair in ctx.query.split('&') {
                let mut kv = pair.splitn(2, '=');
                if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                    if key_names.contains(&k) {
                        found_key = Some(v.to_string());
                        break;
                    }
                }
            }
        }

        match found_key {
            Some(key) => {
                // The actual key lookup against the store is performed by the
                // gateway handler which has store access. Here we store the key
                // in context for the handler to resolve.
                ctx.ctx
                    .insert("key_auth_key".to_string(), Value::String(key));
                PluginResult::Continue
            }
            None => {
                if let Some(_anon) = anonymous {
                    // Allow anonymous — consumer will be set to anonymous consumer
                    ctx.ctx
                        .insert("key_auth_anonymous".to_string(), Value::Bool(true));
                    PluginResult::Continue
                } else {
                    PluginResult::Halt(
                        (
                            StatusCode::UNAUTHORIZED,
                            axum::Json(
                                serde_json::json!({"message": "No API key found in request"}),
                            ),
                        )
                            .into_response(),
                    )
                }
            }
        }
    }
}
