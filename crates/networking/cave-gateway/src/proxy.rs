// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core reverse proxy engine.
//!
//! Handles HTTP/1.1, HTTP/2, WebSocket, gRPC, and TCP stream proxying.
//! Uses reqwest for HTTP upstream calls.
//! WebSocket upgrade is performed via tokio-tungstenite.

use crate::circuit_breaker::CircuitBreakerRegistry;
use crate::health::HealthRegistry;
use crate::lb::{Balancer, Endpoint};
use crate::models::{LbAlgorithm, Protocol, Service, Target, Upstream};
use crate::plugins::PluginChain;
use crate::store::GatewayStore;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

pub type ProxyState = Arc<ProxyEngine>;

pub struct ProxyEngine {
    pub store: Arc<GatewayStore>,
    pub health: HealthRegistry,
    pub circuit_breakers: CircuitBreakerRegistry,
    pub balancers: DashMap<Uuid, Arc<Balancer>>,
    pub http_client: reqwest::Client,
    pub plugin_chain: Arc<PluginChain>,
}

impl ProxyEngine {
    pub fn new(store: Arc<GatewayStore>, plugin_chain: Arc<PluginChain>) -> Arc<Self> {
        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build HTTP client");

        Arc::new(Self {
            store,
            health: HealthRegistry::new(),
            circuit_breakers: CircuitBreakerRegistry::default(),
            balancers: DashMap::new(),
            http_client,
            plugin_chain,
        })
    }

    /// Get or create a balancer for the given upstream.
    pub fn balancer_for(&self, upstream: &Upstream) -> Arc<Balancer> {
        self.balancers
            .entry(upstream.id)
            .or_insert_with(|| Arc::new(Balancer::new(&upstream.algorithm)))
            .value()
            .clone()
    }

    /// Pick a healthy endpoint from the upstream's targets.
    pub fn pick_endpoint(
        &self,
        upstream: &Upstream,
        targets: &[Target],
        hash_key: Option<u64>,
    ) -> Option<Endpoint> {
        let healthy_endpoints: Vec<Endpoint> = targets
            .iter()
            .filter(|t| {
                t.weight > 0
                    && self.health.is_healthy(upstream.id, t.id)
                    && self.circuit_breakers.allow(t.id)
            })
            .map(Endpoint::from)
            .collect();

        if healthy_endpoints.is_empty() {
            // Fallback: try all endpoints regardless of health
            let all: Vec<Endpoint> = targets
                .iter()
                .filter(|t| t.weight > 0)
                .map(Endpoint::from)
                .collect();
            let balancer = self.balancer_for(upstream);
            return balancer.pick(&all, hash_key).cloned();
        }

        let balancer = self.balancer_for(upstream);
        balancer.pick(&healthy_endpoints, hash_key).cloned()
    }

    /// Proxy a standard HTTP request to an upstream service.
    pub async fn proxy_http(
        &self,
        upstream_url: &str,
        method: reqwest::Method,
        headers: HeaderMap,
        body: Bytes,
        target_id: Uuid,
    ) -> Result<Response, StatusCode> {
        self.circuit_breakers
            .allow(target_id)
            .then_some(())
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

        let mut req_builder = self.http_client.request(method, upstream_url);

        // Forward headers (skip hop-by-hop)
        for (name, value) in &headers {
            if !is_hop_by_hop(name.as_str()) {
                req_builder = req_builder.header(name, value);
            }
        }

        req_builder = req_builder.body(body);

        match req_builder.send().await {
            Ok(resp) => {
                let status = StatusCode::from_u16(resp.status().as_u16())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let mut response_headers = HeaderMap::new();
                for (name, value) in resp.headers() {
                    if !is_hop_by_hop(name.as_str()) {
                        response_headers.insert(name.clone(), value.clone());
                    }
                }
                let body_bytes = resp.bytes().await.unwrap_or_default();

                self.circuit_breakers.on_success(target_id);

                let mut response = Response::builder().status(status);
                for (name, value) in response_headers {
                    if let Some(n) = name {
                        response = response.header(n, value);
                    }
                }
                response
                    .body(Body::from(body_bytes))
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            }
            Err(e) => {
                error!(url=%upstream_url, err=%e, "upstream request failed");
                self.circuit_breakers.on_failure(target_id);
                if e.is_timeout() {
                    Err(StatusCode::GATEWAY_TIMEOUT)
                } else if e.is_connect() {
                    Err(StatusCode::BAD_GATEWAY)
                } else {
                    Err(StatusCode::BAD_GATEWAY)
                }
            }
        }
    }

    /// Proxy with retry + backoff.
    pub async fn proxy_with_retry(
        &self,
        upstream_url: &str,
        method: reqwest::Method,
        headers: HeaderMap,
        body: Bytes,
        target_id: Uuid,
        retries: u32,
    ) -> Response {
        let mut last_status = StatusCode::BAD_GATEWAY;
        let mut delay = Duration::from_millis(50);

        for attempt in 0..=retries {
            match self
                .proxy_http(
                    upstream_url,
                    method.clone(),
                    headers.clone(),
                    body.clone(),
                    target_id,
                )
                .await
            {
                Ok(resp) => {
                    if attempt > 0 {
                        debug!(attempt, "proxy succeeded after retry");
                    }
                    return resp;
                }
                Err(status) => {
                    last_status = status;
                    if attempt < retries {
                        // Exponential backoff: 50ms, 100ms, 200ms, ...
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(5));
                    }
                }
            }
        }

        last_status.into_response()
    }
}

fn is_hop_by_hop(header: &str) -> bool {
    matches!(
        header.to_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// WebSocket proxy — upgrades the client connection then forwards frames bidirectionally.
pub async fn proxy_websocket(
    ws: axum::extract::ws::WebSocketUpgrade,
    upstream_url: String,
) -> Response {
    use axum::extract::ws::Message as ClientMsg;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as UpMsg;

    ws.on_upgrade(move |socket| async move {
        let (upstream_ws, _) = match connect_async(upstream_url.as_str()).await {
            Ok(v) => v,
            Err(e) => {
                error!(url=%upstream_url, err=%e, "WebSocket upstream connect failed");
                return;
            }
        };

        let (mut up_sink, mut up_stream) = upstream_ws.split();
        let (mut client_sink, mut client_stream) = socket.split();

        // client → upstream
        let c2u = async move {
            while let Some(Ok(msg)) = client_stream.next().await {
                let up_msg = match msg {
                    ClientMsg::Text(t) => UpMsg::Text(t.to_string().into()),
                    ClientMsg::Binary(b) => UpMsg::Binary(b.to_vec().into()),
                    ClientMsg::Ping(p) => UpMsg::Ping(p.to_vec().into()),
                    ClientMsg::Pong(p) => UpMsg::Pong(p.to_vec().into()),
                    ClientMsg::Close(_) => {
                        let _ = up_sink.send(UpMsg::Close(None)).await;
                        break;
                    }
                };
                if up_sink.send(up_msg).await.is_err() {
                    break;
                }
            }
        };

        // upstream → client
        let u2c = async move {
            while let Some(Ok(msg)) = up_stream.next().await {
                let client_msg = match msg {
                    UpMsg::Text(t) => ClientMsg::Text(t.to_string().into()),
                    UpMsg::Binary(b) => ClientMsg::Binary(b.to_vec().into()),
                    UpMsg::Ping(p) => ClientMsg::Ping(p.to_vec().into()),
                    UpMsg::Pong(p) => ClientMsg::Pong(p.to_vec().into()),
                    UpMsg::Close(_) | UpMsg::Frame(_) => break,
                };
                if client_sink.send(client_msg).await.is_err() {
                    break;
                }
            }
        };

        tokio::select! {
            _ = c2u => {}
            _ = u2c => {}
        }
    })
}

/// gRPC passthrough proxy — forwards the entire body as-is using
/// the binary framing of HTTP/2 gRPC. For full transcoding see grpc_gateway.rs.
pub async fn proxy_grpc(
    engine: &ProxyEngine,
    upstream_url: &str,
    path: &str,
    headers: HeaderMap,
    body: Bytes,
    target_id: Uuid,
) -> Response {
    let full_url = format!("{}{}", upstream_url.trim_end_matches('/'), path);

    let mut req = engine
        .http_client
        .post(&full_url)
        .header("content-type", "application/grpc")
        .header("te", "trailers");

    for (k, v) in &headers {
        if !is_hop_by_hop(k.as_str()) && k.as_str() != "content-type" {
            req = req.header(k, v);
        }
    }

    match req.body(body).send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let mut rb = Response::builder().status(status);
            for (k, v) in resp.headers() {
                rb = rb.header(k, v);
            }
            let b = resp.bytes().await.unwrap_or_default();
            engine.circuit_breakers.on_success(target_id);
            rb.body(Body::from(b))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            engine.circuit_breakers.on_failure(target_id);
            error!(url=%full_url, err=%e, "gRPC upstream failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_by_hop_headers() {
        assert!(is_hop_by_hop("connection"));
        assert!(is_hop_by_hop("Transfer-Encoding"));
        assert!(!is_hop_by_hop("content-type"));
        assert!(!is_hop_by_hop("authorization"));
    }
}
