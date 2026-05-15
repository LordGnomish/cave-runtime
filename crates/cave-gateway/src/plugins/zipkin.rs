// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! zipkin plugin — inject B3 trace headers and report spans.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

pub struct ZipkinPlugin;

fn random_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

#[async_trait]
impl GatewayPlugin for ZipkinPlugin {
    fn name(&self) -> &'static str {
        "zipkin"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let sample_ratio = config["sample_ratio"].as_f64().unwrap_or(1.0);

        // Propagate or generate trace context
        let trace_id = ctx
            .headers
            .get("x-b3-traceid")
            .cloned()
            .unwrap_or_else(|| format!("{}{}", random_id(), random_id()));

        let parent_span_id = ctx.headers.get("x-b3-spanid").cloned();
        let span_id = random_id();
        let sampled = ctx
            .headers
            .get("x-b3-sampled")
            .map(|v| v == "1")
            .unwrap_or_else(|| rand::random::<f64>() < sample_ratio);

        // Inject B3 headers into upstream request
        ctx.headers.insert("x-b3-traceid".to_string(), trace_id.clone());
        ctx.headers.insert("x-b3-spanid".to_string(), span_id.clone());
        if let Some(pid) = parent_span_id {
            ctx.headers.insert("x-b3-parentspanid".to_string(), pid);
        }
        ctx.headers.insert("x-b3-sampled".to_string(), if sampled { "1" } else { "0" }.to_string());

        // Preserve for log phase
        ctx.ctx.insert("zipkin_trace_id".to_string(), Value::String(trace_id));
        ctx.ctx.insert("zipkin_span_id".to_string(), Value::String(span_id));
        ctx.ctx.insert("zipkin_sampled".to_string(), Value::Bool(sampled));
        ctx.ctx.insert("zipkin_start_ms".to_string(), Value::Number(
            serde_json::Number::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ),
        ));

        PluginResult::Continue
    }

    async fn log(&self, ctx: &PluginCtx, config: &Value) {
        let sampled = ctx.ctx.get("zipkin_sampled").and_then(|v| v.as_bool()).unwrap_or(false);
        if !sampled {
            return;
        }

        let endpoint = match config["http_endpoint"].as_str() {
            Some(e) => e.to_string(),
            None => return,
        };

        let trace_id = ctx.ctx.get("zipkin_trace_id").and_then(|v| v.as_str()).unwrap_or("");
        let span_id = ctx.ctx.get("zipkin_span_id").and_then(|v| v.as_str()).unwrap_or("");
        let start_ms = ctx.ctx.get("zipkin_start_ms").and_then(|v| v.as_u64()).unwrap_or(0);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let span = json!([{
            "traceId": trace_id,
            "id": span_id,
            "name": format!("{} {}", ctx.method, ctx.path),
            "timestamp": start_ms * 1000,
            "duration": (now_ms - start_ms) * 1000,
            "kind": "SERVER",
            "localEndpoint": {
                "serviceName": config["local_service_name"].as_str().unwrap_or("cave-gateway"),
            },
            "tags": {
                "http.method": ctx.method,
                "http.path": ctx.path,
                "http.status_code": ctx.response_status.unwrap_or(0).to_string(),
            },
        }]);

        let client = reqwest::Client::new();
        let _ = client
            .post(&endpoint)
            .header("content-type", "application/json")
            .json(&span)
            .send()
            .await;
    }
}
