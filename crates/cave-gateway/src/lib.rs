//! CAVE Gateway — Kong + Gravitee API Gateway replacement.
//!
//! Full feature set:
//!  - Reverse proxy with upstream load balancing (round-robin, consistent-hash, least-conn)
//!  - Route matching: host, path (prefix + regex), methods, headers, SNI
//!  - Admin API: full CRUD for services, routes, upstreams, targets, consumers, plugins
//!  - 20+ built-in plugins: auth, traffic, transform, logging, security
//!  - Consumer management with credentials (key-auth, JWT, basic-auth, HMAC, OAuth2)
//!  - Active + passive health checks
//!  - Circuit breakers per upstream target
//!  - Developer portal (Gravitee): API catalog, subscriptions, API keys, docs
//!  - API versioning and deprecation lifecycle
//!  - Usage tracking and monetization hooks
//!  - Protocol support: HTTP/1.1, HTTP/2, WebSocket proxying, gRPC proxying
//!
//! ## Upstream tracking
//!  - Kong: https://github.com/Kong/kong (v3.x parity target)
//!  - Gravitee: https://github.com/gravitee-io/gravitee-api-management (v4.x parity target)

pub mod admin;
pub mod engine;
pub mod health;
pub mod matcher;
pub mod models;
pub mod plugins;
pub mod portal;
pub mod store;

use axum::{routing::get, Json, Router};
use cave_db::CavePool;
use std::sync::{Arc, RwLock};

/// Shared state for all gateway handlers.
pub struct GatewayState {
    /// Central in-memory store for all gateway entities
    pub store: Arc<RwLock<store::GatewayStore>>,
    /// Load-balancing state per upstream
    pub lb_state: Arc<std::sync::Mutex<std::collections::HashMap<uuid::Uuid, engine::LbState>>>,
    /// Circuit-breaker registry per target
    pub circuit_breakers: Arc<std::sync::Mutex<engine::CircuitBreakerRegistry>>,
    /// Health registry for passive + active health tracking
    pub health: Arc<std::sync::Mutex<health::HealthRegistry>>,
    /// Plugin runtime state (rate limiters, caches)
    pub plugin_state: Arc<std::sync::Mutex<plugins::PluginState>>,
    /// Optional cave-db pool for persistence
    pub pool: Option<Arc<CavePool>>,
}

impl GatewayState {
    pub fn new(pool: Option<Arc<CavePool>>) -> Self {
        Self {
            store: Arc::new(RwLock::new(store::GatewayStore::default())),
            lb_state: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            circuit_breakers: Arc::new(std::sync::Mutex::new(engine::CircuitBreakerRegistry::default())),
            health: Arc::new(std::sync::Mutex::new(health::HealthRegistry::default())),
            plugin_state: Arc::new(std::sync::Mutex::new(plugins::PluginState::default())),
            pool,
        }
    }
}

/// Cave Gateway module name for DB schema
pub const MODULE_NAME: &str = "gateway";

/// Create the full Axum router for the gateway module.
///
/// Mounts:
///   /gateway/admin/**   — Kong-compatible admin API
///   /gateway/portal/**  — Gravitee-compatible developer portal
///   /gateway/health     — module health check
pub fn router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .nest("/gateway/admin", admin::admin_router(Arc::clone(&state)))
        .nest("/gateway/portal", portal::portal_router(Arc::clone(&state)))
        .nest(
            "/gateway/admin",
            portal::lifecycle_router(Arc::clone(&state)),
        )
        .route("/gateway/health", get(health_check))
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-gateway",
        "status": "ok",
        "upstream_kong": "3.x",
        "upstream_gravitee": "4.x",
        "features": [
            "reverse-proxy",
            "load-balancing",
            "route-matching",
            "plugin-system",
            "consumer-management",
            "health-checks",
            "circuit-breaker",
            "developer-portal",
            "api-lifecycle",
            "monetization",
            "websocket-proxying",
            "grpc-proxying"
        ]
    }))
}
