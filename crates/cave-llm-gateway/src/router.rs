//! Provider selection, fallback, load balancing, and format transformation.

use crate::models::{
    HealthStatus, LlmProvider, LlmRequest, LlmResponse, Message, ModelMapping, ProviderType,
    RoutingStrategy, Usage, Choice,
};
use crate::GatewayState;
use std::collections::HashMap;
use tracing::warn;
use uuid::Uuid;

pub struct RouteResult {
    pub provider: LlmProvider,
    pub resolved_model: String,
}

/// Select the best provider for a request based on the active routing policy.
pub fn route_request(state: &GatewayState, request: &LlmRequest) -> Option<RouteResult> {
    let providers = state.providers.lock().unwrap();
    let policies = state.policies.lock().unwrap();
    let mappings = state.model_mappings.lock().unwrap();

    let healthy: Vec<&LlmProvider> = providers
        .iter()
        .filter(|p| {
            p.health_status == HealthStatus::Healthy
                || p.health_status == HealthStatus::Unknown
        })
        .collect();

    if healthy.is_empty() {
        warn!("No healthy providers available");
        return None;
    }

    let policy = policies.first().cloned().unwrap_or_default();

    let provider = match policy.strategy {
        RoutingStrategy::PriorityFallback => {
            let mut sorted = healthy.clone();
            sorted.sort_by_key(|p| p.priority);
            sorted.into_iter().next()
        }
        RoutingStrategy::RoundRobin => load_balance(&healthy),
        RoutingStrategy::LeastLatency => healthy.iter().min_by_key(|p| p.priority).copied(),
        RoutingStrategy::CostOptimized => cost_optimize(&healthy),
        RoutingStrategy::Custom => {
            let mut sorted = healthy.clone();
            sorted.sort_by_key(|p| p.priority);
            sorted.into_iter().next()
        }
    }?;

    let resolved_model = resolve_model(&request.model, &provider.provider_type, &mappings);
    Some(RouteResult {
        provider: provider.clone(),
        resolved_model,
    })
}

/// Try providers in fallback order when the primary fails.
pub fn fallback_chain(
    state: &GatewayState,
    request: &LlmRequest,
    failed_provider_id: Uuid,
) -> Option<RouteResult> {
    let providers = state.providers.lock().unwrap();
    let policies = state.policies.lock().unwrap();
    let mappings = state.model_mappings.lock().unwrap();

    let policy = policies.first().cloned().unwrap_or_default();

    // Try explicit fallback chain first.
    for provider_id in &policy.fallback_chain {
        if *provider_id == failed_provider_id {
            continue;
        }
        if let Some(provider) = providers.iter().find(|p| p.id == *provider_id) {
            if provider.health_status != HealthStatus::Unhealthy {
                let resolved_model =
                    resolve_model(&request.model, &provider.provider_type, &mappings);
                return Some(RouteResult {
                    provider: provider.clone(),
                    resolved_model,
                });
            }
        }
    }

    // Fall back to any remaining healthy provider.
    providers
        .iter()
        .filter(|p| p.id != failed_provider_id && p.health_status != HealthStatus::Unhealthy)
        .next()
        .map(|provider| {
            let resolved_model =
                resolve_model(&request.model, &provider.provider_type, &mappings);
            RouteResult {
                provider: provider.clone(),
                resolved_model,
            }
        })
}

/// Weighted random selection across healthy providers.
pub fn load_balance<'a>(providers: &[&'a LlmProvider]) -> Option<&'a LlmProvider> {
    if providers.is_empty() {
        return None;
    }
    let total_weight: f64 = providers.iter().map(|p| p.weight).sum();
    if total_weight == 0.0 {
        return providers.first().copied();
    }
    let mut target = pseudo_random_f64() * total_weight;
    for provider in providers {
        target -= provider.weight;
        if target <= 0.0 {
            return Some(provider);
        }
    }
    providers.last().copied()
}

/// Pick the cheapest equivalent provider using a static cost heuristic.
pub fn cost_optimize<'a>(providers: &[&'a LlmProvider]) -> Option<&'a LlmProvider> {
    let cost_rank = |pt: &ProviderType| match pt {
        ProviderType::Local => 0u8,
        ProviderType::Ollama => 1,
        ProviderType::Mistral => 2,
        ProviderType::OpenAI => 3,
        ProviderType::Anthropic => 4,
        ProviderType::Bedrock => 5,
        ProviderType::Google => 6,
        ProviderType::AzureOpenAI => 7,
    };
    providers
        .iter()
        .min_by_key(|p| cost_rank(&p.provider_type))
        .copied()
}

/// Resolve a model alias to the provider-specific model name.
pub fn resolve_model(
    alias: &str,
    provider_type: &ProviderType,
    mappings: &HashMap<String, ModelMapping>,
) -> String {
    if let Some(mapping) = mappings.get(alias) {
        if let Some(model) = mapping.provider_models.get(provider_type) {
            return model.clone();
        }
    }
    alias.to_string()
}

/// Transform a unified LlmRequest into the provider's wire format.
pub fn transform_request(
    request: &LlmRequest,
    provider_type: &ProviderType,
    resolved_model: &str,
) -> serde_json::Value {
    match provider_type {
        ProviderType::Anthropic => to_anthropic(request, resolved_model),
        ProviderType::Bedrock => to_bedrock(request, resolved_model),
        _ => to_openai(request, resolved_model),
    }
}

/// Normalize a provider-specific response back to the unified LlmResponse.
pub fn transform_response(
    raw: &serde_json::Value,
    provider: &LlmProvider,
    latency_ms: u64,
) -> LlmResponse {
    match provider.provider_type {
        ProviderType::Anthropic => from_anthropic(raw, provider, latency_ms),
        _ => from_openai(raw, provider, latency_ms),
    }
}

// ─── Wire format helpers ──────────────────────────────────────────────────────

fn to_openai(request: &LlmRequest, model: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": request.messages,
        "stream": request.stream,
    });
    if let Some(t) = request.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(m) = request.max_tokens {
        body["max_tokens"] = serde_json::json!(m);
    }
    if let Some(tools) = &request.tools {
        body["tools"] = tools.clone();
    }
    body
}

fn to_anthropic(request: &LlmRequest, model: &str) -> serde_json::Value {
    // Anthropic separates the system prompt from the message list.
    let system = request
        .messages
        .iter()
        .find(|m| m.role == "system")
        .map(|m| m.content.clone());

    let messages: Vec<serde_json::Value> = request
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": request.max_tokens.unwrap_or(1024),
    });
    if let Some(sys) = system {
        body["system"] = serde_json::json!(sys);
    }
    if let Some(t) = request.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    body
}

fn to_bedrock(request: &LlmRequest, model: &str) -> serde_json::Value {
    // Bedrock Converse API format.
    let messages: Vec<serde_json::Value> = request
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": [{"text": m.content}],
            })
        })
        .collect();

    serde_json::json!({
        "modelId": model,
        "messages": messages,
        "inferenceConfig": {
            "maxTokens": request.max_tokens.unwrap_or(1024),
            "temperature": request.temperature.unwrap_or(1.0),
        },
    })
}

fn from_openai(raw: &serde_json::Value, provider: &LlmProvider, latency_ms: u64) -> LlmResponse {
    let choices: Vec<Choice> = raw["choices"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .enumerate()
                .map(|(i, c)| Choice {
                    index: i as u32,
                    message: Message {
                        role: c["message"]["role"]
                            .as_str()
                            .unwrap_or("assistant")
                            .to_string(),
                        content: c["message"]["content"].as_str().unwrap_or("").to_string(),
                    },
                    finish_reason: c["finish_reason"].as_str().map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();

    let prompt_tokens = raw["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
    let completion_tokens = raw["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;

    LlmResponse {
        id: Uuid::new_v4(),
        model: raw["model"].as_str().unwrap_or("unknown").to_string(),
        provider_used: provider.name.clone(),
        choices,
        usage: Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: raw["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32,
        },
        latency_ms,
        cost: None,
        cached: false,
    }
}

fn from_anthropic(
    raw: &serde_json::Value,
    provider: &LlmProvider,
    latency_ms: u64,
) -> LlmResponse {
    let content = raw["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["text"].as_str())
        .unwrap_or("")
        .to_string();

    let prompt_tokens = raw["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
    let completion_tokens = raw["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

    LlmResponse {
        id: Uuid::new_v4(),
        model: raw["model"].as_str().unwrap_or("unknown").to_string(),
        provider_used: provider.name.clone(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content,
            },
            finish_reason: raw["stop_reason"].as_str().map(str::to_string),
        }],
        usage: Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        },
        latency_ms,
        cost: None,
        cached: false,
    }
}

/// Cheap pseudo-random float in [0, 1) derived from sub-microsecond system time.
/// Good enough for weighted load balancing without pulling in a rand crate.
fn pseudo_random_f64() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 1_000_000) as f64 / 1_000_000.0
}
