// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! correlation-id plugin — Kong-compatible request correlation identifier.
//!
//! Port of `kong/plugins/correlation-id/handler.lua`. On the access phase the
//! plugin guarantees that the upstream request carries a correlation-id header
//! (generating one when the client did not supply it). When `echo_downstream`
//! is enabled the same value is mirrored back onto the client response in the
//! header_filter phase.
//!
//! Three generators are supported, matching upstream:
//!   * `uuid`          — a fresh random UUID v4 per request.
//!   * `uuid#counter`  — a per-worker UUID suffixed with a monotonically
//!                       increasing counter (`<worker_uuid>#<n>`). Cheap and
//!                       still globally unique because the worker UUID is
//!                       unique per process.
//!   * `tracker`       — a composite of connection/server context:
//!                       `<client_ip>-<worker_pid>-<counter>-<timestamp_ms>`.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

/// Default header name (matches Kong's `Kong-Request-ID`).
pub const DEFAULT_HEADER_NAME: &str = "Kong-Request-ID";
/// Default generator (matches Kong's `uuid#counter`).
pub const DEFAULT_GENERATOR: &str = "uuid#counter";

pub struct CorrelationIdPlugin {
    /// Stable per-worker UUID used by the `uuid#counter` generator.
    worker_uuid: String,
    /// Monotonic counter shared by `uuid#counter` and `tracker`.
    counter: AtomicU64,
}

impl Default for CorrelationIdPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrelationIdPlugin {
    pub fn new() -> Self {
        Self {
            worker_uuid: Uuid::new_v4().to_string(),
            counter: AtomicU64::new(0),
        }
    }

    /// Generate a correlation id for the given generator name.
    ///
    /// `client_ip` and `now_ms` feed the `tracker` generator; they are ignored
    /// by the uuid generators.
    pub fn generate(&self, generator: &str, client_ip: &str, now_ms: i64) -> String {
        // STUB (RED): no generator logic yet.
        let _ = (generator, client_ip, now_ms);
        String::new()
    }

    fn next_counter(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst)
    }
}

#[async_trait]
impl GatewayPlugin for CorrelationIdPlugin {
    fn name(&self) -> &'static str {
        "correlation-id"
    }

    async fn access(&self, _ctx: &mut PluginCtx, _config: &Value) -> PluginResult {
        // STUB (RED): does not set the header yet.
        PluginResult::Continue
    }

    async fn header_filter(&self, _ctx: &mut PluginCtx, _config: &Value) {
        // STUB (RED): does not echo downstream yet.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::collections::HashMap;

    fn ctx(headers: &[(&str, &str)]) -> PluginCtx {
        let mut h = HashMap::new();
        for (k, v) in headers {
            h.insert(k.to_string(), v.to_string());
        }
        PluginCtx::new(
            "GET".into(),
            "/x".into(),
            h,
            Bytes::new(),
            "10.0.0.7".into(),
        )
    }

    #[test]
    fn uuid_generator_produces_valid_uuid() {
        let p = CorrelationIdPlugin::new();
        let id = p.generate("uuid", "10.0.0.7", 1000);
        assert!(Uuid::parse_str(&id).is_ok(), "expected a valid uuid, got {id:?}");
    }

    #[test]
    fn uuid_counter_prefixed_with_worker_uuid_and_increments() {
        let p = CorrelationIdPlugin::new();
        let a = p.generate("uuid#counter", "10.0.0.7", 1000);
        let b = p.generate("uuid#counter", "10.0.0.7", 1000);
        assert!(a.starts_with(&p.worker_uuid), "missing worker uuid prefix: {a}");
        assert!(a.contains('#'), "missing # separator: {a}");
        // Counter portion increments and the two ids differ.
        assert_ne!(a, b);
        let n_a: u64 = a.rsplit('#').next().unwrap().parse().unwrap();
        let n_b: u64 = b.rsplit('#').next().unwrap().parse().unwrap();
        assert_eq!(n_b, n_a + 1);
    }

    #[test]
    fn tracker_generator_is_composite_and_unique() {
        let p = CorrelationIdPlugin::new();
        let a = p.generate("tracker", "10.0.0.7", 1717);
        let b = p.generate("tracker", "10.0.0.7", 1717);
        assert!(a.contains("10.0.0.7"), "tracker missing client ip: {a}");
        assert!(a.contains("1717"), "tracker missing timestamp: {a}");
        assert_ne!(a, b, "tracker ids must be unique per request");
    }

    #[test]
    fn unknown_generator_defaults_to_uuid() {
        let p = CorrelationIdPlugin::new();
        let id = p.generate("bogus", "10.0.0.7", 1000);
        assert!(Uuid::parse_str(&id).is_ok());
    }

    #[tokio::test]
    async fn access_generates_header_when_absent() {
        let p = CorrelationIdPlugin::new();
        let mut c = ctx(&[]);
        let cfg = serde_json::json!({ "generator": "uuid" });
        let _ = p.access(&mut c, &cfg).await;
        let v = c.headers.get(&DEFAULT_HEADER_NAME.to_lowercase());
        assert!(v.is_some(), "header should be generated on upstream request");
        assert!(Uuid::parse_str(v.unwrap()).is_ok());
    }

    #[tokio::test]
    async fn access_preserves_existing_header() {
        let p = CorrelationIdPlugin::new();
        let mut c = ctx(&[("kong-request-id", "client-supplied-123")]);
        let cfg = serde_json::json!({});
        let _ = p.access(&mut c, &cfg).await;
        assert_eq!(
            c.headers.get("kong-request-id").map(String::as_str),
            Some("client-supplied-123"),
            "client-supplied correlation id must be preserved"
        );
    }

    #[tokio::test]
    async fn access_honours_custom_header_name() {
        let p = CorrelationIdPlugin::new();
        let mut c = ctx(&[]);
        let cfg = serde_json::json!({ "header_name": "X-Trace", "generator": "uuid" });
        let _ = p.access(&mut c, &cfg).await;
        assert!(c.headers.get("x-trace").is_some());
    }

    #[tokio::test]
    async fn header_filter_echoes_when_enabled() {
        let p = CorrelationIdPlugin::new();
        let mut c = ctx(&[]);
        let cfg = serde_json::json!({ "generator": "uuid", "echo_downstream": true });
        let _ = p.access(&mut c, &cfg).await;
        p.header_filter(&mut c, &cfg).await;
        let upstream = c.headers.get("kong-request-id").cloned().unwrap();
        let echoed = c.response_headers.get("kong-request-id").cloned();
        assert_eq!(echoed, Some(upstream), "echo_downstream must mirror id to response");
    }

    #[tokio::test]
    async fn header_filter_no_echo_by_default() {
        let p = CorrelationIdPlugin::new();
        let mut c = ctx(&[]);
        let cfg = serde_json::json!({ "generator": "uuid" });
        let _ = p.access(&mut c, &cfg).await;
        p.header_filter(&mut c, &cfg).await;
        assert!(
            c.response_headers.get("kong-request-id").is_none(),
            "echo_downstream defaults to false"
        );
    }
}
