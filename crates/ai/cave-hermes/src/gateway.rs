// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LLM gateway — the *transport* layer Hermes' planner / runtime calls
//! when it actually needs a model.
//!
//! Hermes upstream tangles transport with planning inside
//! `providers/{openai,anthropic,ollama}.py`. We separate the two
//! concerns: this module exposes a minimal [`LlmGateway`] async trait
//! plus two backends (Ollama via real HTTP, Anthropic stub for
//! parity-shape only — there's no API key in the workspace and we
//! refuse to leak one). The router / planner / runtime can hold an
//! `Arc<dyn LlmGateway>` and remain provider-agnostic.
//!
//! For sync callers (e.g. [`crate::planner::LlmPlanner`]) we expose
//! [`LlmGateway::complete_blocking`] which wraps `complete` with a
//! caller-supplied tokio [`tokio::runtime::Handle`].
//!
//! ## Why an async trait?
//!
//! Ollama is HTTP and reqwest is async-only. Anthropic-stub doesn't
//! need async per se, but homogenising the surface keeps callers
//! simple. `async-trait` is already a workspace dep so the cost is nil.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

use crate::error::HermesError;
use crate::prompt::ProviderKind;

/// One completion request — provider-neutral.
///
/// The two required fields mirror the universal chat shape: a system
/// prompt (provider-specific assembly is handled by
/// [`crate::prompt::ProviderPrompt`]) and one user message. Hermes
/// upstream supports n-turn message arrays; for the MVP we collapse
/// that into a single user turn because the planner/runtime loop owns
/// turn state externally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub stop: Vec<String>,
}

fn default_max_tokens() -> u32 {
    2048
}

impl CompletionRequest {
    pub fn new(
        model: impl Into<String>,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            system: system.into(),
            user: user.into(),
            max_tokens: default_max_tokens(),
            stop: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub text: String,
    pub provider: ProviderKind,
    pub model: String,
    /// Where available; Ollama reports `eval_count`, Anthropic stub
    /// counts UTF-8 chars / 4. Free-form because Hermes upstream uses
    /// it only as a budget signal.
    pub tokens: u32,
    /// Wall-clock for the call in milliseconds; useful for the router's
    /// degradation heuristics.
    pub latency_ms: u64,
}

#[async_trait]
pub trait LlmGateway: Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// Issue one completion. Implementors are expected to enforce their
    /// own timeouts; the runtime won't wrap the call.
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, HermesError>;

    /// Sync convenience for callers stuck in a sync context. Requires a
    /// live tokio runtime [`Handle`]; in non-tokio environments use
    /// `tokio::runtime::Builder::new_current_thread().enable_all()` and
    /// pass its `handle()`.
    fn complete_blocking(
        &self,
        rt: &Handle,
        req: &CompletionRequest,
    ) -> Result<CompletionResponse, HermesError> {
        tokio::task::block_in_place(|| rt.block_on(self.complete(req)))
    }
}

// ── Ollama gateway ───────────────────────────────────────────────────────────

/// Real HTTP backend pointing at a local Ollama daemon.
///
/// Hermes' upstream `providers/ollama.py` issues a POST to
/// `/api/generate` with a model-tagged JSON envelope; we mirror that
/// shape exactly. The default base URL `http://localhost:11434`
/// matches `cave-local-llm` and Ollama's own default.
pub struct OllamaGateway {
    base_url: String,
    client: reqwest::Client,
}

impl OllamaGateway {
    pub fn new(base_url: impl Into<String>) -> Result<Self, HermesError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| HermesError::PlannerRejected(format!("reqwest init: {e}")))?;
        Ok(Self {
            base_url: base_url.into(),
            client,
        })
    }

    pub fn localhost() -> Result<Self, HermesError> {
        Self::new("http://localhost:11434")
    }

    fn endpoint(&self) -> String {
        format!("{}/api/generate", self.base_url.trim_end_matches('/'))
    }
}

#[derive(Debug, Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: String,
    system: &'a str,
    stream: bool,
    options: OllamaOptions<'a>,
}

#[derive(Debug, Serialize)]
struct OllamaOptions<'a> {
    num_predict: u32,
    stop: &'a [String],
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    response: String,
    #[serde(default)]
    eval_count: u32,
}

#[async_trait]
impl LlmGateway for OllamaGateway {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, HermesError> {
        let body = OllamaRequest {
            model: &req.model,
            prompt: req.user.clone(),
            system: &req.system,
            stream: false,
            options: OllamaOptions {
                num_predict: req.max_tokens,
                stop: &req.stop,
            },
        };
        let started = std::time::Instant::now();
        let resp = self
            .client
            .post(self.endpoint())
            .json(&body)
            .send()
            .await
            .map_err(|e| HermesError::PlannerRejected(format!("ollama send: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(HermesError::PlannerRejected(format!(
                "ollama {status}: {txt}"
            )));
        }
        let parsed: OllamaResponse = resp
            .json()
            .await
            .map_err(|e| HermesError::PlannerRejected(format!("ollama decode: {e}")))?;
        Ok(CompletionResponse {
            text: parsed.response,
            provider: ProviderKind::Ollama,
            model: req.model.clone(),
            tokens: parsed.eval_count,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

// ── Anthropic stub gateway ──────────────────────────────────────────────────
//
// Hermes upstream's `providers/anthropic.py` issues `POST
// /v1/messages` with an `x-api-key` header. cave-runtime has no
// vault-issued Anthropic key, and the directive forbids us from
// shelling out a real API call. The stub mirrors the *interface* —
// caller-side it's indistinguishable — but it returns either a
// caller-injected canned response or a deterministic echo.
//
// This is the same posture Hermes' own test suite takes via its
// `MockAnthropicClient`; once a real API key surfaces in cave-vault
// the stub is hot-swappable for a true `reqwest`-backed adapter
// without touching the trait.

pub struct AnthropicStubGateway {
    canned: Option<String>,
}

impl Default for AnthropicStubGateway {
    fn default() -> Self {
        Self::echo()
    }
}

impl AnthropicStubGateway {
    /// A gateway that echoes the user message back, prefixed with
    /// `[anthropic-stub]`. Used for deterministic unit tests.
    pub fn echo() -> Self {
        Self { canned: None }
    }

    /// A gateway that returns exactly `text` for every request.
    pub fn with_canned(text: impl Into<String>) -> Self {
        Self {
            canned: Some(text.into()),
        }
    }
}

#[async_trait]
impl LlmGateway for AnthropicStubGateway {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, HermesError> {
        if req.user.trim().is_empty() {
            return Err(HermesError::PlannerRejected(
                "anthropic-stub: empty user message".into(),
            ));
        }
        let started = std::time::Instant::now();
        let text = match &self.canned {
            Some(t) => t.clone(),
            None => format!("[anthropic-stub] {}", req.user),
        };
        let tokens = (text.chars().count() as u32) / 4;
        Ok(CompletionResponse {
            text,
            provider: ProviderKind::Anthropic,
            model: req.model.clone(),
            tokens,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn anthropic_stub_echoes_user_message() {
        let gw = AnthropicStubGateway::echo();
        let req = CompletionRequest::new("claude-3-opus", "be brief", "ping");
        let resp = gw.complete(&req).await.unwrap();
        assert_eq!(resp.provider, ProviderKind::Anthropic);
        assert_eq!(resp.text, "[anthropic-stub] ping");
        assert!(resp.tokens > 0);
    }

    #[tokio::test]
    async fn anthropic_stub_returns_canned() {
        let gw = AnthropicStubGateway::with_canned("hello");
        let req = CompletionRequest::new("claude-3-opus", "be brief", "anything");
        let resp = gw.complete(&req).await.unwrap();
        assert_eq!(resp.text, "hello");
    }

    #[tokio::test]
    async fn anthropic_stub_rejects_empty_user() {
        let gw = AnthropicStubGateway::echo();
        let req = CompletionRequest::new("claude-3-opus", "x", "   ");
        let err = gw.complete(&req).await.unwrap_err();
        assert!(matches!(err, HermesError::PlannerRejected(_)));
    }

    #[test]
    fn ollama_endpoint_concatenates_correctly() {
        let g = OllamaGateway::new("http://localhost:11434/").unwrap();
        assert_eq!(g.endpoint(), "http://localhost:11434/api/generate");
        let g2 = OllamaGateway::new("http://localhost:11434").unwrap();
        assert_eq!(g2.endpoint(), "http://localhost:11434/api/generate");
    }

    #[tokio::test]
    async fn ollama_returns_error_on_unreachable_host() {
        // Pin to a port nothing is bound to; the call must fail fast
        // with `PlannerRejected`, never panic.
        let g = OllamaGateway::new("http://127.0.0.1:1").unwrap();
        let req = CompletionRequest::new("qwen3.6", "sys", "hi");
        let err = g.complete(&req).await.unwrap_err();
        assert!(matches!(err, HermesError::PlannerRejected(_)));
    }

    #[tokio::test]
    async fn ollama_surfaces_non_2xx_with_status() {
        // Stand up a one-shot HTTP listener that returns 503 to anyone
        // who connects, then verify our gateway maps that into
        // PlannerRejected.
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let resp = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\n\r\nnope";
                let _ = sock.write_all(resp).await;
                let _ = sock.shutdown().await;
            }
        });

        let g = OllamaGateway::new(format!("http://{addr}")).unwrap();
        let req = CompletionRequest::new("qwen3.6", "sys", "hi");
        let err = g.complete(&req).await.unwrap_err();
        let HermesError::PlannerRejected(reason) = err else {
            panic!("expected PlannerRejected");
        };
        assert!(reason.contains("503"));
        let _ = server.await;
    }

    #[tokio::test]
    async fn ollama_parses_canned_response() {
        // End-to-end happy path against a fake server that mimics
        // ollama's /api/generate response.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let _ = sock.read(&mut buf).await; // drain request
                let body = r#"{"response":"pong","eval_count":7}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let g = OllamaGateway::new(format!("http://{addr}")).unwrap();
        let req = CompletionRequest::new("qwen3.6", "sys", "ping");
        let resp = g.complete(&req).await.unwrap();
        assert_eq!(resp.text, "pong");
        assert_eq!(resp.provider, ProviderKind::Ollama);
        assert_eq!(resp.tokens, 7);
        let _ = server.await;
    }

    #[test]
    fn completion_request_default_max_tokens() {
        let r = CompletionRequest::new("m", "s", "u");
        assert_eq!(r.max_tokens, 2048);
        assert!(r.stop.is_empty());
    }

    #[test]
    fn completion_request_serde_roundtrip() {
        let r = CompletionRequest::new("m", "s", "u");
        let raw = serde_json::to_string(&r).unwrap();
        let back: CompletionRequest = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.model, "m");
        assert_eq!(back.max_tokens, 2048);
    }
}
