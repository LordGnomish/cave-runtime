//! Prompt caching — exact-match cache keyed on request fingerprint.

use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    /// TTL in seconds; 0 = no expiry
    pub ttl_secs: u64,
    pub max_entries: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { enabled: true, ttl_secs: 3600, max_entries: 10_000 }
    }
}

struct CacheEntry {
    response: ChatCompletionResponse,
    inserted_at: Instant,
    ttl: Duration,
    hits: u64,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        if self.ttl.is_zero() {
            return false;
        }
        self.inserted_at.elapsed() > self.ttl
    }
}

pub struct PromptCache {
    config: CacheConfig,
    store: DashMap<u64, CacheEntry>,
    /// Stats
    hits: std::sync::atomic::AtomicU64,
    misses: std::sync::atomic::AtomicU64,
}

impl PromptCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            store: DashMap::new(),
            hits: std::sync::atomic::AtomicU64::new(0),
            misses: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn fingerprint(req: &ChatCompletionRequest) -> u64 {
        let mut hasher = DefaultHasher::new();
        req.model.hash(&mut hasher);
        for msg in &req.messages {
            format!("{:?}", msg.role).hash(&mut hasher);
            if let Some(text) = msg.content.as_text() {
                text.hash(&mut hasher);
            }
        }
        // Include key sampling params that affect output deterministically
        if let Some(seed) = req.seed {
            seed.hash(&mut hasher);
        }
        // temperature = 0 is deterministic; cache that case
        let temp = req.temperature.unwrap_or(1.0);
        if temp == 0.0 {
            0u32.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn get(&self, req: &ChatCompletionRequest) -> Option<ChatCompletionResponse> {
        if !self.config.enabled {
            return None;
        }

        let key = Self::fingerprint(req);
        if let Some(mut entry) = self.store.get_mut(&key) {
            if entry.is_expired() {
                drop(entry);
                self.store.remove(&key);
                self.misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return None;
            }
            entry.hits += 1;
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Some(entry.response.clone());
        }

        self.misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }

    pub fn insert(&self, req: &ChatCompletionRequest, resp: ChatCompletionResponse) {
        if !self.config.enabled {
            return;
        }

        // Evict if over limit (simple: remove a random entry)
        if self.store.len() >= self.config.max_entries {
            if let Some(old_key) = self.store.iter().next().map(|e| *e.key()) {
                self.store.remove(&old_key);
            }
        }

        let key = Self::fingerprint(req);
        let ttl = if self.config.ttl_secs == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs(self.config.ttl_secs)
        };
        self.store.insert(key, CacheEntry { response: resp, inserted_at: Instant::now(), ttl, hits: 0 });
    }

    pub fn invalidate(&self, req: &ChatCompletionRequest) -> bool {
        let key = Self::fingerprint(req);
        self.store.remove(&key).is_some()
    }

    pub fn clear(&self) {
        self.store.clear();
    }

    pub fn stats(&self) -> CacheStats {
        let hits = self.hits.load(std::sync::atomic::Ordering::Relaxed);
        let misses = self.misses.load(std::sync::atomic::Ordering::Relaxed);
        let total = hits + misses;
        CacheStats {
            entries: self.store.len(),
            hits,
            misses,
            hit_rate: if total == 0 { 0.0 } else { hits as f64 / total as f64 },
        }
    }
}

impl Default for PromptCache {
    fn default() -> Self {
        Self::new(CacheConfig::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub entries: usize,
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ChatMessage, Usage};

    fn make_req(text: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage::user(text)],
            temperature: Some(0.0),
            seed: Some(42),
            top_p: None, max_tokens: None, stream: None, stop: None,
            presence_penalty: None, frequency_penalty: None, n: None,
            user: None, tools: None, tool_choice: None, response_format: None, logprobs: None,
        }
    }

    #[test]
    fn cache_hit() {
        let cache = PromptCache::new(CacheConfig::default());
        let req = make_req("hello");
        let resp = ChatCompletionResponse::simple("gpt-4o", "hi".into(), Usage::new(5, 2));
        cache.insert(&req, resp.clone());
        assert!(cache.get(&req).is_some());
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn cache_miss_different_text() {
        let cache = PromptCache::new(CacheConfig::default());
        let req1 = make_req("hello");
        let req2 = make_req("world");
        let resp = ChatCompletionResponse::simple("gpt-4o", "hi".into(), Usage::new(5, 2));
        cache.insert(&req1, resp);
        assert!(cache.get(&req2).is_none());
    }

    #[test]
    fn disabled_cache_never_hits() {
        let cache = PromptCache::new(CacheConfig { enabled: false, ttl_secs: 3600, max_entries: 1000 });
        let req = make_req("hi");
        let resp = ChatCompletionResponse::simple("gpt-4o", "hello".into(), Usage::new(5, 2));
        cache.insert(&req, resp);
        assert!(cache.get(&req).is_none());
    }
}
