// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! response-transformer plugin — add/remove/rename/replace response headers and body fields.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use serde_json::Value;

pub struct ResponseTransformerPlugin;

fn kv_split(s: &str) -> Option<(&str, &str)> {
    s.splitn(2, ':')
        .collect::<Vec<_>>()
        .as_slice()
        .try_into()
        .ok()
        .map(|[k, v]: &[&str; 2]| (*k, *v))
}

#[async_trait]
impl GatewayPlugin for ResponseTransformerPlugin {
    fn name(&self) -> &'static str {
        "response-transformer"
    }

    async fn header_filter(&self, ctx: &mut PluginCtx, config: &Value) {
        // remove
        if let Some(arr) = config["remove"]["headers"].as_array() {
            for v in arr {
                if let Some(k) = v.as_str() {
                    ctx.response_headers.remove(&k.to_lowercase());
                }
            }
        }
        // rename
        if let Some(arr) = config["rename"]["headers"].as_array() {
            for v in arr {
                if let Some(s) = v.as_str() {
                    if let Some((old, new)) = kv_split(s) {
                        if let Some(val) = ctx.response_headers.remove(&old.to_lowercase()) {
                            ctx.response_headers.insert(new.to_lowercase(), val);
                        }
                    }
                }
            }
        }
        // replace
        if let Some(arr) = config["replace"]["headers"].as_array() {
            for v in arr {
                if let Some(s) = v.as_str() {
                    if let Some((k, val)) = kv_split(s) {
                        if ctx.response_headers.contains_key(&k.to_lowercase()) {
                            ctx.response_headers
                                .insert(k.to_lowercase(), val.to_string());
                        }
                    }
                }
            }
        }
        // add
        if let Some(arr) = config["add"]["headers"].as_array() {
            for v in arr {
                if let Some(s) = v.as_str() {
                    if let Some((k, val)) = kv_split(s) {
                        ctx.response_headers
                            .entry(k.to_lowercase())
                            .or_insert_with(|| val.to_string());
                    }
                }
            }
        }
    }

    async fn body_filter(&self, ctx: &mut PluginCtx, config: &Value) {
        if let Ok(mut json_body) = serde_json::from_slice::<Value>(&ctx.response_body) {
            if let Some(obj) = json_body.as_object_mut() {
                if let Some(arr) = config["remove"]["body"].as_array() {
                    for v in arr {
                        if let Some(k) = v.as_str() {
                            obj.remove(k);
                        }
                    }
                }
                if let Some(arr) = config["add"]["body"].as_array() {
                    for v in arr {
                        if let Some(s) = v.as_str() {
                            if let Some((k, val)) = kv_split(s) {
                                obj.entry(k)
                                    .or_insert_with(|| Value::String(val.to_string()));
                            }
                        }
                    }
                }
                if let Ok(new_body) = serde_json::to_vec(obj) {
                    ctx.response_body = bytes::Bytes::from(new_body);
                }
            }
        }
    }
}
