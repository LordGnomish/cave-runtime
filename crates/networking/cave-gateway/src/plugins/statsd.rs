// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! statsd plugin — Kong-compatible StatsD metric sink.
//!
//! Port of `kong/plugins/statsd`. Emits the default Kong metric set over the
//! StatsD UDP line protocol on the log phase. Unlike the (pull-based)
//! prometheus plugin this is a push sink for StatsD / DogStatsD / Datadog
//! agents, which is why it is a distinct subsystem rather than "superseded".
//!
//! Wire protocol (StatsD): `metric_name:value|type[|@sample_rate]` where the
//! type is the abbreviated form — counter `c`, gauge `g`, timer `ms`,
//! histogram `h`, meter `m`, set `s`.

use super::{GatewayPlugin, PluginCtx};
use async_trait::async_trait;
use serde_json::Value;

pub const DEFAULT_HOST: &str = "localhost";
pub const DEFAULT_PORT: u16 = 8125;
pub const DEFAULT_PREFIX: &str = "kong";

/// StatsD metric stat type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatType {
    Counter,
    Gauge,
    Timer,
    Histogram,
    Meter,
    Set,
}

impl StatType {
    /// StatsD wire abbreviation.
    pub fn wire(&self) -> &'static str {
        // STUB (RED)
        ""
    }
}

/// Per-request statistics fed to the metric builder.
#[derive(Debug, Clone, Default)]
pub struct RequestStats {
    pub service: String,
    pub status: u16,
    pub request_size: u64,
    pub response_size: u64,
    /// Total request latency (client→client), milliseconds.
    pub latency_ms: u64,
    /// Time spent in the upstream service, milliseconds.
    pub upstream_latency_ms: u64,
    /// Time spent inside the gateway, milliseconds.
    pub kong_latency_ms: u64,
    /// Authenticated consumer id, if any (drives unique_users / request_per_user).
    pub consumer: Option<String>,
}

/// Format a single StatsD line.
pub fn format_line(name: &str, value: i64, stat: StatType, sample_rate: Option<f64>) -> String {
    // STUB (RED)
    let _ = (name, value, stat, sample_rate);
    String::new()
}

/// Build the default Kong metric set for a completed request.
pub fn build_metrics(prefix: &str, s: &RequestStats) -> Vec<String> {
    // STUB (RED)
    let _ = (prefix, s);
    Vec::new()
}

pub struct StatsdPlugin;

#[async_trait]
impl GatewayPlugin for StatsdPlugin {
    fn name(&self) -> &'static str {
        "statsd"
    }

    async fn log(&self, ctx: &PluginCtx, config: &Value) {
        let host = config["host"].as_str().unwrap_or(DEFAULT_HOST);
        let port = config["port"].as_u64().unwrap_or(DEFAULT_PORT as u64) as u16;
        let prefix = config["prefix"].as_str().unwrap_or(DEFAULT_PREFIX);

        let stats = RequestStats {
            service: ctx
                .service_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unnamed".into()),
            status: ctx.response_status.unwrap_or(0),
            request_size: ctx.body.len() as u64,
            response_size: ctx.response_body.len() as u64,
            consumer: ctx.consumer_username.clone(),
            ..Default::default()
        };
        let lines = build_metrics(prefix, &stats);
        if lines.is_empty() {
            return;
        }
        if let Ok(sock) = tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            let payload = lines.join("\n");
            let _ = sock.send_to(payload.as_bytes(), (host, port)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_abbreviations() {
        assert_eq!(StatType::Counter.wire(), "c");
        assert_eq!(StatType::Gauge.wire(), "g");
        assert_eq!(StatType::Timer.wire(), "ms");
        assert_eq!(StatType::Histogram.wire(), "h");
        assert_eq!(StatType::Meter.wire(), "m");
        assert_eq!(StatType::Set.wire(), "s");
    }

    #[test]
    fn format_line_basic() {
        assert_eq!(
            format_line("kong.svc.request.count", 1, StatType::Counter, None),
            "kong.svc.request.count:1|c"
        );
        assert_eq!(
            format_line("kong.svc.latency", 42, StatType::Timer, None),
            "kong.svc.latency:42|ms"
        );
    }

    #[test]
    fn format_line_with_sample_rate() {
        assert_eq!(
            format_line("kong.svc.request.count", 1, StatType::Counter, Some(0.1)),
            "kong.svc.request.count:1|c|@0.1"
        );
    }

    #[test]
    fn build_metrics_default_set() {
        let s = RequestStats {
            service: "orders".into(),
            status: 200,
            request_size: 120,
            response_size: 340,
            latency_ms: 50,
            upstream_latency_ms: 40,
            kong_latency_ms: 10,
            consumer: None,
        };
        let lines = build_metrics("kong", &s);
        // request_count counter = 1
        assert!(lines.contains(&"kong.orders.request.count:1|c".to_string()));
        // sizes are counters in bytes
        assert!(lines.contains(&"kong.orders.request.size:120|c".to_string()));
        assert!(lines.contains(&"kong.orders.response.size:340|c".to_string()));
        // status count keyed by code
        assert!(lines.contains(&"kong.orders.status.200:1|c".to_string()));
        // latencies
        assert!(lines.contains(&"kong.orders.latency:50|ms".to_string()));
        assert!(lines.contains(&"kong.orders.upstream_latency:40|ms".to_string()));
        assert!(lines.contains(&"kong.orders.kong_latency:10|ms".to_string()));
    }

    #[test]
    fn build_metrics_emits_per_user_when_consumer_present() {
        let s = RequestStats {
            service: "orders".into(),
            status: 200,
            consumer: Some("alice".into()),
            ..Default::default()
        };
        let lines = build_metrics("kong", &s);
        // unique_users tracked as a set, request_per_user as a counter
        assert!(lines.contains(&"kong.orders.user.uniques:alice|s".to_string()));
        assert!(lines.contains(&"kong.orders.user.alice.request.count:1|c".to_string()));
    }

    #[test]
    fn build_metrics_no_user_metrics_when_anonymous() {
        let s = RequestStats {
            service: "orders".into(),
            status: 200,
            ..Default::default()
        };
        let lines = build_metrics("kong", &s);
        assert!(!lines.iter().any(|l| l.contains("user.uniques")));
    }
}
