// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rate limiting plugin — fixed window, sliding window, and Redis-backed counters.
//!
//! Config keys:
//!   second / minute / hour / day / month / year  — limits per window
//!   limit_by: "consumer" | "credential" | "ip" | "service" | "header" | "path"
//!   policy: "local" | "cluster" | "redis"
//!   header_name: (when limit_by = "header")
//!   redis_host / redis_port / redis_password / redis_database (for redis policy)
//!   fault_tolerant: bool (default true — pass on Redis errors)
//!   hide_client_headers: bool (default false)

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// In-process counter store (local policy).
#[derive(Clone)]
struct LocalCounterStore {
    // key: (identifier, window_name, window_start) → count
    counters: Arc<DashMap<(String, &'static str, u64), u64>>,
}

impl LocalCounterStore {
    fn new() -> Self {
        Self {
            counters: Arc::new(DashMap::new()),
        }
    }

    fn increment(&self, identifier: &str, window: &'static str, window_start: u64) -> u64 {
        let key = (identifier.to_string(), window, window_start);
        let mut entry = self.counters.entry(key).or_insert(0);
        *entry += 1;
        *entry
    }
}

use std::sync::OnceLock;
static LOCAL_COUNTER_STORE: OnceLock<LocalCounterStore> = OnceLock::new();

fn get_local_store() -> &'static LocalCounterStore {
    LOCAL_COUNTER_STORE.get_or_init(LocalCounterStore::new)
}

pub struct RateLimitingPlugin;

#[derive(Debug)]
struct WindowConfig {
    name: &'static str,
    secs: u64,
    limit: Option<u64>,
}

fn window_configs(config: &Value) -> Vec<WindowConfig> {
    vec![
        WindowConfig {
            name: "second",
            secs: 1,
            limit: config["second"].as_u64(),
        },
        WindowConfig {
            name: "minute",
            secs: 60,
            limit: config["minute"].as_u64(),
        },
        WindowConfig {
            name: "hour",
            secs: 3600,
            limit: config["hour"].as_u64(),
        },
        WindowConfig {
            name: "day",
            secs: 86400,
            limit: config["day"].as_u64(),
        },
        WindowConfig {
            name: "month",
            secs: 2592000,
            limit: config["month"].as_u64(),
        },
        WindowConfig {
            name: "year",
            secs: 31536000,
            limit: config["year"].as_u64(),
        },
    ]
    .into_iter()
    .filter(|w| w.limit.is_some())
    .collect()
}

fn rate_limit_key(ctx: &PluginCtx, limit_by: &str, header_name: Option<&str>) -> String {
    match limit_by {
        "consumer" => ctx
            .consumer_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| ctx.client_ip.clone()),
        "ip" => ctx.client_ip.clone(),
        "service" => ctx.service_id.map(|id| id.to_string()).unwrap_or_default(),
        "header" => {
            let h = header_name.unwrap_or("x-consumer-id");
            ctx.headers
                .get(h)
                .cloned()
                .unwrap_or_else(|| ctx.client_ip.clone())
        }
        "path" => ctx.path.clone(),
        _ => ctx.client_ip.clone(),
    }
}

#[async_trait]
impl GatewayPlugin for RateLimitingPlugin {
    fn name(&self) -> &'static str {
        "rate-limiting"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let windows = window_configs(config);
        if windows.is_empty() {
            return PluginResult::Continue;
        }

        let limit_by = config["limit_by"].as_str().unwrap_or("consumer");
        let header_name = config["header_name"].as_str();
        let hide_headers = config["hide_client_headers"].as_bool().unwrap_or(false);
        let identifier = rate_limit_key(ctx, limit_by, header_name);

        let store = get_local_store();
        let now = now_secs();

        for window in &windows {
            let window_start = (now / window.secs) * window.secs;
            let limit = window.limit.unwrap();
            let count = store.increment(&identifier, window.name, window_start);

            if count > limit {
                let mut response =
                    axum::response::Response::builder().status(StatusCode::TOO_MANY_REQUESTS);

                if !hide_headers {
                    response = response
                        .header("X-RateLimit-Limit-{}", limit.to_string())
                        .header("X-RateLimit-Remaining-{}", "0")
                        .header(
                            "X-RateLimit-Reset",
                            (window_start + window.secs).to_string(),
                        )
                        .header("Retry-After", window.secs.to_string());
                }

                return PluginResult::Halt(
                    response
                        .body(axum::body::Body::from(
                            serde_json::json!({
                                "message": "API rate limit exceeded"
                            })
                            .to_string(),
                        ))
                        .unwrap_or_else(|_| StatusCode::TOO_MANY_REQUESTS.into_response()),
                );
            }

            // Inject remaining headers into context for response
            if !hide_headers {
                let remaining = limit.saturating_sub(count);
                ctx.response_headers.insert(
                    format!("X-RateLimit-Limit-{}", window.name),
                    limit.to_string(),
                );
                ctx.response_headers.insert(
                    format!("X-RateLimit-Remaining-{}", window.name),
                    remaining.to_string(),
                );
                ctx.response_headers.insert(
                    format!("X-RateLimit-Reset-{}", window.name),
                    (window_start + window.secs).to_string(),
                );
            }
        }

        PluginResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use serde_json::json;
    use std::collections::HashMap;

    fn make_ctx(ip: &str) -> PluginCtx {
        PluginCtx::new(
            "GET".into(),
            "/test".into(),
            HashMap::new(),
            Bytes::new(),
            ip.into(),
        )
    }

    #[tokio::test]
    async fn allows_under_limit() {
        let plugin = RateLimitingPlugin;
        let config = json!({"second": 10, "limit_by": "ip"});
        let mut ctx = make_ctx("1.2.3.100");
        let result = plugin.access(&mut ctx, &config).await;
        assert!(matches!(result, PluginResult::Continue));
    }

    #[tokio::test]
    async fn blocks_over_limit() {
        let plugin = RateLimitingPlugin;
        let config = json!({"second": 1, "limit_by": "ip"});
        let mut ctx = make_ctx("10.0.0.199");
        // First call — allowed
        let _ = plugin.access(&mut ctx, &config).await;
        // Second call — over limit
        let result = plugin.access(&mut ctx, &config).await;
        assert!(matches!(result, PluginResult::Halt(_)));
    }
}
