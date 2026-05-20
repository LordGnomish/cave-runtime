// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Main proxy request handler — the hot path.
//!
//! 1. Match route
//! 2. Run plugin access phase
//! 3. Pick upstream endpoint (load balance)
//! 4. Forward request with retry
//! 5. Run response plugin phases
//! 6. Return response

use crate::matcher::{compile_route, match_request, upstream_path};
use crate::models::{Protocol, Service};
use crate::plugins::{PluginChain, PluginCtx};
use crate::proxy::ProxyEngine;
use crate::store::SharedStore;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

pub struct GatewayHandlerState {
    pub store: SharedStore,
    pub proxy: Arc<ProxyEngine>,
    pub plugins: Arc<PluginChain>,
}

/// The catch-all proxy handler — matches routes and forwards requests.
pub async fn proxy_handler(
    State(state): State<Arc<GatewayHandlerState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let method = req.method().to_string();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();
    let full_path = if query.is_empty() {
        path.clone()
    } else {
        format!("{}?{}", path, query)
    };
    let client_ip = addr.ip().to_string();

    // Collect headers
    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|val| (k.as_str().to_lowercase(), val.to_string()))
        })
        .collect();

    let protocol =
        if headers.get("upgrade").map(|v| v.to_lowercase()) == Some("websocket".to_string()) {
            Protocol::Ws
        } else if headers
            .get("content-type")
            .map(|v| v.starts_with("application/grpc"))
            == Some(true)
        {
            Protocol::Grpc
        } else {
            Protocol::Http
        };

    // Collect body (buffered)
    let body_bytes = match axum::body::to_bytes(req.into_body(), 128 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    // Build plugin context
    let mut ctx = PluginCtx::new(
        method.clone(),
        full_path.clone(),
        headers.clone(),
        body_bytes.clone(),
        client_ip.clone(),
    );

    // Compile routes for matching
    let routes = state.store.list_routes();
    let compiled: Vec<_> = routes.iter().map(compile_route).collect();

    let host = headers.get("host").cloned().unwrap_or_default();
    let match_result = match_request(&compiled, &method, &host, &path, &headers, &protocol, None);

    let match_result = match match_result {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(
                    serde_json::json!({"message": "no route and no API found with those values"}),
                ),
            )
                .into_response();
        }
    };

    ctx.route_id = Some(match_result.route_id);

    // Resolve service
    let service = match match_result
        .service_id
        .and_then(|id| state.store.services.get(&id).map(|e| e.value().clone()))
    {
        Some(s) => s,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"message": "service not found for route"})),
            )
                .into_response();
        }
    };

    ctx.service_id = Some(service.id);

    // Gather applicable plugins (global + service + route)
    let plugin_configs: Vec<(String, serde_json::Value)> = state
        .store
        .global_plugins()
        .into_iter()
        .chain(state.store.plugins_for_service(&service.id))
        .chain(state.store.plugins_for_route(&match_result.route_id))
        .filter(|p| p.enabled)
        .map(|p| (p.name.clone(), p.config.clone()))
        .collect();

    // Run access phase
    if let Some(halt_response) = state.plugins.run_access(&mut ctx, &plugin_configs).await {
        return halt_response;
    }

    // Resolve upstream
    let _upstream_url = resolve_upstream_url(&state.store, &service, &match_result, &path);

    // Proxy the request
    let http_method =
        reqwest::Method::from_bytes(ctx.method.as_bytes()).unwrap_or(reqwest::Method::GET);

    let mut forward_headers = HeaderMap::new();
    for (k, v) in &ctx.headers {
        if let (Ok(name), Ok(val)) = (
            axum::http::HeaderName::from_bytes(k.as_bytes()),
            axum::http::HeaderValue::from_str(v),
        ) {
            forward_headers.insert(name, val);
        }
    }

    // Add forwarding headers
    if !match_result.preserve_host {
        if let Some(host) = service_host(&service) {
            if let Ok(val) = axum::http::HeaderValue::from_str(&host) {
                forward_headers.insert(axum::http::header::HOST, val);
            }
        }
    }
    let _ = forward_headers.insert(
        axum::http::HeaderName::from_static("x-forwarded-for"),
        axum::http::HeaderValue::from_str(&client_ip)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("unknown")),
    );
    let _ = forward_headers.insert(
        axum::http::HeaderName::from_static("x-real-ip"),
        axum::http::HeaderValue::from_str(&client_ip)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("unknown")),
    );

    // Pick endpoint for LB
    let (target_id, upstream_url) = {
        // Try to find a named upstream first
        if let Some(upstream) = state.store.get_upstream_by_id_or_name(&service.host) {
            let targets = state.store.targets_for_upstream(&upstream.id);
            if let Some(ep) = state.proxy.pick_endpoint(&upstream, &targets, None) {
                let url = format!(
                    "{}://{}:{}{}",
                    scheme_for_service(&service),
                    ep.host,
                    ep.port,
                    upstream_path(&match_result, &path, service.path.as_deref())
                );
                (ep.target_id, url)
            } else {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
        } else {
            // Direct to service host
            let up_path = upstream_path(&match_result, &path, service.path.as_deref());
            let url = format!(
                "{}://{}:{}{}",
                scheme_for_service(&service),
                service.host,
                service.port,
                up_path
            );
            (uuid::Uuid::new_v4(), url)
        }
    };

    let response = state
        .proxy
        .proxy_with_retry(
            &upstream_url,
            http_method,
            forward_headers,
            ctx.body.clone(),
            target_id,
            service.retries,
        )
        .await;

    // Collect response status/headers/body for plugin log phase
    let status = response.status().as_u16();
    ctx.response_status = Some(status);

    // Run log phase (async, not awaited for performance)
    let plugins = state.plugins.clone();
    let plugin_configs_clone = plugin_configs.clone();
    let ctx_clone = ctx.clone();
    tokio::spawn(async move {
        plugins.run_log(&ctx_clone, &plugin_configs_clone).await;
    });

    response
}

fn resolve_upstream_url(
    _store: &SharedStore,
    service: &Service,
    match_result: &crate::matcher::MatchResult,
    path: &str,
) -> String {
    let up_path = upstream_path(match_result, path, service.path.as_deref());
    format!(
        "{}://{}:{}{}",
        scheme_for_service(service),
        service.host,
        service.port,
        up_path
    )
}

fn scheme_for_service(service: &Service) -> &'static str {
    match service.protocol {
        Protocol::Https | Protocol::Grpcs | Protocol::Wss => "https",
        Protocol::Grpc => "http",
        Protocol::Ws => "ws",
        _ => "http",
    }
}

fn service_host(service: &Service) -> Option<String> {
    Some(format!("{}:{}", service.host, service.port))
}
