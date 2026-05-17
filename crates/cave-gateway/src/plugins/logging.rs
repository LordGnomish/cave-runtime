// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Logging plugins: http-log, file-log, tcp-log, syslog.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

fn build_log_entry(ctx: &PluginCtx) -> Value {
    json!({
        "request": {
            "method": ctx.method,
            "path": ctx.path,
            "querystring": ctx.query,
            "headers": ctx.headers,
            "size": ctx.body.len(),
            "id": ctx.request_id,
        },
        "response": {
            "status": ctx.response_status,
            "size": ctx.response_body.len(),
        },
        "consumer": {
            "id": ctx.consumer_id,
            "username": ctx.consumer_username,
        },
        "service": {
            "id": ctx.service_id,
        },
        "route": {
            "id": ctx.route_id,
        },
        "client_ip": ctx.client_ip,
        "started_at": chrono::Utc::now().timestamp_millis(),
    })
}

// ── HTTP log ──────────────────────────────────────────────────────────────────

pub struct HttpLogPlugin;

#[async_trait]
impl GatewayPlugin for HttpLogPlugin {
    fn name(&self) -> &'static str {
        "http-log"
    }

    async fn log(&self, ctx: &PluginCtx, config: &Value) {
        let endpoint = match config["http_endpoint"].as_str() {
            Some(e) => e.to_string(),
            None => return,
        };

        let payload = build_log_entry(ctx);
        let client = reqwest::Client::new();
        let method = config["method"].as_str().unwrap_or("POST");
        let timeout_ms = config["timeout"].as_u64().unwrap_or(10000);

        let req = match method {
            "PUT" => client.put(&endpoint),
            "PATCH" => client.patch(&endpoint),
            _ => client.post(&endpoint),
        };

        let _ = req
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .json(&payload)
            .send()
            .await;
    }
}

// ── File log ──────────────────────────────────────────────────────────────────

pub struct FileLogPlugin;

#[async_trait]
impl GatewayPlugin for FileLogPlugin {
    fn name(&self) -> &'static str {
        "file-log"
    }

    async fn log(&self, ctx: &PluginCtx, config: &Value) {
        let path = config["path"].as_str().unwrap_or("/tmp/cave-gateway.log");
        let reopen = config["reopen"].as_bool().unwrap_or(false);
        let payload = build_log_entry(ctx).to_string();

        // Async file write
        use tokio::io::AsyncWriteExt;
        if let Ok(mut file) = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
        {
            let _ = file.write_all(format!("{}\n", payload).as_bytes()).await;
        }
    }
}

// ── Structured access log (always-on) ────────────────────────────────────────

pub struct AccessLogPlugin;

#[async_trait]
impl GatewayPlugin for AccessLogPlugin {
    fn name(&self) -> &'static str {
        "access-log"
    }

    async fn log(&self, ctx: &PluginCtx, _config: &Value) {
        info!(
            method = %ctx.method,
            path = %ctx.path,
            status = ?ctx.response_status,
            client_ip = %ctx.client_ip,
            consumer = ?ctx.consumer_username,
            request_id = %ctx.request_id,
            "request completed"
        );
    }
}
