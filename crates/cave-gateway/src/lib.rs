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
        }
    }
}

pub fn router(state: Arc<GatewayState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "gateway";
