//! Gravitee Flow-based Policy Designer — visual request/response policy chains
//! with pre-route, route, post-route, and error stages plus conditional execution.

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

pub struct FlowStore {
    pub flows: HashMap<Uuid, PolicyFlow>,
}

impl FlowStore {
    pub fn new() -> Self {
        Self { flows: HashMap::new() }
    }

    pub fn create(&mut self, req: CreateFlowRequest) -> PolicyFlow {
        let now = chrono::Utc::now();
        let flow = PolicyFlow {
            id: Uuid::new_v4(),
            name: req.name,
            api_id: req.api_id,
            pre_route: req.pre_route.unwrap_or_default(),
            route: req.route.unwrap_or_default(),
            post_route: req.post_route.unwrap_or_default(),
            error: req.error.unwrap_or_default(),
            created_at: now,
            updated_at: now,
        };
        self.flows.insert(flow.id, flow.clone());
        flow
    }

    pub fn delete_flow(&mut self, id: Uuid) -> bool {
        self.flows.remove(&id).is_some()
    }

    /// Evaluate which policy steps would fire for a given request context.
    /// Condition expressions are simple key=value checks on method/path prefix.
    pub fn evaluate(&self, id: Uuid, req: &EvaluateFlowRequest) -> Option<FlowEvaluation> {
        let flow = self.flows.get(&id)?;
        let mut executed_steps = Vec::new();

        for (stage_name, steps) in [
            ("pre_route", &flow.pre_route),
            ("route", &flow.route),
            ("post_route", &flow.post_route),
        ] {
            for step in steps {
                let (would_execute, reason) = evaluate_step(step, req);
                executed_steps.push(ExecutedStep {
                    stage: stage_name.to_string(),
                    step_id: step.id,
                    policy_type: step.policy_type.clone(),
                    would_execute,
                    reason,
                });
            }
        }

        Some(FlowEvaluation {
            flow_id: id,
            path: req.path.clone(),
            method: req.method.clone(),
            executed_steps,
        })
    }
}

impl Default for FlowStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate whether a policy step's condition is satisfied by the request.
/// Supported condition syntax:
///   - None → always execute
///   - "method=POST" → matches HTTP method
///   - "path=/api/v1" → path starts with prefix
///   - "header=X-Custom:value" → header equals value
fn evaluate_step(step: &PolicyStep, req: &EvaluateFlowRequest) -> (bool, String) {
    if !step.enabled {
        return (false, "step is disabled".into());
    }
    let condition = match &step.condition {
        None => return (true, "no condition — always executes".into()),
        Some(c) => c,
    };

    if let Some(method) = condition.strip_prefix("method=") {
        let matches = req.method.eq_ignore_ascii_case(method);
        return (matches, format!("method condition '{}' — {}", method, if matches { "match" } else { "no match" }));
    }

    if let Some(prefix) = condition.strip_prefix("path=") {
        let matches = req.path.starts_with(prefix);
        return (matches, format!("path prefix '{}' — {}", prefix, if matches { "match" } else { "no match" }));
    }

    if let Some(header_expr) = condition.strip_prefix("header=") {
        let mut parts = header_expr.splitn(2, ':');
        let header_name = parts.next().unwrap_or("").trim();
        let expected_val = parts.next().unwrap_or("").trim();
        let actual = req.headers.as_ref()
            .and_then(|h| h.get(header_name))
            .map(|v| v.as_str())
            .unwrap_or("");
        let matches = actual == expected_val;
        return (matches, format!("header '{}' condition — {}", header_name, if matches { "match" } else { "no match" }));
    }

    // Unknown condition syntax — execute defensively.
    (true, format!("unknown condition syntax '{}' — executing by default", condition))
}

// ── Routes ────────────────────────────────────────────────────────────────────

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/v1/gateway/flows", get(list_flows).post(create_flow))
        .route("/api/v1/gateway/flows/{id}", get(get_flow).delete(delete_flow))
        .route("/api/v1/gateway/flows/{id}/evaluate", post(evaluate_flow))
}

async fn list_flows(State(state): State<Arc<GatewayState>>) -> Json<Vec<PolicyFlow>> {
    let store = state.flows.lock().unwrap();
    Json(store.flows.values().cloned().collect())
}

async fn create_flow(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateFlowRequest>,
) -> Json<PolicyFlow> {
    let mut store = state.flows.lock().unwrap();
    Json(store.create(req))
}

async fn get_flow(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.flows.lock().unwrap();
    match store.flows.get(&id) {
        Some(f) => Json(serde_json::to_value(f).unwrap()),
        None => Json(serde_json::json!({ "error": "flow not found" })),
    }
}

async fn delete_flow(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.flows.lock().unwrap();
    Json(serde_json::json!({ "deleted": store.delete_flow(id) }))
}

async fn evaluate_flow(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<EvaluateFlowRequest>,
) -> Json<serde_json::Value> {
    let store = state.flows.lock().unwrap();
    match store.evaluate(id, &req) {
        Some(eval) => Json(serde_json::to_value(eval).unwrap()),
        None => Json(serde_json::json!({ "error": "flow not found" })),
    }
}
