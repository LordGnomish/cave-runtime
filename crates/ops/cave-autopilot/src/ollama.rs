// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ollama HTTP client (local LLM tiers L1 + L2).
//!
//! Talks the `/api/generate` and `/api/tags` endpoints of a local Ollama
//! server (default `http://localhost:11434`). Request *construction* and
//! response *parsing* are pure functions so they can be unit-tested without a
//! live server; only [`OllamaClient::generate`] / [`OllamaClient::list_models`]
//! perform real I/O.
//!
//! Model resolution is deliberate: the daemon is configured with the *named*
//! tier models (`mellum2:12b-moe`, `qwen3-coder-next:80b-moe`), but if those
//! are not pulled it falls back to whatever resident coding model exists
//! (e.g. `qwen3.6:35b-a3b-coding-mxfp8`). We never silently fail a tier just
//! because the aspirational model isn't on disk yet.

use crate::error::{AutopilotError, Result};
use serde::{Deserialize, Serialize};

/// Request body for `POST /api/generate` (non-streaming).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GenerateRequest {
    pub model: String,
    pub prompt: String,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub options: GenerateOptions,
}

/// The subset of Ollama generation options the autopilot pins. Low temperature:
/// we want deterministic, compile-able code, not creativity.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GenerateOptions {
    pub temperature: f32,
    pub num_ctx: u32,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            temperature: 0.1,
            num_ctx: 16384,
        }
    }
}

/// Response body from `/api/generate`.
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateResponse {
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Debug, Clone, Deserialize)]
struct TagModel {
    name: String,
}

/// Client bound to one Ollama base URL.
#[derive(Debug, Clone)]
pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// Pure request builder — no I/O.
    pub fn build_generate_request(
        model: &str,
        system: Option<&str>,
        prompt: &str,
    ) -> GenerateRequest {
        GenerateRequest {
            model: model.to_string(),
            prompt: prompt.to_string(),
            stream: false,
            system: system.map(|s| s.to_string()),
            options: GenerateOptions::default(),
        }
    }

    /// Pure response parser — no I/O.
    pub fn parse_generate_response(body: &str) -> Result<String> {
        let resp: GenerateResponse = serde_json::from_str(body)?;
        Ok(resp.response)
    }

    fn parse_tags(body: &str) -> Result<Vec<String>> {
        let t: TagsResponse = serde_json::from_str(body)?;
        Ok(t.models.into_iter().map(|m| m.name).collect())
    }

    /// Pick the preferred model if it's in the installed list, else the
    /// fallback if *it* is installed, else `None`.
    pub fn resolve_model(installed: &[String], preferred: &str, fallback: &str) -> Option<String> {
        if installed.iter().any(|m| m == preferred) {
            Some(preferred.to_string())
        } else if installed.iter().any(|m| m == fallback) {
            Some(fallback.to_string())
        } else {
            None
        }
    }

    /// List installed model names via `/api/tags`.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url);
        let body = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AutopilotError::Llm(format!("ollama /api/tags: {e}")))?
            .text()
            .await
            .map_err(|e| AutopilotError::Llm(format!("ollama tags body: {e}")))?;
        Self::parse_tags(&body)
    }

    /// True if any model is installed (used as a liveness probe).
    pub async fn is_up(&self) -> bool {
        self.list_models().await.is_ok()
    }

    /// Run a non-streaming generation and return the completion text.
    pub async fn generate(&self, model: &str, system: Option<&str>, prompt: &str) -> Result<String> {
        let req = Self::build_generate_request(model, system, prompt);
        let url = format!("{}/api/generate", self.base_url);
        let body = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AutopilotError::Llm(format!("ollama /api/generate: {e}")))?
            .text()
            .await
            .map_err(|e| AutopilotError::Llm(format!("ollama generate body: {e}")))?;
        Self::parse_generate_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_is_non_streaming_low_temp() {
        let r = OllamaClient::build_generate_request("qwen", Some("be terse"), "hello");
        assert_eq!(r.model, "qwen");
        assert!(!r.stream);
        assert_eq!(r.system.as_deref(), Some("be terse"));
        assert_eq!(r.options.temperature, 0.1);
        // Serializes with snake-case keys Ollama expects.
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["stream"], serde_json::json!(false));
        assert_eq!(j["options"]["num_ctx"], serde_json::json!(16384));
    }

    #[test]
    fn parse_generate_response_extracts_text() {
        let body = r#"{"model":"qwen","response":"fn main() {}","done":true}"#;
        assert_eq!(
            OllamaClient::parse_generate_response(body).unwrap(),
            "fn main() {}"
        );
    }

    #[test]
    fn parse_tags_lists_names() {
        let body = r#"{"models":[{"name":"qwen3.6:35b-a3b-coding-mxfp8"},{"name":"mellum2:12b-moe"}]}"#;
        let names = OllamaClient::parse_tags(body).unwrap();
        assert_eq!(names, vec!["qwen3.6:35b-a3b-coding-mxfp8", "mellum2:12b-moe"]);
    }

    #[test]
    fn resolve_prefers_named_then_fallback_then_none() {
        let installed = vec!["qwen3.6:35b-a3b-coding-mxfp8".to_string()];
        // preferred missing -> fallback (resident) wins
        assert_eq!(
            OllamaClient::resolve_model(&installed, "qwen3-coder-next:80b-moe", "qwen3.6:35b-a3b-coding-mxfp8"),
            Some("qwen3.6:35b-a3b-coding-mxfp8".to_string())
        );
        // preferred present -> preferred wins
        let both = vec![
            "qwen3-coder-next:80b-moe".to_string(),
            "qwen3.6:35b-a3b-coding-mxfp8".to_string(),
        ];
        assert_eq!(
            OllamaClient::resolve_model(&both, "qwen3-coder-next:80b-moe", "qwen3.6:35b-a3b-coding-mxfp8"),
            Some("qwen3-coder-next:80b-moe".to_string())
        );
        // neither -> None
        assert_eq!(
            OllamaClient::resolve_model(&[], "a", "b"),
            None
        );
    }

    #[test]
    fn new_trims_trailing_slash() {
        let c = OllamaClient::new("http://localhost:11434/");
        assert_eq!(c.base_url, "http://localhost:11434");
    }

    #[test]
    fn build_pull_request_is_non_streaming() {
        let r = OllamaClient::build_pull_request("mellum2:12b-moe");
        assert_eq!(r.name, "mellum2:12b-moe");
        assert!(!r.stream);
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["name"], serde_json::json!("mellum2:12b-moe"));
        assert_eq!(j["stream"], serde_json::json!(false));
    }

    #[test]
    fn parse_pull_status_detects_success() {
        // Non-streaming pull ends with a {"status":"success"} object.
        assert!(OllamaClient::pull_succeeded(r#"{"status":"success"}"#));
        // A failure carries an error field, never status=success.
        assert!(!OllamaClient::pull_succeeded(
            r#"{"error":"pull model manifest: file does not exist"}"#
        ));
        // Mid-stream progress lines are not success on their own.
        assert!(!OllamaClient::pull_succeeded(
            r#"{"status":"pulling manifest"}"#
        ));
    }

    #[test]
    fn resolve_tier_prefers_named_then_resident_fallback() {
        // The honest reality on this machine: named MoE tiers aren't pulled, so
        // both L1 and L2 resolve to the resident coding model.
        let installed = vec!["qwen3.6:35b-a3b-coding-mxfp8".to_string()];
        let tiers = OllamaClient::resolve_tiers(
            &installed,
            "mellum2:12b-moe",
            "qwen3-coder-next:80b-moe",
            "qwen3.6:35b-a3b-coding-mxfp8",
        );
        assert_eq!(tiers.router, "qwen3.6:35b-a3b-coding-mxfp8");
        assert_eq!(tiers.coder, "qwen3.6:35b-a3b-coding-mxfp8");
        // Both fell back, so neither named model was actually present.
        assert!(tiers.router_fell_back);
        assert!(tiers.coder_fell_back);
    }

    #[test]
    fn resolve_tiers_uses_named_when_present() {
        let installed = vec![
            "mellum2:12b-moe".to_string(),
            "qwen3-coder-next:80b-moe".to_string(),
            "qwen3.6:35b-a3b-coding-mxfp8".to_string(),
        ];
        let tiers = OllamaClient::resolve_tiers(
            &installed,
            "mellum2:12b-moe",
            "qwen3-coder-next:80b-moe",
            "qwen3.6:35b-a3b-coding-mxfp8",
        );
        assert_eq!(tiers.router, "mellum2:12b-moe");
        assert_eq!(tiers.coder, "qwen3-coder-next:80b-moe");
        assert!(!tiers.router_fell_back);
        assert!(!tiers.coder_fell_back);
    }
}
