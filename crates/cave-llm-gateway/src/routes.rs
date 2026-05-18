// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP routes for cave-llm-gateway — OpenAI-compatible API + admin endpoints.

use crate::api_keys::{ApiKeyStore, Scope};
use crate::alias::{AliasRegistry, ModelAlias};
use crate::error::GatewayError;
use crate::openai::{ChatCompletionRequest, ModelList, ModelObject, OpenAIError};
use crate::rate_limit::RateLimit;
use crate::router::GatewayRouter;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

type AppState = Arc<InnerState>;

struct InnerState {
    router: Arc<GatewayRouter>,
    api_keys: Arc<ApiKeyStore>,
}

pub fn create_router(state: Arc<GatewayState>) -> Router {
    let api_keys = Arc::new(ApiKeyStore::new());
    let inner = Arc::new(InnerState {
        router: Arc::clone(&state.router),
        api_keys,
    });

    Router::new()
        // OpenAI-compatible endpoints
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        .route("/v1/models/{model}", get(get_model))
        // Admin — providers
        .route("/api/gateway/health", get(health))
        .route("/api/gateway/providers", get(list_providers))
        // Admin — aliases
        .route("/api/gateway/aliases", get(list_aliases).post(create_alias))
        .route("/api/gateway/aliases/{alias}", delete(delete_alias))
        // Admin — rate limits
        .route("/api/gateway/rate-limits/{consumer}", get(get_rate_limit).put(set_rate_limit))
        // Admin — API keys
        .route("/api/gateway/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api/gateway/api-keys/{id}", delete(revoke_api_key))
        // Admin — usage / cost
        .route("/api/gateway/usage", get(global_usage))
        .route("/api/gateway/usage/{consumer}", get(consumer_usage))
        // Admin — cache
        .route("/api/gateway/cache/stats", get(cache_stats))
        .route("/api/gateway/cache/clear", post(clear_cache))
        // Admin — logs
        .route("/api/gateway/logs", get(recent_logs))
        .route("/api/gateway/logs/{consumer}", get(consumer_logs))
        // Admin — guardrails
        .route("/api/gateway/guardrails", get(list_guardrails))
        .with_state(inner)
}

fn extract_consumer(headers: &HeaderMap) -> String {
    headers
        .get("x-consumer-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string()
}

async fn health(State(s): State<AppState>) -> Json<serde_json::Value> {
    let providers = s.router.provider_names();
    Json(json!({
        "module": "cave-llm-gateway",
        "status": "ok",
        "providers": providers,
    }))
}

async fn chat_completions(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    let consumer = extract_consumer(&headers);

    match s.router.complete(&consumer, req).await {
        Ok(resp) => Json(serde_json::to_value(resp).unwrap()).into_response(),
        Err(GatewayError::RateLimitExceeded { retry_after_ms, .. }) => {
            let err = OpenAIError::rate_limit("Rate limit exceeded");
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::to_value(err).unwrap())).into_response();
            resp.headers_mut().insert(
                "Retry-After",
                format!("{}", retry_after_ms / 1000 + 1).parse().unwrap(),
            );
            resp
        }
        Err(GatewayError::GuardrailBlocked { rule, .. }) => {
            let err = OpenAIError::invalid_request(&format!("Request blocked by guardrail: {rule}"));
            (StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response()
        }
        Err(GatewayError::Unauthorized(msg)) => {
            let err = OpenAIError::invalid_request(&msg);
            (StatusCode::UNAUTHORIZED, Json(serde_json::to_value(err).unwrap())).into_response()
        }
        Err(e) => {
            let err = OpenAIError::server_error(&e.to_string());
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::to_value(err).unwrap())).into_response()
        }
    }
}

async fn list_models(State(s): State<AppState>) -> Json<serde_json::Value> {
    let now = chrono::Utc::now().timestamp();
    let mut models: Vec<ModelObject> = Vec::new();

    for provider_name in s.router.provider_names() {
        if let Some(provider) = s.router.providers.get(&provider_name) {
            for model in provider.supported_models() {
                models.push(ModelObject {
                    id: model,
                    object: "model".into(),
                    created: now,
                    owned_by: provider_name.clone(),
                });
            }
        }
    }

    // Also expose aliases as model IDs
    for alias in s.router.aliases.list() {
        models.push(ModelObject {
            id: alias.alias,
            object: "model".into(),
            created: now,
            owned_by: "cave-llm-gateway".into(),
        });
    }

    let list = ModelList { object: "list".into(), data: models };
    Json(serde_json::to_value(list).unwrap())
}

async fn get_model(
    State(s): State<AppState>,
    Path(model): Path<String>,
) -> impl IntoResponse {
    let now = chrono::Utc::now().timestamp();

    // Check aliases first
    if let Some(alias) = s.router.aliases.resolve(&model) {
        return Json(json!({
            "id": alias.alias,
            "object": "model",
            "created": now,
            "owned_by": alias.provider,
        })).into_response();
    }

    // Check provider models
    for provider_name in s.router.provider_names() {
        if let Some(provider) = s.router.providers.get(&provider_name) {
            if provider.supported_models().contains(&model) {
                return Json(json!({
                    "id": model,
                    "object": "model",
                    "created": now,
                    "owned_by": provider_name,
                })).into_response();
            }
        }
    }

    StatusCode::NOT_FOUND.into_response()
}

async fn list_providers(State(s): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let providers: Vec<serde_json::Value> = s.router.provider_names().into_iter().map(|name| {
        let models = s.router.providers.get(&name)
            .map(|p| p.supported_models())
            .unwrap_or_default();
        json!({ "name": name, "models": models })
    }).collect();
    Json(providers)
}

async fn list_aliases(State(s): State<AppState>) -> Json<Vec<ModelAlias>> {
    Json(s.router.aliases.list())
}

#[derive(Deserialize)]
struct CreateAliasRequest {
    alias: String,
    provider: String,
    model: String,
    description: Option<String>,
}

async fn create_alias(
    State(s): State<AppState>,
    Json(req): Json<CreateAliasRequest>,
) -> impl IntoResponse {
    s.router.aliases.register(ModelAlias {
        alias: req.alias.clone(),
        provider: req.provider,
        model: req.model,
        description: req.description,
    });
    (StatusCode::CREATED, Json(json!({ "alias": req.alias }))).into_response()
}

async fn delete_alias(
    State(s): State<AppState>,
    Path(alias): Path<String>,
) -> impl IntoResponse {
    if s.router.aliases.delete(&alias) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn get_rate_limit(
    State(s): State<AppState>,
    Path(consumer): Path<String>,
) -> Json<RateLimit> {
    Json(s.router.rate_limiter.get_limit(&consumer))
}

async fn set_rate_limit(
    State(s): State<AppState>,
    Path(consumer): Path<String>,
    Json(limit): Json<RateLimit>,
) -> StatusCode {
    s.router.rate_limiter.set_limit(&consumer, limit);
    StatusCode::OK
}

#[derive(Deserialize)]
struct CreateApiKeyRequest {
    name: String,
    consumer: String,
    scopes: Vec<String>,
    ttl_days: Option<u32>,
}

async fn create_api_key(
    State(s): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    let scopes: Vec<Scope> = req.scopes.iter().map(|s| match s.as_str() {
        "chat_completions" => Scope::ChatCompletions,
        "models_list" => Scope::ModelsList,
        "admin" => Scope::Admin,
        _ => Scope::All,
    }).collect();

    let key = s.api_keys.create(&req.name, &req.consumer, scopes, req.ttl_days);
    (StatusCode::CREATED, Json(serde_json::to_value(key).unwrap())).into_response()
}

async fn list_api_keys(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "keys": s.api_keys.list() }))
}

async fn revoke_api_key(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match s.api_keys.revoke(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn global_usage(State(s): State<AppState>) -> Json<serde_json::Value> {
    let stats = s.router.cost_tracker.global_stats();
    Json(serde_json::to_value(stats).unwrap())
}

async fn consumer_usage(
    State(s): State<AppState>,
    Path(consumer): Path<String>,
) -> Json<serde_json::Value> {
    let stats = s.router.cost_tracker.consumer_stats(&consumer);
    Json(serde_json::to_value(stats).unwrap())
}

async fn cache_stats(State(s): State<AppState>) -> Json<serde_json::Value> {
    let stats = s.router.cache.stats();
    Json(serde_json::to_value(stats).unwrap())
}

async fn clear_cache(State(s): State<AppState>) -> StatusCode {
    s.router.cache.clear();
    StatusCode::OK
}

async fn recent_logs(State(s): State<AppState>) -> Json<serde_json::Value> {
    let logs = s.router.logger.list(100);
    Json(json!({ "logs": logs }))
}

async fn consumer_logs(
    State(s): State<AppState>,
    Path(consumer): Path<String>,
) -> Json<serde_json::Value> {
    let logs = s.router.logger.list_for_consumer(&consumer, 100);
    Json(json!({ "consumer": consumer, "logs": logs }))
}

async fn list_guardrails(State(s): State<AppState>) -> Json<serde_json::Value> {
    let rules = s.router.guardrails.list_rules();
    Json(json!({ "rules": rules }))
}
