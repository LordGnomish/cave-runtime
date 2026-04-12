//! API gateway & traffic management — Kong + Gravitee unified.
//!
//! Supersedes: Kong API Gateway + Gravitee API Platform
//! Upstream tracking: see cave-upstream for monitored features.
//!
//! Kong features:
//! - Route definitions and upstream service registry
//! - Rate limiting: token bucket and sliding window
//! - Auth proxy: JWT validation, API key auth, OAuth2 passthrough
//! - Load balancing: round-robin, least-connections, weighted
//! - Circuit breaker pattern per upstream
//! - Plugin system: CORS, request-size-limit, IP restriction, bot detection,
//!   request/response transformation
//!
//! Gravitee features (beyond Kong):
//! - API Designer / API-first studio with OpenAPI/AsyncAPI spec management
//! - Automated API quality scoring across documentation, security, and design
//! - Developer Portal: self-service API catalog, consumer management, subscription plans
//! - API Monetization: usage-based billing, tiered pricing, invoice generation
//! - API Lifecycle Management: Draft → PendingReview → Published → Deprecated → Retired
//! - Review & Approval Workflow with audit trail
//! - Multi-Protocol Gateway: HTTP, gRPC, WebSocket, GraphQL, MQTT, SSE
//! - Flow-based Policy Designer: pre-route → route → post-route → error chains
//!
//! All state is held in-memory with Arc<Mutex<...>> per subsystem.

pub mod api_designer;
pub mod flows;
pub mod gateway;
pub mod lifecycle;
pub mod marketplace;
pub mod models;
pub mod monetization;
pub mod protocols;
pub mod routes;

use axum::Router;
use gateway::GatewayEngine;
use std::sync::{Arc, Mutex};

pub struct GatewayState {
    // Kong engine
    pub engine: Arc<Mutex<GatewayEngine>>,
    // Gravitee extensions
    pub designer: Arc<Mutex<api_designer::ApiDesignerStore>>,
    pub marketplace: Arc<Mutex<marketplace::MarketplaceStore>>,
    pub monetization: Arc<Mutex<monetization::MonetizationStore>>,
    pub lifecycle: Arc<Mutex<lifecycle::LifecycleStore>>,
    pub protocols: Arc<Mutex<protocols::ProtocolStore>>,
    pub flows: Arc<Mutex<flows::FlowStore>>,
}

impl Default for GatewayState {
    fn default() -> Self {
        Self {
            engine: Arc::new(Mutex::new(GatewayEngine::new())),
            designer: Arc::new(Mutex::new(api_designer::ApiDesignerStore::new())),
            marketplace: Arc::new(Mutex::new(marketplace::MarketplaceStore::new())),
            monetization: Arc::new(Mutex::new(monetization::MonetizationStore::new())),
            lifecycle: Arc::new(Mutex::new(lifecycle::LifecycleStore::new())),
            protocols: Arc::new(Mutex::new(protocols::ProtocolStore::new())),
            flows: Arc::new(Mutex::new(flows::FlowStore::new())),
//! CAVE Gateway — Kong + Gravitee API Gateway replacement.
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
//! ## Upstream tracking
//!  - Kong: https://github.com/Kong/kong (v3.x parity target)
//!  - Gravitee: https://github.com/gravitee-io/gravitee-api-management (v4.x parity target)
pub mod admin;
pub mod engine;
pub mod health;
pub mod matcher;
pub mod plugins;
pub mod portal;
pub mod store;
use axum::{routing::get, Json, Router};
use cave_db::CavePool;
use std::sync::{Arc, RwLock};
/// Shared state for all gateway handlers.
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
impl GatewayState {
    pub fn new(pool: Option<Arc<CavePool>>) -> Self {
            store: Arc::new(RwLock::new(store::GatewayStore::default())),
            lb_state: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            circuit_breakers: Arc::new(std::sync::Mutex::new(engine::CircuitBreakerRegistry::default())),
            health: Arc::new(std::sync::Mutex::new(health::HealthRegistry::default())),
            plugin_state: Arc::new(std::sync::Mutex::new(plugins::PluginState::default())),
            pool,
        }
    }
}

pub fn router(state: Arc<GatewayState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "gateway";
/// Cave Gateway module name for DB schema
/// Create the full Axum router for the gateway module.
///
/// Mounts:
///   /gateway/admin/**   — Kong-compatible admin API
///   /gateway/portal/**  — Gravitee-compatible developer portal
///   /gateway/health     — module health check
    Router::new()
        .nest("/gateway/admin", admin::admin_router(Arc::clone(&state)))
        .nest("/gateway/portal", portal::portal_router(Arc::clone(&state)))
        .nest(
            "/gateway/admin",
            portal::lifecycle_router(Arc::clone(&state)),
        )
        .route("/gateway/health", get(health_check))
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
