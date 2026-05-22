// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plugin system — Kong-compatible plugin pipeline.
//!
//! Each plugin implements the `GatewayPlugin` trait and is executed in
//! access → rewrite → proxy → header_filter → body_filter → log order.

pub mod acl;
pub mod basic_auth;
pub mod bot_detection;
pub mod cors;
pub mod grpc_gateway;
pub mod hmac_auth;
pub mod ip_restriction;
pub mod jwt;
pub mod key_auth;
pub mod logging;
pub mod oauth2;
pub mod prometheus;
pub mod proxy_cache;
pub mod rate_limiting;
pub mod request_size_limiting;
pub mod request_termination;
pub mod request_transformer;
pub mod response_transformer;
pub mod zipkin;

use async_trait::async_trait;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Context passed through the plugin pipeline for each request.
#[derive(Debug, Clone)]
pub struct PluginCtx {
    pub request_id: Uuid,
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
    pub consumer_id: Option<Uuid>,
    pub consumer_username: Option<String>,
    pub service_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub client_ip: String,
    pub response_status: Option<u16>,
    pub response_headers: HashMap<String, String>,
    pub response_body: Bytes,
    // Arbitrary key-value store for inter-plugin communication
    pub ctx: HashMap<String, Value>,
}

impl PluginCtx {
    pub fn new(
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: Bytes,
        client_ip: String,
    ) -> Self {
        let query = path.splitn(2, '?').nth(1).unwrap_or("").to_string();
        let path = path.splitn(2, '?').next().unwrap_or(&path).to_string();
        Self {
            request_id: Uuid::new_v4(),
            method,
            path,
            query,
            headers,
            body,
            consumer_id: None,
            consumer_username: None,
            service_id: None,
            route_id: None,
            client_ip,
            response_status: None,
            response_headers: HashMap::new(),
            response_body: Bytes::new(),
            ctx: HashMap::new(),
        }
    }
}

/// Plugin return value during the access phase.
pub enum PluginResult {
    /// Continue to next plugin / upstream
    Continue,
    /// Short-circuit and return this response immediately
    Halt(Response),
    /// Modify the context and continue
    Modified,
}

/// Every plugin implements this trait.
#[async_trait]
pub trait GatewayPlugin: Send + Sync {
    fn name(&self) -> &'static str;

    /// Access phase — authentication, authorization, rate limiting.
    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        PluginResult::Continue
    }

    /// Header filter — modify response headers before sending to client.
    async fn header_filter(&self, ctx: &mut PluginCtx, config: &Value) {
        // default: no-op
    }

    /// Body filter — modify response body.
    async fn body_filter(&self, ctx: &mut PluginCtx, config: &Value) {
        // default: no-op
    }

    /// Log phase — logging, metrics (always runs).
    async fn log(&self, ctx: &PluginCtx, config: &Value) {
        // default: no-op
    }
}

/// Ordered list of enabled plugins for a given request scope.
pub struct PluginChain {
    plugins: HashMap<String, Arc<dyn GatewayPlugin>>,
}

impl PluginChain {
    pub fn new() -> Arc<Self> {
        let mut plugins: HashMap<String, Arc<dyn GatewayPlugin>> = HashMap::new();

        plugins.insert(
            "rate-limiting".to_string(),
            Arc::new(rate_limiting::RateLimitingPlugin),
        );
        plugins.insert("key-auth".to_string(), Arc::new(key_auth::KeyAuthPlugin));
        plugins.insert("jwt".to_string(), Arc::new(jwt::JwtPlugin));
        plugins.insert("oauth2".to_string(), Arc::new(oauth2::OAuth2Plugin));
        plugins.insert(
            "basic-auth".to_string(),
            Arc::new(basic_auth::BasicAuthPlugin),
        );
        plugins.insert("hmac-auth".to_string(), Arc::new(hmac_auth::HmacAuthPlugin));
        plugins.insert("acl".to_string(), Arc::new(acl::AclPlugin));
        plugins.insert("cors".to_string(), Arc::new(cors::CorsPlugin));
        plugins.insert(
            "request-transformer".to_string(),
            Arc::new(request_transformer::RequestTransformerPlugin),
        );
        plugins.insert(
            "response-transformer".to_string(),
            Arc::new(response_transformer::ResponseTransformerPlugin),
        );
        plugins.insert(
            "ip-restriction".to_string(),
            Arc::new(ip_restriction::IpRestrictionPlugin),
        );
        plugins.insert(
            "bot-detection".to_string(),
            Arc::new(bot_detection::BotDetectionPlugin),
        );
        plugins.insert(
            "request-size-limiting".to_string(),
            Arc::new(request_size_limiting::RequestSizeLimitingPlugin),
        );
        plugins.insert(
            "proxy-cache".to_string(),
            Arc::new(proxy_cache::ProxyCachePlugin::new()),
        );
        plugins.insert(
            "request-termination".to_string(),
            Arc::new(request_termination::RequestTerminationPlugin),
        );
        plugins.insert("http-log".to_string(), Arc::new(logging::HttpLogPlugin));
        plugins.insert("file-log".to_string(), Arc::new(logging::FileLogPlugin));
        plugins.insert(
            "prometheus".to_string(),
            Arc::new(prometheus::PrometheusPlugin::new()),
        );
        plugins.insert("zipkin".to_string(), Arc::new(zipkin::ZipkinPlugin));
        plugins.insert(
            "grpc-gateway".to_string(),
            Arc::new(grpc_gateway::GrpcGatewayPlugin),
        );

        Arc::new(Self { plugins })
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn GatewayPlugin>> {
        self.plugins.get(name).cloned()
    }

    /// Run the access phase for a list of plugin configs.
    pub async fn run_access(
        &self,
        ctx: &mut PluginCtx,
        plugin_configs: &[(String, Value)],
    ) -> Option<Response> {
        for (name, config) in plugin_configs {
            if let Some(plugin) = self.get(name) {
                match plugin.access(ctx, config).await {
                    PluginResult::Continue | PluginResult::Modified => {}
                    PluginResult::Halt(resp) => return Some(resp),
                }
            }
        }
        None
    }

    /// Run the log phase (fire-and-forget).
    pub async fn run_log(&self, ctx: &PluginCtx, plugin_configs: &[(String, Value)]) {
        for (name, config) in plugin_configs {
            if let Some(plugin) = self.get(name) {
                plugin.log(ctx, config).await;
            }
        }
    }
}

impl Default for PluginChain {
    fn default() -> Self {
        // construct without wrapping in Arc for testing
        let chain = PluginChain::new();
        // unwrap Arc — only used in tests
        Arc::try_unwrap(chain).unwrap_or_else(|a| PluginChain {
            plugins: a.plugins.clone(),
        })
    }
}
