//! HTTP routes for CAVE LLM Gateway.
//!
//! Two sets of routes are exposed:
//! - OpenAI-compatible endpoints under `/v1/...` for drop-in replacement.
//! - Gateway management endpoints under `/api/llm/...`.

use crate::{
    models::{
        ChatCompletionChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage,
        CompletionChoice, CompletionRequest, CompletionResponse, EmbeddingObject, EmbeddingRequest,
        EmbeddingResponse, Provider, RequestLog, SpendRecord, StringOrArray, Usage,
    },
    router::ProviderRouter,
    store::LlmGatewayStore,
    LlmGatewayState,
};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

/// Build the full router with both OpenAI-compatible and management routes.
pub fn create_router(state: Arc<LlmGatewayState>) -> Router {
    Router::new()
        // ── OpenAI-compatible endpoints ──────────────────────────────────────
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/completions", post(completions))
        .route("/v1/embeddings", post(embeddings))
        .route("/v1/models", get(list_models_openai))
        // ── Gateway management endpoints ─────────────────────────────────────
        .route("/api/llm/health", get(health))
        .route("/api/llm/providers", get(list_providers))
        .route("/api/llm/models", get(list_models))
        .route("/api/llm/models/aliases", post(add_model_alias))
        .route("/api/llm/spend", get(get_spend))
        .route("/api/llm/logs", get(get_logs))
        .route("/api/llm/analytics", get(get_analytics))
        .route("/api/llm/budgets", post(create_budget))
        .route("/api/llm/budgets", get(list_budgets))
        .with_state(state)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Produce a deterministic mock chat completion response for testing / stub mode.
fn mock_chat_response(req: &ChatCompletionRequest) -> ChatCompletionResponse {
    ChatCompletionResponse {
        id: format!("chatcmpl-{}", Uuid::new_v4()),
        object: "chat.completion".to_string(),
        created: Utc::now().timestamp(),
        model: req.model.clone(),
        choices: vec![ChatCompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: "Mock response from CAVE LLM Gateway.".to_string(),
            },
            finish_reason: "stop".to_string(),
        }],
        usage: Usage {
            prompt_tokens: 10,
            completion_tokens: 8,
            total_tokens: 18,
        },
    }
}

/// Produce a deterministic mock embedding vector.
fn mock_embedding(index: u32) -> Vec<f32> {
    // 8-dimensional unit-ish vector for testing.
    (0..8)
        .map(|i| ((index as f32 + i as f32) * 0.1).sin())
        .collect()
}

// ── OpenAI-compatible handlers ────────────────────────────────────────────────

async fn chat_completions(
    State(state): State<Arc<LlmGatewayState>>,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionResponse>, (StatusCode, Json<serde_json::Value>)> {
    // 1. Run guardrails.
    let guardrail_result = state.guardrails.run_all(&req.messages);
    if !guardrail_result.passed {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": "Content policy violation",
                "violations": guardrail_result.violations,
            })),
        ));
    }

    // 2. Check cache.
    let cache_key = LlmGatewayStore::cache_key(&req.model, &req.messages);
    if let Some(cached) = state.store.get_cached(&cache_key) {
        let response: ChatCompletionResponse = serde_json::from_value(cached)
            .unwrap_or_else(|_| mock_chat_response(&req));
        state.store.log_request(RequestLog {
            id: Uuid::new_v4(),
            request_id: response.id.clone(),
            user_id: req.user.clone(),
            provider: Provider::OpenAi,
            model: req.model.clone(),
            request_type: "chat".to_string(),
            prompt_tokens: Some(response.usage.prompt_tokens),
            completion_tokens: Some(response.usage.completion_tokens),
            latency_ms: 0,
            status_code: 200,
            error: None,
            was_cached: true,
            created_at: Utc::now(),
        });
        return Ok(Json(response));
    }

    // 3. Resolve model and provider.
    let (provider, model_id) = state
        .router
        .resolve_model(&req.model)
        .unwrap_or((Provider::OpenAi, req.model.clone()));

    // 4. Generate stub response (real impl would call provider API).
    let response = mock_chat_response(&req);

    // 5. Record spend.
    let cost = ProviderRouter::estimate_cost(
        &provider,
        &model_id,
        response.usage.prompt_tokens,
        response.usage.completion_tokens,
    );
    state.store.record_spend(SpendRecord {
        id: Uuid::new_v4(),
        user_id: req.user.clone(),
        team_id: None,
        project_id: None,
        provider: provider.clone(),
        model: model_id.clone(),
        prompt_tokens: response.usage.prompt_tokens,
        completion_tokens: response.usage.completion_tokens,
        total_tokens: response.usage.total_tokens,
        cost_usd: cost,
        request_id: response.id.clone(),
        created_at: Utc::now(),
    });

    // 6. Log the request.
    state.store.log_request(RequestLog {
        id: Uuid::new_v4(),
        request_id: response.id.clone(),
        user_id: req.user.clone(),
        provider,
        model: model_id,
        request_type: "chat".to_string(),
        prompt_tokens: Some(response.usage.prompt_tokens),
        completion_tokens: Some(response.usage.completion_tokens),
        latency_ms: 5,
        status_code: 200,
        error: None,
        was_cached: false,
        created_at: Utc::now(),
    });

    // 7. Cache the response.
    state.store.set_cache(
        cache_key,
        serde_json::to_value(&response).unwrap(),
        req.model.clone(),
    );

    Ok(Json(response))
}

async fn completions(
    State(state): State<Arc<LlmGatewayState>>,
    Json(req): Json<CompletionRequest>,
) -> Json<CompletionResponse> {
    let (provider, model_id) = state
        .router
        .resolve_model(&req.model)
        .unwrap_or((Provider::OpenAi, req.model.clone()));

    let response = CompletionResponse {
        id: format!("cmpl-{}", Uuid::new_v4()),
        object: "text_completion".to_string(),
        created: Utc::now().timestamp(),
        model: req.model.clone(),
        choices: vec![CompletionChoice {
            text: "Mock completion from CAVE LLM Gateway.".to_string(),
            index: 0,
            finish_reason: "stop".to_string(),
        }],
    };

    state.store.record_spend(SpendRecord {
        id: Uuid::new_v4(),
        user_id: None,
        team_id: None,
        project_id: None,
        provider,
        model: model_id,
        prompt_tokens: 5,
        completion_tokens: 7,
        total_tokens: 12,
        cost_usd: 0.0,
        request_id: response.id.clone(),
        created_at: Utc::now(),
    });

    Json(response)
}

async fn embeddings(
    State(state): State<Arc<LlmGatewayState>>,
    Json(req): Json<EmbeddingRequest>,
) -> Json<EmbeddingResponse> {
    let inputs: Vec<String> = match &req.input {
        StringOrArray::Single(s) => vec![s.clone()],
        StringOrArray::Multiple(v) => v.clone(),
    };

    let data: Vec<EmbeddingObject> = inputs
        .iter()
        .enumerate()
        .map(|(i, _)| EmbeddingObject {
            index: i as u32,
            embedding: mock_embedding(i as u32),
            object: "embedding".to_string(),
        })
        .collect();

    let input_tokens = inputs.iter().map(|s| s.split_whitespace().count() as u32).sum();

    state.store.record_spend(SpendRecord {
        id: Uuid::new_v4(),
        user_id: None,
        team_id: None,
        project_id: None,
        provider: Provider::OpenAi,
        model: req.model.clone(),
        prompt_tokens: input_tokens,
        completion_tokens: 0,
        total_tokens: input_tokens,
        cost_usd: 0.0,
        request_id: Uuid::new_v4().to_string(),
        created_at: Utc::now(),
    });

    Json(EmbeddingResponse {
        object: "list".to_string(),
        data,
        model: req.model,
        usage: Usage {
            prompt_tokens: input_tokens,
            completion_tokens: 0,
            total_tokens: input_tokens,
        },
    })
}

async fn list_models_openai(
    State(state): State<Arc<LlmGatewayState>>,
) -> Json<serde_json::Value> {
    let models: Vec<serde_json::Value> = state
        .router
        .list_models()
        .into_iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "object": "model",
                "owned_by": "cave-llm-gateway",
            })
        })
        .collect();

    Json(serde_json::json!({
        "object": "list",
        "data": models,
    }))
}

// ── Gateway management handlers ───────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-llm-gateway",
        "status": "ok",
        "upstream": "LiteLLM, OpenRouter",
    }))
}

async fn list_providers(
    State(state): State<Arc<LlmGatewayState>>,
) -> Json<serde_json::Value> {
    let providers: Vec<serde_json::Value> = state
        .router
        .list_providers()
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "provider": p.provider,
                "api_base_url": p.api_base_url,
                "enabled": p.enabled,
                "weight": p.weight,
                "models": p.models,
            })
        })
        .collect();

    Json(serde_json::json!({ "providers": providers }))
}

async fn list_models(
    State(state): State<Arc<LlmGatewayState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "models": state.router.list_models() }))
}

#[derive(Deserialize)]
struct AddAliasRequest {
    alias: String,
    provider: String,
    model_id: String,
    max_tokens: Option<u32>,
    pinned_version: Option<String>,
}

async fn add_model_alias(
    State(_state): State<Arc<LlmGatewayState>>,
    Json(req): Json<AddAliasRequest>,
) -> Json<serde_json::Value> {
    // In a real implementation the router would be mutable or wrapped in a RwLock.
    // For this stub we acknowledge the request.
    Json(serde_json::json!({
        "status": "ok",
        "alias": req.alias,
        "provider": req.provider,
        "model_id": req.model_id,
        "max_tokens": req.max_tokens,
        "pinned_version": req.pinned_version,
    }))
}

#[derive(Deserialize)]
struct SpendQuery {
    user_id: Option<String>,
    team_id: Option<String>,
    #[allow(dead_code)]
    project_id: Option<String>,
}

async fn get_spend(
    State(state): State<Arc<LlmGatewayState>>,
    Query(query): Query<SpendQuery>,
) -> Json<serde_json::Value> {
    use chrono::Duration;
    let period_start = Utc::now() - Duration::days(30);
    let spend = if let Some(user_id) = &query.user_id {
        state.store.get_spend_by_user(user_id, period_start)
    } else if let Some(team_id) = &query.team_id {
        state.store.get_spend_by_team(team_id, period_start)
    } else {
        // Return total spend.
        state.store.list_spend_records(10_000).iter().map(|r| r.cost_usd).sum()
    };

    Json(serde_json::json!({
        "spend_usd": spend,
        "period_start": period_start,
    }))
}

#[derive(Deserialize)]
struct LogsQuery {
    limit: Option<usize>,
}

async fn get_logs(
    State(state): State<Arc<LlmGatewayState>>,
    Query(query): Query<LogsQuery>,
) -> Json<serde_json::Value> {
    let limit = query.limit.unwrap_or(100);
    let logs = state.store.list_logs(limit);
    Json(serde_json::json!({ "logs": logs, "count": logs.len() }))
}

async fn get_analytics(
    State(state): State<Arc<LlmGatewayState>>,
) -> Json<serde_json::Value> {
    Json(state.store.analytics_summary())
}

#[derive(Deserialize)]
struct CreateBudgetRequest {
    entity_type: crate::models::BudgetEntityType,
    entity_id: String,
    limit_usd: f64,
    period: crate::models::BudgetPeriod,
}

async fn create_budget(
    State(state): State<Arc<LlmGatewayState>>,
    Json(req): Json<CreateBudgetRequest>,
) -> Json<serde_json::Value> {
    use crate::models::BudgetLimit;
    use chrono::Duration;

    let reset_at = match req.period {
        crate::models::BudgetPeriod::Daily => Utc::now() + Duration::days(1),
        crate::models::BudgetPeriod::Weekly => Utc::now() + Duration::weeks(1),
        crate::models::BudgetPeriod::Monthly => Utc::now() + Duration::days(30),
    };

    let budget = BudgetLimit {
        id: Uuid::new_v4(),
        entity_type: req.entity_type,
        entity_id: req.entity_id,
        limit_usd: req.limit_usd,
        period: req.period,
        current_spend: 0.0,
        reset_at,
    };
    let id = budget.id;
    state.store.add_budget_limit(budget);

    Json(serde_json::json!({ "status": "created", "id": id }))
}

async fn list_budgets(
    State(state): State<Arc<LlmGatewayState>>,
) -> Json<serde_json::Value> {
    let budgets = state.store.list_budget_limits();
    Json(serde_json::json!({ "budgets": budgets }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Method, Request},
    };
    use tower::util::ServiceExt;

    fn test_state() -> Arc<LlmGatewayState> {
        Arc::new(LlmGatewayState::default())
    }

    async fn post_json(app: Router<()>, path: &str, body: serde_json::Value) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn get_path(app: Router<()>, path: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_chat_completions_returns_valid_response() {
        let state = test_state();
        let app = create_router(state);
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{ "role": "user", "content": "Hello!" }]
        });
        let resp = post_json(app, "/v1/chat/completions", body).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert!(json["choices"].as_array().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn test_chat_completions_guardrail_blocks_ssn() {
        let state = test_state();
        let app = create_router(state);
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{ "role": "user", "content": "My SSN is 123-45-6789" }]
        });
        let resp = post_json(app, "/v1/chat/completions", body).await;
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_embeddings_returns_vectors() {
        let state = test_state();
        let app = create_router(state);
        let body = serde_json::json!({
            "model": "text-embedding-3-small",
            "input": "Hello world"
        });
        let resp = post_json(app, "/v1/embeddings", body).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["object"], "list");
        let data = json["data"].as_array().unwrap();
        assert!(!data.is_empty());
        assert!(data[0]["embedding"].as_array().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = test_state();
        let app = create_router(state);
        let resp = get_path(app, "/api/llm/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_models_endpoint() {
        let state = test_state();
        let app = create_router(state);
        let resp = get_path(app, "/api/llm/models").await;
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json["models"].as_array().unwrap().len() > 0);
    }
}
