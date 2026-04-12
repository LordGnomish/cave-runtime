//! HTTP routes for cave-infra.

use crate::mcp::{JsonRpcRequest, McpServer};
use crate::nlp::parse_intent;
use crate::plan::generate_plan;
use crate::provider::ProviderRegistry;
use crate::resource::{ResourceKind, ResourceSpec, ResourceState};
use crate::templates::TemplateRegistry;
use crate::InfraState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

type AppState = Arc<InnerState>;

struct InnerState {
    infra: Arc<crate::InfraState>,
    registry: Arc<ProviderRegistry>,
    templates: Arc<TemplateRegistry>,
    mcp: Arc<McpServer>,
}

pub fn create_router(state: Arc<InfraState>) -> Router {
    let registry = Arc::new(ProviderRegistry::new());
    let templates = Arc::new(TemplateRegistry::new());
    let mcp = Arc::new(McpServer::new(
        Arc::clone(&state.store),
        Arc::clone(&registry),
    ));
    let inner = Arc::new(InnerState {
        infra: state,
        registry,
        templates,
        mcp,
    });

    Router::new()
        .route("/api/infra/health", get(health))
        // Resources
        .route("/api/infra/resources", get(list_resources))
        .route("/api/infra/resources/{kind}/{name}", get(get_resource).delete(delete_resource))
        // Plan + Apply
        .route("/api/infra/plan", post(plan_resources))
        .route("/api/infra/apply", post(apply_resources))
        // Drift
        .route("/api/infra/drift", get(drift_report))
        .route("/api/infra/resources/{kind}/{name}/reconcile", post(reconcile_resource))
        // Rollback
        .route("/api/infra/resources/{kind}/{name}/rollback", post(rollback_resource))
        .route("/api/infra/resources/{kind}/{name}/history", get(resource_history))
        // Templates
        .route("/api/infra/templates", get(list_templates))
        .route("/api/infra/templates/{name}", get(get_template))
        .route("/api/infra/templates/{name}/render", post(render_template))
        // Providers
        .route("/api/infra/providers", get(list_providers))
        // NLP
        .route("/api/infra/natural", post(natural_language))
        // MCP protocol endpoint
        .route("/mcp", post(mcp_endpoint))
        .with_state(inner)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"module": "cave-infra", "status": "ok", "upstream": "terraform"}))
}

async fn list_resources(State(s): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(s.infra.store.list().into_iter().map(|r| json!({
        "key": r.key(),
        "kind": r.spec.kind.as_str(),
        "name": r.spec.name,
        "provider": r.spec.provider,
        "status": format!("{:?}", r.status),
        "provider_id": r.provider_id,
    })).collect())
}

async fn get_resource(
    State(s): State<AppState>,
    Path((kind, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{kind}/{name}");
    match s.infra.store.get(&key) {
        Ok(r) => Json(json!({
            "key": r.key(),
            "spec": r.spec.properties,
            "status": format!("{:?}", r.status),
            "outputs": r.outputs,
            "provider_id": r.provider_id,
            "version": r.version,
        })).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_resource(
    State(s): State<AppState>,
    Path((kind, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{kind}/{name}");
    match s.infra.store.delete(&key) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct PlanRequest {
    resources: Vec<ResourceSpecDto>,
}

#[derive(Deserialize)]
struct ResourceSpecDto {
    kind: String,
    name: String,
    provider: String,
    #[serde(default)]
    properties: HashMap<String, serde_json::Value>,
    #[serde(default)]
    depends_on: Vec<String>,
}

impl From<ResourceSpecDto> for ResourceSpec {
    fn from(d: ResourceSpecDto) -> Self {
        ResourceSpec {
            kind: ResourceKind::from_str(&d.kind),
            name: d.name,
            provider: d.provider,
            properties: d.properties,
            depends_on: d.depends_on,
            tags: HashMap::new(),
        }
    }
}

async fn plan_resources(
    State(s): State<AppState>,
    Json(req): Json<PlanRequest>,
) -> impl IntoResponse {
    let specs: Vec<ResourceSpec> = req.resources.into_iter().map(Into::into).collect();
    match generate_plan(&specs, &s.infra.store) {
        Ok(plan) => Json(json!({
            "plan_id": plan.id,
            "has_changes": plan.has_changes(),
            "summary": plan.summary,
            "changes": plan.changes,
        })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn apply_resources(
    State(s): State<AppState>,
    Json(req): Json<PlanRequest>,
) -> impl IntoResponse {
    let specs: Vec<ResourceSpec> = req.resources.into_iter().map(Into::into).collect();

    // Validate and order
    let ordered = match crate::graph::apply_order(&specs) {
        Ok(o) => o,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    };

    let mut applied = Vec::new();
    for spec in ordered {
        let result = s.registry.create(&spec.provider, spec).await;
        match result {
            Ok(prov_result) => {
                let mut state = ResourceState::new(spec.clone());
                state.apply_actual(prov_result.actual, Some(prov_result.provider_id));
                let key = s.infra.store.upsert(state);
                applied.push(key);
            }
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                    "error": e.to_string(),
                    "applied_so_far": applied,
                }))).into_response();
            }
        }
    }

    Json(json!({"applied": applied})).into_response()
}

async fn drift_report(State(s): State<AppState>) -> Json<serde_json::Value> {
    let report = crate::drift::detect_drift(&s.infra.store, &s.registry).await;
    Json(json!({
        "checked_at": report.checked_at,
        "total": report.total_resources,
        "drifted": report.drifted.len(),
        "healthy": report.healthy,
        "resources": report.drifted,
    }))
}

async fn reconcile_resource(
    State(s): State<AppState>,
    Path((kind, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{kind}/{name}");
    match crate::drift::reconcile(&s.infra.store, &s.registry, &key).await {
        Ok(r) => Json(json!({"key": r.key(), "status": format!("{:?}", r.status)})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn rollback_resource(
    State(s): State<AppState>,
    Path((kind, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{kind}/{name}");
    match crate::rollback::rollback_resource(&s.infra.store, &s.registry, &key).await {
        Ok(record) => Json(json!({
            "success": record.success,
            "from_version": record.from_version,
            "to_version": record.to_version,
        })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn resource_history(
    State(s): State<AppState>,
    Path((kind, name)): Path<(String, String)>,
) -> Json<Vec<serde_json::Value>> {
    let key = format!("{kind}/{name}");
    Json(s.infra.store.history(&key).into_iter().map(|r| json!({
        "version": r.version,
        "status": format!("{:?}", r.status),
        "updated_at": r.updated_at,
    })).collect())
}

async fn list_templates(State(s): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(s.templates.list().into_iter().map(|t| json!({
        "name": t.name,
        "description": t.description,
        "version": t.version,
        "params": t.params.iter().map(|p| json!({"name": p.name, "required": p.required})).collect::<Vec<_>>(),
    })).collect())
}

async fn get_template(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.templates.get(&name) {
        Ok(t) => Json(json!({"name": t.name, "description": t.description, "params": t.params})).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn render_template(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Json(params): Json<HashMap<String, serde_json::Value>>,
) -> impl IntoResponse {
    match s.templates.render(&name, &params) {
        Ok(specs) => Json(json!({
            "template": name,
            "resources": specs.iter().map(|s| json!({
                "kind": s.kind.as_str(),
                "name": s.name,
                "provider": s.provider,
                "depends_on": s.depends_on,
            })).collect::<Vec<_>>(),
        })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn list_providers(State(s): State<AppState>) -> Json<Vec<String>> {
    Json(s.registry.list_names())
}

#[derive(Deserialize)]
struct NaturalRequest {
    text: String,
}

async fn natural_language(
    State(_s): State<AppState>,
    Json(req): Json<NaturalRequest>,
) -> Json<serde_json::Value> {
    let intent = parse_intent(&req.text);
    Json(json!({
        "action": format!("{:?}", intent.action),
        "confidence": intent.confidence,
        "resources": intent.resource_specs.iter().map(|s| json!({
            "kind": s.kind.as_str(),
            "name": s.name,
            "provider": s.provider,
            "properties": s.properties,
        })).collect::<Vec<_>>(),
    }))
}

async fn mcp_endpoint(
    State(s): State<AppState>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<serde_json::Value> {
    let resp = s.mcp.handle(req).await;
    Json(serde_json::to_value(resp).unwrap_or(json!({"error": "serialization failed"})))
}
