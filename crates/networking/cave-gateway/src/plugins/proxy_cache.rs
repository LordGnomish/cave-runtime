// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! proxy-cache plugin — in-memory LRU cache for GET responses.
//!
//! Config:
//!   response_code: [200, 301, 404]   — which status codes to cache
//!   request_method: ["GET", "HEAD"]
//!   content_type: ["application/json"]
//!   cache_ttl: u64 (seconds, default 300)
//!   strategy: "memory" | "disk"
//!   cache_control: bool  — respect Cache-Control headers
//!   vary_headers: ["Accept", "Authorization"]
//!   cache_on_upstream_header_missing: bool

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use bytes::Bytes;
use lru::LruCache;
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct CacheEntry {
    status: u16,
    headers: HashMap<String, String>,
    body: Bytes,
    expires: Instant,
}

pub struct ProxyCachePlugin {
    cache: Mutex<LruCache<String, CacheEntry>>,
}

impl ProxyCachePlugin {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(1000).unwrap())),
        }
    }

    fn cache_key(ctx: &PluginCtx, vary_headers: &[&str]) -> String {
        let mut key = format!("{}:{}", ctx.method, ctx.path);
        if !ctx.query.is_empty() {
            key.push('?');
            key.push_str(&ctx.query);
        }
        for h in vary_headers {
            if let Some(v) = ctx.headers.get(*h) {
                key.push_str(&format!("|{}:{}", h, v));
            }
        }
        key
    }
}

impl Default for ProxyCachePlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GatewayPlugin for ProxyCachePlugin {
    fn name(&self) -> &'static str {
        "proxy-cache"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let methods: Vec<&str> = config["request_method"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| vec!["GET", "HEAD"]);

        if !methods.contains(&ctx.method.as_str()) {
            return PluginResult::Continue;
        }

        // Check Cache-Control: no-cache
        if config["cache_control"].as_bool().unwrap_or(true) {
            if let Some(cc) = ctx.headers.get("cache-control") {
                if cc.contains("no-cache") || cc.contains("no-store") {
                    return PluginResult::Continue;
                }
            }
        }

        let vary_headers: Vec<&str> = config["vary_headers"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let key = Self::cache_key(ctx, &vary_headers);

        if let Ok(mut cache) = self.cache.lock() {
            if let Some(entry) = cache.get(&key) {
                if entry.expires > Instant::now() {
                    // Cache hit — short-circuit
                    ctx.response_status = Some(entry.status);
                    ctx.response_headers = entry.headers.clone();
                    ctx.response_body = entry.body.clone();
                    ctx.response_headers
                        .insert("X-Cache-Status".to_string(), "Hit".to_string());

                    let mut resp = axum::response::Response::builder().status(entry.status);
                    for (k, v) in &entry.headers {
                        resp = resp.header(k, v);
                    }
                    resp = resp.header("X-Cache-Status", "Hit");

                    return PluginResult::Halt(
                        resp.body(axum::body::Body::from(entry.body.clone()))
                            .unwrap_or_else(|_| {
                                use axum::response::IntoResponse;
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
                            }),
                    );
                } else {
                    cache.pop(&key);
                }
            }
        }

        ctx.ctx.insert("cache_key".to_string(), Value::String(key));
        PluginResult::Continue
    }

    async fn body_filter(&self, ctx: &mut PluginCtx, config: &Value) {
        // Store response in cache after successful upstream call
        let cacheable_codes: Vec<u64> = config["response_code"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_u64()).collect())
            .unwrap_or_else(|| vec![200, 301, 302]);

        let status = ctx.response_status.unwrap_or(200) as u64;
        if !cacheable_codes.contains(&status) {
            return;
        }

        // Respect upstream Cache-Control
        if let Some(cc) = ctx.response_headers.get("cache-control") {
            if cc.contains("no-store") || cc.contains("private") {
                return;
            }
        }

        let ttl_secs = config["cache_ttl"].as_u64().unwrap_or(300);
        let key = match ctx.ctx.get("cache_key").and_then(|v| v.as_str()) {
            Some(k) => k.to_string(),
            None => return,
        };

        let entry = CacheEntry {
            status: ctx.response_status.unwrap_or(200),
            headers: ctx.response_headers.clone(),
            body: ctx.response_body.clone(),
            expires: Instant::now() + Duration::from_secs(ttl_secs),
        };

        if let Ok(mut cache) = self.cache.lock() {
            cache.put(key, entry);
        }
    }
}
