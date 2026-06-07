// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Claude API client (escalation tier L3).
//!
//! Thin wrapper over the Anthropic Messages API. The autopilot only reaches
//! here after the local coder has burned its retry budget on a task, so every
//! call is metered: [`ClaudeCompletion`] surfaces the token usage that the
//! daemon books against the daily budget. Request construction and response
//! parsing are pure for testability; [`ClaudeClient::complete`] is the only
//! method that performs I/O.

use crate::error::{AutopilotError, Result};
use serde::{Deserialize, Serialize};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

/// Request body for the Messages API.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// What a completion returns: the assembled text plus token accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCompletion {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl ClaudeCompletion {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    content: Vec<RawContentBlock>,
    #[serde(default)]
    usage: RawUsage,
}

#[derive(Debug, Deserialize)]
struct RawContentBlock {
    #[serde(default, rename = "type")]
    _ty: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

/// Client carrying the API key + target model.
#[derive(Debug, Clone)]
pub struct ClaudeClient {
    api_key: String,
    model: String,
    base_url: String,
    http: reqwest::Client,
}

impl ClaudeClient {
    /// Build from an explicit key.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: API_URL.to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// Build from the `ANTHROPIC_API_KEY` environment variable. Returns
    /// `Config` error if unset so the daemon can degrade to local-only mode
    /// instead of panicking.
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| AutopilotError::Config("ANTHROPIC_API_KEY not set".into()))?;
        Ok(Self::new(key, model))
    }

    /// Whether a key is available without constructing a client.
    pub fn key_available() -> bool {
        std::env::var("ANTHROPIC_API_KEY")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    }

    /// Pure request builder.
    pub fn build_request(
        model: &str,
        system: Option<&str>,
        user: &str,
        max_tokens: u32,
    ) -> MessagesRequest {
        MessagesRequest {
            model: model.to_string(),
            max_tokens,
            system: system.map(|s| s.to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: user.to_string(),
            }],
        }
    }

    /// Pure response parser: concatenate text blocks, lift usage.
    pub fn parse_response(body: &str) -> Result<ClaudeCompletion> {
        let raw: RawResponse = serde_json::from_str(body)
            .map_err(|e| AutopilotError::Llm(format!("claude response parse: {e}")))?;
        let text = raw
            .content
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        Ok(ClaudeCompletion {
            text,
            input_tokens: raw.usage.input_tokens,
            output_tokens: raw.usage.output_tokens,
        })
    }

    /// Run one completion against the Messages API.
    pub async fn complete(
        &self,
        system: Option<&str>,
        user: &str,
        max_tokens: u32,
    ) -> Result<ClaudeCompletion> {
        let req = Self::build_request(&self.model, system, user, max_tokens);
        let body = self
            .http
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&req)
            .send()
            .await
            .map_err(|e| AutopilotError::Llm(format!("claude request: {e}")))?
            .text()
            .await
            .map_err(|e| AutopilotError::Llm(format!("claude body: {e}")))?;
        Self::parse_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_shapes_messages_api() {
        let r = ClaudeClient::build_request("claude-opus-4-7", Some("sys"), "fix this", 4096);
        assert_eq!(r.model, "claude-opus-4-7");
        assert_eq!(r.max_tokens, 4096);
        assert_eq!(r.system.as_deref(), Some("sys"));
        assert_eq!(r.messages.len(), 1);
        assert_eq!(r.messages[0].role, "user");
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["messages"][0]["content"], serde_json::json!("fix this"));
    }

    #[test]
    fn parse_response_concatenates_blocks_and_lifts_usage() {
        let body = r#"{
            "content":[{"type":"text","text":"part1 "},{"type":"text","text":"part2"}],
            "usage":{"input_tokens":120,"output_tokens":45}
        }"#;
        let c = ClaudeClient::parse_response(body).unwrap();
        assert_eq!(c.text, "part1 part2");
        assert_eq!(c.input_tokens, 120);
        assert_eq!(c.output_tokens, 45);
        assert_eq!(c.total_tokens(), 165);
    }

    #[test]
    fn parse_response_tolerates_missing_usage() {
        let body = r#"{"content":[{"type":"text","text":"ok"}]}"#;
        let c = ClaudeClient::parse_response(body).unwrap();
        assert_eq!(c.text, "ok");
        assert_eq!(c.total_tokens(), 0);
    }
}
