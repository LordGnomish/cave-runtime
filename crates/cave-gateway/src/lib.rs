//! CAVE Gateway — Kong + Gravitee parity API gateway (charter-aligned).
//!
//! Per ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 the canonical upstream pair
//! is Kong (proxy data path + plugin DSL) and Gravitee (Developer Portal +
//! API lifecycle + IAM). Envoy was rejected in seven ADRs and the previous
//! xDS surface was removed on 2026-05-02 to keep the crate charter-aligned.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                       cave-gateway                           │
//! │                                                              │
//! │  ┌─────────────┐  ┌──────────────┐  ┌──────────────────┐    │
//! │  │  Admin API  │  │  Proxy       │  │  Gravitee API    │    │
//! │  │  :8001      │  │  :8000       │  │  :8002           │    │
//! │  │  Kong CRUD  │  │  HTTP/WS/    │  │  apis / plans /  │    │
//! │  │             │  │  gRPC/TCP    │  │  apps / subs     │    │
//! │  └──────┬──────┘  └──────┬───────┘  └────────┬─────────┘    │
//! │         │                │                   │              │
//! │         └────────────────┴───────────────────┘              │
//! │                          │                                   │
//! │                   ┌──────▼──────┐                           │
//! │                   │ GatewayStore│  (in-memory, thread-safe) │
//! │                   └─────────────┘                           │
//! │                                                              │
//! │  Plugin pipeline (per-request):                              │
//! │  rate-limiting → key-auth → jwt → acl → cors → ...          │
//! │  → proxy → response-transformer → prometheus → zipkin        │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Upstream tracking: Kong + Gravitee
//! - Kong:     https://github.com/Kong/kong            (v3.5.0)
//! - Gravitee: https://github.com/gravitee-io/gravitee-api-management (v4.x)

pub mod admin;
pub mod circuit_breaker;
pub mod gravitee;
pub mod handler;
pub mod health;
pub mod lb;
pub mod matcher;
pub mod models;
pub mod plugins;
pub mod proxy;
pub mod store;
pub mod tls;

use axum::{routing::any, Router};
use handler::{proxy_handler, GatewayHandlerState};
use plugins::PluginChain;
use proxy::ProxyEngine;
use std::sync::Arc;
use store::{GatewayStore, SharedStore};

pub const MODULE_NAME: &str = "gateway";

/// Top-level gateway state.
pub struct GatewayState {
    pub store: SharedStore,
    pub proxy: Arc<ProxyEngine>,
    pub plugins: Arc<PluginChain>,
    pub tls_resolver: Arc<tls::SniCertResolver>,
    pub acme_challenges: Arc<tls::AcmeChallengeStore>,
}

impl GatewayState {
    pub fn new() -> Arc<Self> {
        let store = GatewayStore::new();
        let plugins = PluginChain::new();
        let proxy = ProxyEngine::new(store.clone(), plugins.clone());
        let tls_resolver = tls::SniCertResolver::new();
        let acme_challenges = Arc::new(tls::AcmeChallengeStore::default());

        Arc::new(Self {
            store,
            proxy,
            plugins,
            tls_resolver,
            acme_challenges,
        })
    }
}

impl Default for GatewayState {
    fn default() -> Self {
        Arc::try_unwrap(GatewayState::new()).unwrap_or_else(|s| {
            // This path shouldn't happen since new() creates a fresh Arc
            panic!("GatewayState::default called with existing references")
        })
    }
}

/// Build the proxy router (handles all traffic forwarding).
pub fn proxy_router(state: Arc<GatewayState>) -> Router {
    let handler_state = Arc::new(GatewayHandlerState {
        store: state.store.clone(),
        proxy: state.proxy.clone(),
        plugins: state.plugins.clone(),
    });

    Router::new()
        // ACME HTTP-01 challenges
        .route(
            "/.well-known/acme-challenge/{token}",
            axum::routing::get({
                let acme = state.acme_challenges.clone();
                move |path| tls::acme_challenge_handler(path, axum::extract::State(acme.clone()))
            }),
        )
        // Catch-all proxy handler
        .fallback(any(proxy_handler))
        .with_state(handler_state)
}

/// Build the Admin API router (Kong-compatible management plane).
pub fn admin_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .nest("/admin/v1", admin::admin_router(state.store.clone()))
        // Kong-style: also mount at root for compat
        // removed: root nest breaks axum 0.8
        // .nest("", admin::admin_router(state.store.clone()))
}

/// Build the Gravitee API/plan/application/subscription router.
pub fn gravitee_router(state: Arc<GatewayState>) -> Router {
    gravitee::apis::router(state.store.clone())
}

/// Create a unified router with all gateway components merged.
/// In production, Admin / Gravitee / proxy planes typically run on
/// separate ports.
pub fn router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .merge(admin_router(state.clone()))
        .merge(gravitee_router(state.clone()))
        .merge(proxy_router(state))
}
