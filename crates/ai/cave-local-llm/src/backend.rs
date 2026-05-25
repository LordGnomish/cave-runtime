// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inference backend trait — abstracts Ollama, OpenAI-compat, and future
//! llama.cpp-server / vLLM clients behind one async surface.
//!
//! Cite: this module is the cave-side abstraction (not an upstream port).
//! It mirrors the spirit of LangChain's `BaseChatModel` / LiteLLM's
//! `ModelResponse` — a thin shape over "complete chat" + "generate
//! text" + "embed text" so the daemon, scheduler, and downstream
//! cave-hermes / cave-llm-gateway can swap providers without rewriting
//! their consumers.

use crate::ollama::{ChatMessage, OllamaClient, OllamaError};
use crate::ollama_extras::{EmbedInput, EmbedRequest, OllamaLifecycle};
use crate::openai_compat::{
    OpenAiChatMessage, OpenAiChatRequest, OpenAiCompatClient, OpenAiEmbeddingInput,
    OpenAiEmbeddingRequest,
};
use std::collections::HashMap;
use std::pin::Pin;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("backend error from Ollama client: {0}")]
    Ollama(#[from] OllamaError),
    #[error("backend not configured: {0}")]
    NotConfigured(String),
    #[error("backend produced no output")]
    EmptyOutput,
}

pub type BackendResult<T> = Result<T, BackendError>;

/// Request shape consumed by [`InferenceBackend::chat`]. cave defines its
/// own request type so the trait stays agnostic of the underlying client's
/// schema (Ollama uses `ChatMessage`, OpenAI uses `OpenAiChatMessage`,
/// llama.cpp speaks JSON-RPC). Adapters live in this module.
#[derive(Debug, Clone)]
pub struct BackendChatRequest {
    pub model: String,
    pub messages: Vec<BackendMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct BackendMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct BackendChatResponse {
    pub model: String,
    pub content: String,
    pub finish_reason: Option<String>,
    pub usage: BackendUsage,
}

#[derive(Debug, Clone, Default)]
pub struct BackendUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct BackendEmbedRequest {
    pub model: String,
    pub input: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BackendEmbedResponse {
    pub model: String,
    pub embeddings: Vec<Vec<f64>>,
}

/// The async surface — kept narrow (chat + embed + name) so swapping a
/// backend is a one-line config change in the daemon.
///
/// We hand-write the boxed-future return type instead of pulling in
/// `async-trait` so the crate stays dep-clean.
pub trait InferenceBackend: Send + Sync {
    fn name(&self) -> &str;

    fn chat<'a>(
        &'a self,
        req: BackendChatRequest,
    ) -> Pin<
        Box<dyn std::future::Future<Output = BackendResult<BackendChatResponse>> + Send + 'a>,
    >;

    fn embed<'a>(
        &'a self,
        req: BackendEmbedRequest,
    ) -> Pin<
        Box<dyn std::future::Future<Output = BackendResult<BackendEmbedResponse>> + Send + 'a>,
    >;
}

/// Adapter: wrap an [`OllamaClient`] as an [`InferenceBackend`].
pub struct OllamaBackend {
    client: OllamaClient,
}

impl OllamaBackend {
    pub fn new(client: OllamaClient) -> Self {
        Self { client }
    }
}

impl InferenceBackend for OllamaBackend {
    fn name(&self) -> &str {
        "ollama"
    }

    fn chat<'a>(
        &'a self,
        req: BackendChatRequest,
    ) -> Pin<
        Box<dyn std::future::Future<Output = BackendResult<BackendChatResponse>> + Send + 'a>,
    > {
        Box::pin(async move {
            let mut options = serde_json::Map::new();
            if let Some(t) = req.temperature {
                options.insert("temperature".into(), serde_json::json!(t));
            }
            if let Some(s) = req.seed {
                options.insert("seed".into(), serde_json::json!(s));
            }
            if let Some(m) = req.max_tokens {
                options.insert("num_predict".into(), serde_json::json!(m));
            }
            let chat_req = crate::ollama::ChatRequest {
                model: req.model,
                messages: req
                    .messages
                    .into_iter()
                    .map(|m| ChatMessage {
                        role: m.role,
                        content: m.content,
                    })
                    .collect(),
                stream: Some(false),
                options: if options.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(options))
                },
            };
            let r = self.client.chat(chat_req).await?;
            Ok(BackendChatResponse {
                model: r.model,
                content: r.message.content,
                finish_reason: r.done.then(|| "stop".to_string()),
                usage: BackendUsage::default(),
            })
        })
    }

    fn embed<'a>(
        &'a self,
        req: BackendEmbedRequest,
    ) -> Pin<
        Box<dyn std::future::Future<Output = BackendResult<BackendEmbedResponse>> + Send + 'a>,
    > {
        Box::pin(async move {
            let lc = OllamaLifecycle::from(&self.client);
            let ereq = EmbedRequest {
                model: req.model.clone(),
                input: if req.input.len() == 1 {
                    EmbedInput::One(req.input.into_iter().next().unwrap())
                } else {
                    EmbedInput::Many(req.input)
                },
                options: None,
            };
            let r = lc.embed(ereq).await?;
            Ok(BackendEmbedResponse {
                model: r.model,
                embeddings: r.embeddings,
            })
        })
    }
}

/// Adapter: wrap an [`OpenAiCompatClient`] as an [`InferenceBackend`].
pub struct OpenAiCompatBackend {
    client: OpenAiCompatClient,
}

impl OpenAiCompatBackend {
    pub fn new(client: OpenAiCompatClient) -> Self {
        Self { client }
    }
}

impl InferenceBackend for OpenAiCompatBackend {
    fn name(&self) -> &str {
        "openai-compat"
    }

    fn chat<'a>(
        &'a self,
        req: BackendChatRequest,
    ) -> Pin<
        Box<dyn std::future::Future<Output = BackendResult<BackendChatResponse>> + Send + 'a>,
    > {
        Box::pin(async move {
            let chat_req = OpenAiChatRequest {
                model: req.model,
                messages: req
                    .messages
                    .into_iter()
                    .map(|m| OpenAiChatMessage {
                        role: m.role,
                        content: m.content,
                    })
                    .collect(),
                temperature: req.temperature,
                max_tokens: req.max_tokens,
                top_p: None,
                stream: Some(false),
                seed: req.seed,
                stop: None,
            };
            let r = self.client.chat_completions(chat_req).await?;
            let choice = r.choices.into_iter().next().ok_or(BackendError::EmptyOutput)?;
            Ok(BackendChatResponse {
                model: r.model,
                content: choice.message.content,
                finish_reason: choice.finish_reason,
                usage: BackendUsage {
                    prompt_tokens: r.usage.prompt_tokens,
                    completion_tokens: r.usage.completion_tokens,
                    total_tokens: r.usage.total_tokens,
                },
            })
        })
    }

    fn embed<'a>(
        &'a self,
        req: BackendEmbedRequest,
    ) -> Pin<
        Box<dyn std::future::Future<Output = BackendResult<BackendEmbedResponse>> + Send + 'a>,
    > {
        Box::pin(async move {
            let ereq = OpenAiEmbeddingRequest {
                model: req.model.clone(),
                input: if req.input.len() == 1 {
                    OpenAiEmbeddingInput::One(req.input.into_iter().next().unwrap())
                } else {
                    OpenAiEmbeddingInput::Many(req.input)
                },
            };
            let r = self.client.embeddings(ereq).await?;
            Ok(BackendEmbedResponse {
                model: r.model,
                embeddings: r.data.into_iter().map(|d| d.embedding).collect(),
            })
        })
    }
}

/// Registry — name → backend. The daemon registers backends at startup
/// and the scheduler picks one by name per request.
#[derive(Default)]
pub struct BackendRegistry {
    backends: HashMap<String, Box<dyn InferenceBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, backend: Box<dyn InferenceBackend>) {
        self.backends.insert(name.into(), backend);
    }

    pub fn get(&self, name: &str) -> Option<&dyn InferenceBackend> {
        self.backends.get(name).map(|b| b.as_ref())
    }

    pub fn names(&self) -> Vec<&str> {
        self.backends.keys().map(String::as_str).collect()
    }

    pub fn len(&self) -> usize {
        self.backends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeBackend(pub String);
    impl InferenceBackend for FakeBackend {
        fn name(&self) -> &str {
            "fake"
        }
        fn chat<'a>(
            &'a self,
            _req: BackendChatRequest,
        ) -> Pin<
            Box<dyn std::future::Future<Output = BackendResult<BackendChatResponse>> + Send + 'a>,
        > {
            let model = self.0.clone();
            Box::pin(async move {
                Ok(BackendChatResponse {
                    model,
                    content: "hello".to_string(),
                    finish_reason: Some("stop".into()),
                    usage: BackendUsage::default(),
                })
            })
        }
        fn embed<'a>(
            &'a self,
            _req: BackendEmbedRequest,
        ) -> Pin<
            Box<dyn std::future::Future<Output = BackendResult<BackendEmbedResponse>> + Send + 'a>,
        > {
            let model = self.0.clone();
            Box::pin(async move {
                Ok(BackendEmbedResponse {
                    model,
                    embeddings: vec![vec![0.0, 0.0]],
                })
            })
        }
    }

    #[test]
    fn registry_register_and_get() {
        let mut r = BackendRegistry::new();
        r.register("a", Box::new(FakeBackend("qwen".into())));
        assert_eq!(r.len(), 1);
        assert!(r.get("a").is_some());
        assert!(r.get("missing").is_none());
    }

    #[test]
    fn registry_names_reports_registered() {
        let mut r = BackendRegistry::new();
        r.register("a", Box::new(FakeBackend("qwen".into())));
        r.register("b", Box::new(FakeBackend("llama".into())));
        let mut n = r.names();
        n.sort();
        assert_eq!(n, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn fake_backend_chat_returns_message() {
        let b = FakeBackend("qwen".into());
        let r = b
            .chat(BackendChatRequest {
                model: "qwen".into(),
                messages: vec![],
                temperature: None,
                max_tokens: None,
                seed: None,
            })
            .await
            .unwrap();
        assert_eq!(r.content, "hello");
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn fake_backend_embed_returns_vector() {
        let b = FakeBackend("qwen".into());
        let r = b
            .embed(BackendEmbedRequest {
                model: "qwen".into(),
                input: vec!["hello".into()],
            })
            .await
            .unwrap();
        assert_eq!(r.embeddings.len(), 1);
        assert_eq!(r.embeddings[0], vec![0.0, 0.0]);
    }

    #[test]
    fn backend_usage_default_is_zero() {
        let u = BackendUsage::default();
        assert_eq!(u.total_tokens, 0);
    }
}
