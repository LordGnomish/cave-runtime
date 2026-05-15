// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Request/response logging for the LLM gateway.

use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLog {
    pub id: String,
    pub consumer: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u64,
    pub status: LogStatus,
    pub error: Option<String>,
    pub timestamp_ms: i64,
    /// Truncated prompt (first 200 chars)
    pub prompt_preview: String,
    /// Truncated response (first 200 chars)
    pub response_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStatus {
    Success,
    Error,
    RateLimited,
    GuardrailBlocked,
    CacheHit,
}

pub struct RequestLogger {
    logs: DashMap<String, RequestLog>,
    /// Consumer → request count
    counter: DashMap<String, AtomicU64>,
    max_entries: usize,
}

impl RequestLogger {
    pub fn new(max_entries: usize) -> Self {
        Self {
            logs: DashMap::new(),
            counter: DashMap::new(),
            max_entries,
        }
    }

    pub fn log_success(
        &self,
        consumer: &str,
        provider: &str,
        req: &ChatCompletionRequest,
        resp: &ChatCompletionResponse,
        latency_ms: u64,
        cache_hit: bool,
    ) -> String {
        let prompt_preview = req.messages.last()
            .and_then(|m| m.content.as_text())
            .map(|t| t.chars().take(200).collect::<String>())
            .unwrap_or_default();

        let response_preview = resp.choices.first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.as_text())
            .map(|t| t.chars().take(200).collect::<String>())
            .unwrap_or_default();

        let log = RequestLog {
            id: Uuid::new_v4().to_string(),
            consumer: consumer.to_string(),
            provider: provider.to_string(),
            model: req.model.clone(),
            input_tokens: resp.usage.prompt_tokens,
            output_tokens: resp.usage.completion_tokens,
            latency_ms,
            status: if cache_hit { LogStatus::CacheHit } else { LogStatus::Success },
            error: None,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            prompt_preview,
            response_preview,
        };

        self.insert(log)
    }

    pub fn log_error(
        &self,
        consumer: &str,
        provider: &str,
        model: &str,
        latency_ms: u64,
        status: LogStatus,
        error: &str,
    ) -> String {
        let log = RequestLog {
            id: Uuid::new_v4().to_string(),
            consumer: consumer.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            input_tokens: 0,
            output_tokens: 0,
            latency_ms,
            status,
            error: Some(error.to_string()),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            prompt_preview: String::new(),
            response_preview: String::new(),
        };

        self.insert(log)
    }

    fn insert(&self, log: RequestLog) -> String {
        // Evict oldest if over limit
        if self.logs.len() >= self.max_entries {
            if let Some(old_key) = self.logs.iter().next().map(|e| e.key().clone()) {
                self.logs.remove(&old_key);
            }
        }

        // Increment consumer counter
        self.counter
            .entry(log.consumer.clone())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);

        let id = log.id.clone();
        self.logs.insert(id.clone(), log);
        id
    }

    pub fn get(&self, id: &str) -> Option<RequestLog> {
        self.logs.get(id).map(|e| e.clone())
    }

    pub fn list(&self, limit: usize) -> Vec<RequestLog> {
        let mut logs: Vec<RequestLog> = self.logs.iter().map(|e| e.value().clone()).collect();
        logs.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
        logs.truncate(limit);
        logs
    }

    pub fn list_for_consumer(&self, consumer: &str, limit: usize) -> Vec<RequestLog> {
        let mut logs: Vec<RequestLog> = self.logs.iter()
            .filter(|e| e.value().consumer == consumer)
            .map(|e| e.value().clone())
            .collect();
        logs.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
        logs.truncate(limit);
        logs
    }

    pub fn consumer_request_count(&self, consumer: &str) -> u64 {
        self.counter.get(consumer).map(|c| c.load(Ordering::Relaxed)).unwrap_or(0)
    }
}

impl Default for RequestLogger {
    fn default() -> Self {
        Self::new(10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ChatMessage, Usage};

    fn make_req() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage::user("what is rust?")],
            temperature: None, top_p: None, max_tokens: None, stream: None,
            stop: None, presence_penalty: None, frequency_penalty: None,
            n: None, user: None, tools: None, tool_choice: None,
            response_format: None, seed: None, logprobs: None,
        }
    }

    #[test]
    fn log_success_and_retrieve() {
        let logger = RequestLogger::new(100);
        let req = make_req();
        let resp = ChatCompletionResponse::simple("gpt-4o", "Rust is a systems language".into(), Usage::new(10, 20));
        let id = logger.log_success("alice", "openai", &req, &resp, 250, false);
        let log = logger.get(&id).unwrap();
        assert_eq!(log.consumer, "alice");
        assert_eq!(log.status, LogStatus::Success);
        assert_eq!(log.latency_ms, 250);
    }

    #[test]
    fn consumer_counter() {
        let logger = RequestLogger::new(100);
        let req = make_req();
        let resp = ChatCompletionResponse::simple("gpt-4o", "hi".into(), Usage::new(5, 5));
        logger.log_success("bob", "openai", &req, &resp, 100, false);
        logger.log_success("bob", "openai", &req, &resp, 100, false);
        assert_eq!(logger.consumer_request_count("bob"), 2);
    }
}
