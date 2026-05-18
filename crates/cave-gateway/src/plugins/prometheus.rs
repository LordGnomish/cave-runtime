// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus plugin — expose /metrics in OpenMetrics format.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;
use serde_json::Value;
use std::sync::{Arc, Mutex};

#[derive(Clone, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet, Debug)]
pub struct RequestLabels {
    pub method: String,
    pub status: String,
    pub route: String,
    pub service: String,
}

pub struct GatewayMetrics {
    pub registry: Mutex<Registry>,
    pub http_requests_total: Family<RequestLabels, Counter>,
    pub http_request_duration_seconds: Family<RequestLabels, Histogram>,
    pub upstream_latency_seconds: Family<RequestLabels, Histogram>,
    pub connections_active: Gauge,
}

impl GatewayMetrics {
    pub fn new() -> Arc<Self> {
        let mut registry = Registry::default();

        let http_requests_total = Family::<RequestLabels, Counter>::default();
        registry.register(
            "kong_http_requests_total",
            "Total HTTP requests",
            http_requests_total.clone(),
        );

        let http_request_duration_seconds =
            Family::<RequestLabels, Histogram>::new_with_constructor(|| {
                Histogram::new(exponential_buckets(0.001, 2.0, 16))
            });
        registry.register(
            "kong_http_request_duration_seconds",
            "HTTP request duration",
            http_request_duration_seconds.clone(),
        );

        let upstream_latency_seconds =
            Family::<RequestLabels, Histogram>::new_with_constructor(|| {
                Histogram::new(exponential_buckets(0.001, 2.0, 16))
            });
        registry.register(
            "kong_upstream_latency_seconds",
            "Upstream latency",
            upstream_latency_seconds.clone(),
        );

        let connections_active = Gauge::default();
        registry.register(
            "kong_nginx_http_current_connections",
            "Active connections",
            connections_active.clone(),
        );

        Arc::new(Self {
            registry: Mutex::new(registry),
            http_requests_total,
            http_request_duration_seconds,
            upstream_latency_seconds,
            connections_active,
        })
    }

    pub fn render(&self) -> String {
        let registry = self.registry.lock().unwrap();
        let mut output = String::new();
        encode(&mut output, &registry).unwrap_or_default();
        output
    }
}

pub struct PrometheusPlugin {
    pub metrics: Arc<GatewayMetrics>,
}

impl PrometheusPlugin {
    pub fn new() -> Self {
        Self { metrics: GatewayMetrics::new() }
    }
}

impl Default for PrometheusPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GatewayPlugin for PrometheusPlugin {
    fn name(&self) -> &'static str {
        "prometheus"
    }

    async fn log(&self, ctx: &PluginCtx, config: &Value) {
        let labels = RequestLabels {
            method: ctx.method.clone(),
            status: ctx.response_status.unwrap_or(0).to_string(),
            route: ctx.route_id.map(|id| id.to_string()).unwrap_or_default(),
            service: ctx.service_id.map(|id| id.to_string()).unwrap_or_default(),
        };

        self.metrics
            .http_requests_total
            .get_or_create(&labels)
            .inc();

        // Latency stored in ctx by proxy engine
        if let Some(latency) = ctx.ctx.get("upstream_latency_ms").and_then(|v| v.as_f64()) {
            self.metrics
                .upstream_latency_seconds
                .get_or_create(&labels)
                .observe(latency / 1000.0);
        }
    }
}
