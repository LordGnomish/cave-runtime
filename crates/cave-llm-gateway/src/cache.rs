//! Semantic response cache — hash prompt → cached LlmResponse.

use crate::models::{LlmRequest, LlmResponse, Message, SemanticCacheEntry};
use crate::GatewayState;
use chrono::Utc;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tracing::{debug, info};

/// Produce a deterministic hex string key for a set of messages.
pub fn prompt_hash(messages: &[Message]) -> String {
    let mut hasher = DefaultHasher::new();
    for msg in messages {
        msg.role.hash(&mut hasher);
        msg.content.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

/// Check the cache for a fresh response matching this request.
pub fn semantic_cache_lookup(state: &GatewayState, request: &LlmRequest) -> Option<LlmResponse> {
    let hash = prompt_hash(&request.messages);
    let mut cache = state.cache.lock().unwrap();

    // Stale entries are evicted on miss.
    if let Some(entry) = cache.get_mut(&hash) {
        if entry.is_fresh() && entry.model == request.model {
            entry.hit_count += 1;
            let mut response = entry.response.clone();
            response.cached = true;
            debug!(hash = %hash, hits = entry.hit_count, "Cache hit");
            return Some(response);
        }
        // Entry exists but is stale — drop it.
        cache.remove(&hash);
    }

    None
}

/// Store a completed response in the cache with the given TTL.
pub fn cache_store(
    state: &GatewayState,
    request: &LlmRequest,
    response: &LlmResponse,
    ttl_seconds: u64,
) {
    let hash = prompt_hash(&request.messages);
    let entry = SemanticCacheEntry {
        prompt_hash: hash.clone(),
        response: response.clone(),
        created_at: Utc::now(),
        ttl_seconds,
        hit_count: 0,
        model: request.model.clone(),
    };
    state.cache.lock().unwrap().insert(hash, entry);
    info!(model = %request.model, ttl = ttl_seconds, "Response cached");
}

/// Evict cache entries matching the given filters.
/// Returns the number of entries removed.
pub fn cache_invalidate(
    state: &GatewayState,
    model_filter: Option<&str>,
    max_age_seconds: Option<u64>,
) -> usize {
    let mut cache = state.cache.lock().unwrap();
    let before = cache.len();

    cache.retain(|_, entry| {
        if model_filter.is_some_and(|m| entry.model == m) {
            return false;
        }
        if let Some(max_age) = max_age_seconds {
            let age = Utc::now()
                .signed_duration_since(entry.created_at)
                .num_seconds();
            if age >= 0 && (age as u64) >= max_age {
                return false;
            }
        }
        true
    });

    before - cache.len()
}

/// Fuzzy prompt match for cache hits on semantically similar (not byte-identical) prompts.
///
/// Current implementation falls back to exact hash matching. A future version can
/// compare prompt embeddings from cave-ai-obs once that module exposes an embedding
/// endpoint.
pub fn similarity_match(
    state: &GatewayState,
    request: &LlmRequest,
    _similarity_threshold: f64,
) -> Option<LlmResponse> {
    semantic_cache_lookup(state, request)
}

/// Return a JSON summary of cache metrics.
pub fn cache_stats(state: &GatewayState) -> serde_json::Value {
    let cache = state.cache.lock().unwrap();
    let total = cache.len();
    let fresh = cache.values().filter(|e| e.is_fresh()).count();
    let total_hits: u64 = cache.values().map(|e| e.hit_count).sum();

    serde_json::json!({
        "total_entries": total,
        "fresh_entries": fresh,
        "stale_entries": total - fresh,
        "total_hits": total_hits,
    })
}
