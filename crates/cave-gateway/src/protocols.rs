//! Gravitee Multi-Protocol Gateway — HTTP, gRPC, WebSocket, GraphQL, MQTT, and
//! SSE as first-class citizens with unified routing and policy enforcement.

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ── Store ─────────────────────────────────────────────────────────────────────

pub struct ProtocolStore {
    pub endpoints: HashMap<Uuid, ProtocolEndpoint>,
    /// Per-protocol message routing decisions for analytics.
    pub message_log: Vec<MessageRoutingEvent>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageRoutingEvent {
    pub endpoint_id: Uuid,
    pub protocol: String,
    pub topic_or_path: String,
    pub routed_to: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

impl ProtocolStore {
    pub fn new() -> Self {
        Self {
            endpoints: HashMap::new(),
            message_log: Vec::new(),
        }
    }

    pub fn register(&mut self, req: RegisterEndpointRequest) -> ProtocolEndpoint {
        let ep = ProtocolEndpoint {
            id: Uuid::new_v4(),
            name: req.name,
            protocol: req.protocol,
            listen_path: req.listen_path,
            upstream_address: req.upstream_address,
            config: req.config,
            active: true,
            created_at: chrono::Utc::now(),
        };
        self.endpoints.insert(ep.id, ep.clone());
        ep
    }

    pub fn deregister(&mut self, id: Uuid) -> bool {
        self.endpoints.remove(&id).is_some()
    }

    /// Determine which upstream endpoint should handle an incoming message.
    /// Returns the upstream address or an error describing why routing failed.
    pub fn route_message(&mut self, req: &RouteMessageRequest) -> Result<String, String> {
        let ep = self.endpoints.values()
            .find(|e| e.active && e.protocol == req.protocol && {
                req.topic_or_path.starts_with(&e.listen_path)
                    || e.listen_path == req.topic_or_path
            })
            .ok_or_else(|| format!(
                "no active {:?} endpoint matching '{}'",
                req.protocol, req.topic_or_path
            ))?;

        // Protocol-specific validation.
        match (&req.protocol, &ep.config) {
            (ProtocolType::Grpc, ProtocolConfig::Grpc(cfg)) => {
                let method = req.topic_or_path.rsplit('/').next().unwrap_or("");
                if !cfg.methods.is_empty() && !cfg.methods.iter().any(|m| m == method) {
                    return Err(format!("gRPC method '{}' not allowed on this endpoint", method));
                }
            }
            (ProtocolType::GraphQL, ProtocolConfig::GraphQL(cfg)) => {
                if cfg.max_depth > 0 {
                    // Heuristic depth check: count nesting level in payload.
                    let depth = req.payload.as_ref()
                        .and_then(|p| p.as_str())
                        .map(|s| s.chars().filter(|&c| c == '{').count())
                        .unwrap_or(0) as u32;
                    if depth > cfg.max_depth {
                        return Err(format!("GraphQL query depth {} exceeds limit {}", depth, cfg.max_depth));
                    }
                }
            }
            (ProtocolType::Mqtt, ProtocolConfig::Mqtt(cfg)) => {
                if !req.topic_or_path.starts_with(&cfg.topic_prefix) {
                    return Err(format!(
                        "MQTT topic '{}' does not match prefix '{}'",
                        req.topic_or_path, cfg.topic_prefix
                    ));
                }
            }
            _ => {}
        }

        let upstream = ep.upstream_address.clone();
        self.message_log.push(MessageRoutingEvent {
            endpoint_id: ep.id,
            protocol: format!("{:?}", req.protocol),
            topic_or_path: req.topic_or_path.clone(),
            routed_to: upstream.clone(),
            occurred_at: chrono::Utc::now(),
        });
        // Keep log bounded.
        if self.message_log.len() > 10_000 {
            self.message_log.drain(0..5_000);
        }
        Ok(upstream)
    }

    pub fn protocol_summary(&self) -> serde_json::Value {
        let mut by_proto: HashMap<String, usize> = HashMap::new();
        for ep in self.endpoints.values() {
            *by_proto.entry(format!("{:?}", ep.protocol)).or_insert(0) += 1;
        }
        serde_json::json!({
            "total_endpoints": self.endpoints.len(),
            "active_endpoints": self.endpoints.values().filter(|e| e.active).count(),
            "by_protocol": by_proto,
            "total_messages_routed": self.message_log.len(),
        })
    }
}

impl Default for ProtocolStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/v1/gateway/protocols", get(list_endpoints).post(register_endpoint))
        .route("/api/v1/gateway/protocols/summary", get(protocol_summary))
        .route("/api/v1/gateway/protocols/:id", get(get_endpoint).delete(deregister_endpoint))
        .route("/api/v1/gateway/protocols/route", post(route_message))
        .route("/api/v1/gateway/protocols/messages", get(message_log))
}

async fn list_endpoints(State(state): State<Arc<GatewayState>>) -> Json<Vec<ProtocolEndpoint>> {
    let store = state.protocols.lock().unwrap();
    Json(store.endpoints.values().cloned().collect())
}

async fn register_endpoint(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<RegisterEndpointRequest>,
) -> Json<ProtocolEndpoint> {
    let mut store = state.protocols.lock().unwrap();
    Json(store.register(req))
}

async fn get_endpoint(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.protocols.lock().unwrap();
    match store.endpoints.get(&id) {
        Some(ep) => Json(serde_json::to_value(ep).unwrap()),
        None => Json(serde_json::json!({ "error": "endpoint not found" })),
    }
}

async fn deregister_endpoint(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.protocols.lock().unwrap();
    Json(serde_json::json!({ "removed": store.deregister(id) }))
}

async fn route_message(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<RouteMessageRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.protocols.lock().unwrap();
    match store.route_message(&req) {
        Ok(upstream) => Json(serde_json::json!({
            "routed": true,
            "upstream": upstream,
            "protocol": format!("{:?}", req.protocol),
        })),
        Err(e) => Json(serde_json::json!({ "routed": false, "error": e })),
    }
}

async fn protocol_summary(State(state): State<Arc<GatewayState>>) -> Json<serde_json::Value> {
    let store = state.protocols.lock().unwrap();
    Json(store.protocol_summary())
}

async fn message_log(State(state): State<Arc<GatewayState>>) -> Json<Vec<MessageRoutingEvent>> {
    let store = state.protocols.lock().unwrap();
    let recent: Vec<_> = store.message_log.iter().rev().take(100).cloned().collect();
    Json(recent)
}
