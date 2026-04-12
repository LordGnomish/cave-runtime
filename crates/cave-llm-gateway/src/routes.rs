//! HTTP routes — OpenAI-compatible chat/completion/embedding endpoints + management APIs.

use crate::budget;
use crate::cache;
use crate::guardrails::{self, GuardrailResult};
use crate::models::*;
use crate::router;
use crate::GatewayState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

pub fn create_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        // ── OpenAI-compatible inference endpoints ────────────────────────────
        .route("/api/v1/llm/chat/completions", post(chat_completions))
        .route("/api/v1/llm/completions", post(completions))
        .route("/api/v1/llm/embeddings", post(embeddings))
        .route("/api/v1/llm/models", get(list_models))
        // ── Provider management ──────────────────────────────────────────────
        .route(
            "/api/v1/llm/providers",
            get(list_providers).post(create_provider),
        )
        .route(
            "/api/v1/llm/providers/{id}",
            get(get_provider).put(update_provider).delete(delete_provider),
        )
        // ── Routing policies ─────────────────────────────────────────────────
        .route(
            "/api/v1/llm/policies",
            get(list_policies).post(create_policy),
        )
        .route(
            "/api/v1/llm/policies/{id}",
            get(get_policy).put(update_policy).delete(delete_policy),
        )
        // ── Token budgets ────────────────────────────────────────────────────
        .route(
            "/api/v1/llm/budgets",
            get(list_budgets).post(create_budget),
        )
        .route(
            "/api/v1/llm/budgets/{id}",
            get(get_budget).put(update_budget).delete(delete_budget),
        )
        // ── Observability ────────────────────────────────────────────────────
        .route("/api/v1/llm/usage", get(usage_stats))
        .route("/api/v1/llm/cache/stats", get(cache_stats))
        // ── Guardrails ───────────────────────────────────────────────────────
        .route(
            "/api/v1/llm/guardrails",
            get(list_guardrails).post(create_guardrail),
        )
        .route(
            "/api/v1/llm/guardrails/{id}",
            get(get_guardrail).put(update_guardrail).delete(delete_guardrail),
        )
        // ── Health ───────────────────────────────────────────────────────────
        .route("/api/v1/llm/health", get(health))
        .with_state(state)
}

// ─── Chat completions (OpenAI drop-in) ───────────────────────────────────────

async fn chat_completions(
    State(state): State<Arc<GatewayState>>,
    Json(mut request): Json<LlmRequest>,
) -> Result<Json<LlmResponse>, (StatusCode, Json<serde_json::Value>)> {
    info!(model = %request.model, "Chat completion request");

    // 1. Pre-flight guardrails.
    match guardrails::evaluate_guardrails(&state, &request) {
        GuardrailResult::Block(reason) => {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "blocked", "reason": reason})),
            ));
        }
        GuardrailResult::Redact(modified) => {
            request = modified;
        }
        _ => {}
    }

    // 2. Semantic cache check.
    if let Some(cached) = cache::semantic_cache_lookup(&state, &request) {
        info!(model = %request.model, "Returning cached response");
        return Ok(Json(cached));
    }

    // 3. Route to a healthy provider.
    let route = router::route_request(&state, &request).ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no healthy providers available"})),
        )
    })?;

    info!(
        provider = %route.provider.name,
        model = %route.resolved_model,
        "Forwarding to provider"
    );

    // 4. Build provider-specific request body.
    let body =
        router::transform_request(&request, &route.provider.provider_type, &route.resolved_model);

    let start = std::time::Instant::now();

    // 5. Forward to provider (stub — replace with reqwest call to route.provider.endpoint).
    let raw = stub_provider_response(&route.provider);

    let latency_ms = start.elapsed().as_millis() as u64;
    let _ = body; // used in real implementation

    // 6. Normalize response.
    let response = router::transform_response(&raw, &route.provider, latency_ms);

    // 7. Track token usage against budgets.
    if let Some(meta) = &request.metadata {
        if let Some(team) = &meta.team {
            budget::track_usage(&state, BudgetScope::Team, team, response.usage.total_tokens as u64);
        }
        if let Some(project) = &meta.project {
            budget::track_usage(&state, BudgetScope::Project, project, response.usage.total_tokens as u64);
        }
        if let Some(user) = &meta.user_id {
            budget::track_usage(&state, BudgetScope::User, user, response.usage.total_tokens as u64);
        }
    }

    // 8. Cache the response (1 hour TTL by default).
    cache::cache_store(&state, &request, &response, 3_600);

    Ok(Json(response))
}

/// Stub provider call. Replace with:
///   reqwest::Client::new()
///     .post(&provider.endpoint)
///     .bearer_auth(resolved_api_key)
///     .json(&body)
///     .send().await?.json::<serde_json::Value>().await?
fn stub_provider_response(provider: &LlmProvider) -> serde_json::Value {
    let model = provider
        .models_available
        .first()
        .cloned()
        .unwrap_or_else(|| "stub-model".to_string());
    serde_json::json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": format!("[stub: {}]", provider.name)},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
    })
}

// ─── Legacy completions ───────────────────────────────────────────────────────

async fn completions(
    State(_state): State<Arc<GatewayState>>,
    Json(request): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let model = request["model"].as_str().unwrap_or("unknown");
    Json(serde_json::json!({
        "id": format!("cmpl-{}", Uuid::new_v4()),
        "object": "text_completion",
        "model": model,
        "choices": [{"text": "[stub legacy completion]", "index": 0, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 10, "total_tokens": 15}
    }))
}

// ─── Embeddings ───────────────────────────────────────────────────────────────

async fn embeddings(
    State(_state): State<Arc<GatewayState>>,
    Json(request): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let model = request["model"].as_str().unwrap_or("text-embedding-ada-002");
    Json(serde_json::json!({
        "object": "list",
        "model": model,
        "data": [{"object": "embedding", "index": 0, "embedding": vec![0.0f32; 1536]}],
        "usage": {"prompt_tokens": 8, "total_tokens": 8}
    }))
}

// ─── Model list ───────────────────────────────────────────────────────────────

async fn list_models(State(state): State<Arc<GatewayState>>) -> Json<serde_json::Value> {
    let providers = state.providers.lock().unwrap();
    let data: Vec<serde_json::Value> = providers
        .iter()
        .flat_map(|p| {
            p.models_available.iter().map(|m| {
                serde_json::json!({
                    "id": m,
                    "object": "model",
                    "owned_by": p.name,
                    "provider_type": p.provider_type,
                })
            })
        })
        .collect();
    Json(serde_json::json!({"object": "list", "data": data}))
}

// ─── Provider CRUD ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ProviderPayload {
    name: String,
    provider_type: ProviderType,
    endpoint: String,
    api_key_ref: Option<String>,
    models_available: Vec<String>,
    priority: u32,
    weight: f64,
}

async fn list_providers(State(state): State<Arc<GatewayState>>) -> Json<Vec<LlmProvider>> {
    Json(state.providers.lock().unwrap().clone())
}

async fn create_provider(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<ProviderPayload>,
) -> (StatusCode, Json<LlmProvider>) {
    let provider = LlmProvider {
        id: Uuid::new_v4(),
        name: payload.name,
        provider_type: payload.provider_type,
        endpoint: payload.endpoint,
        api_key_ref: payload.api_key_ref,
        models_available: payload.models_available,
        health_status: HealthStatus::Unknown,
        priority: payload.priority,
        weight: payload.weight,
    };
    state.providers.lock().unwrap().push(provider.clone());
    (StatusCode::CREATED, Json(provider))
}

async fn get_provider(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<LlmProvider>, (StatusCode, Json<serde_json::Value>)> {
    state
        .providers
        .lock()
        .unwrap()
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("provider"))
}

async fn update_provider(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<ProviderPayload>,
) -> Result<Json<LlmProvider>, (StatusCode, Json<serde_json::Value>)> {
    let mut providers = state.providers.lock().unwrap();
    providers
        .iter_mut()
        .find(|p| p.id == id)
        .map(|p| {
            p.name = payload.name;
            p.provider_type = payload.provider_type;
            p.endpoint = payload.endpoint;
            p.api_key_ref = payload.api_key_ref;
            p.models_available = payload.models_available;
            p.priority = payload.priority;
            p.weight = payload.weight;
            Json(p.clone())
        })
        .ok_or_else(|| not_found("provider"))
}

async fn delete_provider(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    state.providers.lock().unwrap().retain(|p| p.id != id);
    Json(serde_json::json!({"deleted": id}))
}

// ─── Policy CRUD ──────────────────────────────────────────────────────────────

async fn list_policies(State(state): State<Arc<GatewayState>>) -> Json<Vec<RoutingPolicy>> {
    Json(state.policies.lock().unwrap().clone())
}

async fn create_policy(
    State(state): State<Arc<GatewayState>>,
    Json(mut policy): Json<RoutingPolicy>,
) -> (StatusCode, Json<RoutingPolicy>) {
    policy.id = Uuid::new_v4();
    state.policies.lock().unwrap().push(policy.clone());
    (StatusCode::CREATED, Json(policy))
}

async fn get_policy(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<RoutingPolicy>, (StatusCode, Json<serde_json::Value>)> {
    state
        .policies
        .lock()
        .unwrap()
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("policy"))
}

async fn update_policy(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(updated): Json<RoutingPolicy>,
) -> Result<Json<RoutingPolicy>, (StatusCode, Json<serde_json::Value>)> {
    let mut policies = state.policies.lock().unwrap();
    policies
        .iter_mut()
        .find(|p| p.id == id)
        .map(|p| {
            *p = RoutingPolicy { id, ..updated };
            Json(p.clone())
        })
        .ok_or_else(|| not_found("policy"))
}

async fn delete_policy(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    state.policies.lock().unwrap().retain(|p| p.id != id);
    Json(serde_json::json!({"deleted": id}))
}

// ─── Budget CRUD ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BudgetPayload {
    scope: BudgetScope,
    scope_id: String,
    period: BudgetPeriod,
    limit: u64,
    alert_threshold: f64,
}

async fn list_budgets(State(state): State<Arc<GatewayState>>) -> Json<Vec<TokenBudget>> {
    Json(state.budgets.lock().unwrap().clone())
}

async fn create_budget(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<BudgetPayload>,
) -> (StatusCode, Json<TokenBudget>) {
    let b = TokenBudget {
        id: Uuid::new_v4(),
        scope: payload.scope,
        scope_id: payload.scope_id,
        period: payload.period,
        limit: payload.limit,
        current_usage: 0,
        alert_threshold: payload.alert_threshold,
        period_start: Utc::now(),
    };
    state.budgets.lock().unwrap().push(b.clone());
    (StatusCode::CREATED, Json(b))
}

async fn get_budget(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<TokenBudget>, (StatusCode, Json<serde_json::Value>)> {
    state
        .budgets
        .lock()
        .unwrap()
        .iter()
        .find(|b| b.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("budget"))
}

async fn update_budget(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<BudgetPayload>,
) -> Result<Json<TokenBudget>, (StatusCode, Json<serde_json::Value>)> {
    let mut budgets = state.budgets.lock().unwrap();
    budgets
        .iter_mut()
        .find(|b| b.id == id)
        .map(|b| {
            b.scope = payload.scope;
            b.scope_id = payload.scope_id;
            b.period = payload.period;
            b.limit = payload.limit;
            b.alert_threshold = payload.alert_threshold;
            Json(b.clone())
        })
        .ok_or_else(|| not_found("budget"))
}

async fn delete_budget(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    state.budgets.lock().unwrap().retain(|b| b.id != id);
    Json(serde_json::json!({"deleted": id}))
}

// ─── Usage & cache stats ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct UsageQuery {
    team: Option<String>,
    #[allow(dead_code)]
    period: Option<String>,
}

async fn usage_stats(
    State(state): State<Arc<GatewayState>>,
    Query(q): Query<UsageQuery>,
) -> Json<serde_json::Value> {
    Json(budget::generate_report(
        &state,
        q.team.as_ref().map(|_| &BudgetScope::Team),
        q.team.as_deref(),
    ))
}

async fn cache_stats(State(state): State<Arc<GatewayState>>) -> Json<serde_json::Value> {
    Json(cache::cache_stats(&state))
}

// ─── Guardrail CRUD ───────────────────────────────────────────────────────────

async fn list_guardrails(State(state): State<Arc<GatewayState>>) -> Json<Vec<Guardrail>> {
    Json(state.guardrails.lock().unwrap().clone())
}

async fn create_guardrail(
    State(state): State<Arc<GatewayState>>,
    Json(mut g): Json<Guardrail>,
) -> (StatusCode, Json<Guardrail>) {
    g.id = Uuid::new_v4();
    state.guardrails.lock().unwrap().push(g.clone());
    (StatusCode::CREATED, Json(g))
}

async fn get_guardrail(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Guardrail>, (StatusCode, Json<serde_json::Value>)> {
    state
        .guardrails
        .lock()
        .unwrap()
        .iter()
        .find(|g| g.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("guardrail"))
}

async fn update_guardrail(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(updated): Json<Guardrail>,
) -> Result<Json<Guardrail>, (StatusCode, Json<serde_json::Value>)> {
    let mut guardrails = state.guardrails.lock().unwrap();
    guardrails
        .iter_mut()
        .find(|g| g.id == id)
        .map(|g| {
            *g = Guardrail { id, ..updated };
            Json(g.clone())
        })
        .ok_or_else(|| not_found("guardrail"))
}

async fn delete_guardrail(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    state.guardrails.lock().unwrap().retain(|g| g.id != id);
    Json(serde_json::json!({"deleted": id}))
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<GatewayState>>) -> Json<serde_json::Value> {
    let providers = state.providers.lock().unwrap();
    let total = providers.len();
    let healthy = providers
        .iter()
        .filter(|p| p.health_status == HealthStatus::Healthy)
        .count();
    drop(providers);

    Json(serde_json::json!({
        "module": "cave-llm-gateway",
        "status": "ok",
        "upstream": "LiteLLM / AI Gateway",
        "providers": {"total": total, "healthy": healthy},
        "cache": cache::cache_stats(&state),
    }))
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn not_found(resource: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": format!("{resource} not found")})),
    )
}
