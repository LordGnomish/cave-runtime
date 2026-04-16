//! In-process GGUF inference provider — runs the model inside the runtime
//! binary itself (no external server). Feature-gated behind `embedded-llm`
//! so default builds stay light.
//!
//! Default targets a coding model (Qwen2.5-Coder family) on Apple Silicon
//! via Metal. Other GGUF chat models work as long as their chat template is
//! one of the variants in `ChatTemplate`.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, ChatMessage, Role, Usage};
use crate::provider::LlmProvider;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

// ── Configuration ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedConfig {
    /// Provider name registered in the gateway (e.g. "embedded-coder").
    pub name: String,
    /// Absolute path to a GGUF file. `cave llm pull` writes here.
    pub model_path: PathBuf,
    /// Logical model id reported via `/v1/models` and accepted in requests.
    pub model_id: String,
    /// Context window in tokens.
    pub context_size: u32,
    /// Layers to offload to GPU. -1 = all (recommended on Apple Silicon).
    pub gpu_layers: i32,
    /// Batch size for prompt processing.
    pub batch_size: u32,
    /// Number of CPU threads for the parts that stay on CPU.
    pub threads: u32,
    /// Chat template applied to OpenAI-style messages before tokenisation.
    pub chat_template: ChatTemplate,
    /// Sampling defaults if the request omits them.
    pub default_temperature: f32,
    pub default_top_p: f32,
    pub default_max_tokens: u32,
}

impl Default for EmbeddedConfig {
    fn default() -> Self {
        Self {
            name: "embedded".into(),
            model_path: default_model_path("qwen2.5-coder-7b-instruct-q4_k_m.gguf"),
            model_id: "qwen2.5-coder-7b".into(),
            context_size: 16_384,
            gpu_layers: -1,
            batch_size: 512,
            threads: 6,
            chat_template: ChatTemplate::Qwen,
            default_temperature: 0.2,
            default_top_p: 0.95,
            default_max_tokens: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatTemplate {
    /// ChatML-style (Qwen2.5, Qwen2.5-Coder).
    Qwen,
    /// Llama 3 instruct format.
    Llama3,
    /// Mistral instruct `[INST] ... [/INST]`.
    Mistral,
    /// Phi-3 `<|user|> ... <|assistant|>`.
    Phi3,
}

impl ChatTemplate {
    /// Render OpenAI messages into a single prompt string for the model.
    pub fn render(&self, messages: &[ChatMessage]) -> String {
        match self {
            Self::Qwen => render_qwen(messages),
            Self::Llama3 => render_llama3(messages),
            Self::Mistral => render_mistral(messages),
            Self::Phi3 => render_phi3(messages),
        }
    }

    /// Sequences that should terminate generation for this template.
    pub fn stop_sequences(&self) -> &'static [&'static str] {
        match self {
            Self::Qwen => &["<|im_end|>", "<|endoftext|>"],
            Self::Llama3 => &["<|eot_id|>", "<|end_of_text|>"],
            Self::Mistral => &["</s>"],
            Self::Phi3 => &["<|end|>", "<|endoftext|>"],
        }
    }
}

fn role_text(m: &ChatMessage) -> &str {
    m.content.as_text().unwrap_or("")
}

fn render_qwen(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for m in messages {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool | Role::Function => "user",
        };
        out.push_str(&format!("<|im_start|>{role}\n{}<|im_end|>\n", role_text(m)));
    }
    out.push_str("<|im_start|>assistant\n");
    out
}

fn render_llama3(messages: &[ChatMessage]) -> String {
    let mut out = String::from("<|begin_of_text|>");
    for m in messages {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool | Role::Function => "user",
        };
        out.push_str(&format!(
            "<|start_header_id|>{role}<|end_header_id|>\n\n{}<|eot_id|>",
            role_text(m)
        ));
    }
    out.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
    out
}

fn render_mistral(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    let mut sys: Option<String> = None;
    for m in messages {
        match m.role {
            Role::System => sys = Some(role_text(m).to_string()),
            Role::User => {
                let content = match &sys.take() {
                    Some(s) => format!("{s}\n\n{}", role_text(m)),
                    None => role_text(m).to_string(),
                };
                out.push_str(&format!("[INST] {content} [/INST]"));
            }
            Role::Assistant => out.push_str(&format!(" {} </s>", role_text(m))),
            Role::Tool | Role::Function => out.push_str(&format!("[INST] {} [/INST]", role_text(m))),
        }
    }
    out
}

fn render_phi3(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for m in messages {
        let tag = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool | Role::Function => "user",
        };
        out.push_str(&format!("<|{tag}|>\n{}<|end|>\n", role_text(m)));
    }
    out.push_str("<|assistant|>\n");
    out
}

/// `~/.cave/models/<filename>` — keeps weights out of the repo, shared across
/// runtime / CLI / dev tools.
pub fn default_model_path(filename: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cave").join("models").join(filename)
}

// ── Provider implementation ───────────────────────────────────────────────────

pub struct EmbeddedProvider {
    config: EmbeddedConfig,
    inner: Arc<EmbeddedInner>,
}

impl EmbeddedProvider {
    pub fn new(config: EmbeddedConfig) -> GatewayResult<Self> {
        let inner = EmbeddedInner::load(&config)?;
        Ok(Self { config, inner: Arc::new(inner) })
    }

    pub fn config(&self) -> &EmbeddedConfig {
        &self.config
    }
}

#[async_trait]
impl LlmProvider for EmbeddedProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![self.config.model_id.clone()]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let prompt = self.config.chat_template.render(&req.messages);
        let max_tokens = req.max_tokens.unwrap_or(self.config.default_max_tokens);
        let temperature = req.temperature.unwrap_or(self.config.default_temperature);
        let top_p = req.top_p.unwrap_or(self.config.default_top_p);

        let inner = Arc::clone(&self.inner);
        let model_id = self.config.model_id.clone();
        let stop = self.config.chat_template.stop_sequences();

        // llama.cpp inference is blocking; run on a dedicated thread so we
        // don't stall the tokio reactor.
        let (text, prompt_tokens, completion_tokens) = tokio::task::spawn_blocking(move || {
            inner.generate(&prompt, max_tokens, temperature, top_p, stop)
        })
        .await
        .map_err(|e| GatewayError::Internal(format!("inference task join: {e}")))??;

        Ok(ChatCompletionResponse::simple(
            &model_id,
            text,
            Usage::new(prompt_tokens, completion_tokens),
        ))
    }

    async fn health_check(&self) -> bool {
        self.inner.is_loaded()
    }
}

// ── Inference backend ─────────────────────────────────────────────────────────

#[cfg(feature = "embedded-llm")]
mod backend {
    use super::*;
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::{AddBos, LlamaModel, Special};
    use llama_cpp_2::token::data_array::LlamaTokenDataArray;
    use parking_lot::Mutex;
    use std::num::NonZeroU32;
    use std::sync::OnceLock;

    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

    fn backend() -> GatewayResult<&'static LlamaBackend> {
        BACKEND
            .get_or_init(|| LlamaBackend::init().expect("llama backend init"));
        Ok(BACKEND.get().unwrap())
    }

    pub struct EmbeddedInner {
        model: LlamaModel,
        ctx_params: LlamaContextParams,
        // Single-flight inference; llama.cpp contexts are not Send across awaits.
        lock: Mutex<()>,
    }

    impl EmbeddedInner {
        pub fn load(cfg: &EmbeddedConfig) -> GatewayResult<Self> {
            if !cfg.model_path.exists() {
                return Err(GatewayError::Internal(format!(
                    "model file not found: {} — run `cave llm pull` first",
                    cfg.model_path.display()
                )));
            }
            let backend = backend()?;
            let model_params = LlamaModelParams::default().with_n_gpu_layers(cfg.gpu_layers as u32);
            let model = LlamaModel::load_from_file(backend, &cfg.model_path, &model_params)
                .map_err(|e| GatewayError::Internal(format!("load model: {e}")))?;

            let ctx_params = LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(cfg.context_size))
                .with_n_batch(cfg.batch_size)
                .with_n_threads(cfg.threads as i32)
                .with_n_threads_batch(cfg.threads as i32);

            Ok(Self { model, ctx_params, lock: Mutex::new(()) })
        }

        pub fn is_loaded(&self) -> bool {
            true
        }

        pub fn generate(
            &self,
            prompt: &str,
            max_tokens: u32,
            temperature: f32,
            top_p: f32,
            stop: &[&str],
        ) -> GatewayResult<(String, u32, u32)> {
            let _g = self.lock.lock();
            let backend = backend()?;
            let mut ctx = self
                .model
                .new_context(backend, self.ctx_params.clone())
                .map_err(|e| GatewayError::Internal(format!("new context: {e}")))?;

            let tokens = self
                .model
                .str_to_token(prompt, AddBos::Always)
                .map_err(|e| GatewayError::Internal(format!("tokenize: {e}")))?;
            let prompt_tokens = tokens.len() as u32;

            let mut batch = LlamaBatch::new(self.ctx_params.n_batch() as usize, 1);
            for (i, t) in tokens.iter().enumerate() {
                let last = i == tokens.len() - 1;
                batch
                    .add(*t, i as i32, &[0], last)
                    .map_err(|e| GatewayError::Internal(format!("batch add: {e}")))?;
            }
            ctx.decode(&mut batch)
                .map_err(|e| GatewayError::Internal(format!("decode prompt: {e}")))?;

            let mut output = String::new();
            let mut completion_tokens: u32 = 0;
            let mut cursor = tokens.len() as i32;
            let eos = self.model.token_eos();

            for _ in 0..max_tokens {
                let mut candidates = LlamaTokenDataArray::from_iter(
                    ctx.candidates_ith(batch.n_tokens() - 1),
                    false,
                );
                ctx.sample_top_p(&mut candidates, top_p, 1);
                ctx.sample_temp(&mut candidates, temperature);
                let next = ctx.sample_token(&mut candidates);

                if next == eos {
                    break;
                }
                let piece = self
                    .model
                    .token_to_str(next, Special::Tokenize)
                    .unwrap_or_default();
                output.push_str(&piece);
                completion_tokens += 1;

                if stop.iter().any(|s| output.ends_with(s)) {
                    for s in stop {
                        if let Some(idx) = output.rfind(s) {
                            output.truncate(idx);
                        }
                    }
                    break;
                }

                batch.clear();
                batch
                    .add(next, cursor, &[0], true)
                    .map_err(|e| GatewayError::Internal(format!("batch add: {e}")))?;
                cursor += 1;
                ctx.decode(&mut batch)
                    .map_err(|e| GatewayError::Internal(format!("decode step: {e}")))?;
            }

            Ok((output, prompt_tokens, completion_tokens))
        }
    }
}

#[cfg(not(feature = "embedded-llm"))]
mod backend {
    use super::*;

    /// Stub used when the `embedded-llm` feature is off — the type still
    /// exists so downstream code compiles, but loading always fails.
    pub struct EmbeddedInner;

    impl EmbeddedInner {
        pub fn load(_cfg: &EmbeddedConfig) -> GatewayResult<Self> {
            Err(GatewayError::Internal(
                "embedded LLM support not compiled in — rebuild with `--features embedded-llm`".into(),
            ))
        }

        pub fn is_loaded(&self) -> bool {
            false
        }

        pub fn generate(
            &self,
            _prompt: &str,
            _max_tokens: u32,
            _temperature: f32,
            _top_p: f32,
            _stop: &[&str],
        ) -> GatewayResult<(String, u32, u32)> {
            Err(GatewayError::Internal("embedded LLM not available".into()))
        }
    }
}

use backend::EmbeddedInner;

// ── Catalogue of curated GGUF models ──────────────────────────────────────────

/// A GGUF model the user can fetch with `cave llm pull <id>`. Hosted on
/// Hugging Face; URLs are direct downloads of the quantised weights.
#[derive(Debug, Clone, Serialize)]
pub struct ModelEntry {
    pub id: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub approx_size_mb: u32,
    pub context: u32,
    pub template: ChatTemplate,
    pub description: &'static str,
}

pub const CATALOG: &[ModelEntry] = &[
    ModelEntry {
        id: "qwen2.5-coder-7b",
        filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        approx_size_mb: 4_700,
        context: 32_768,
        template: ChatTemplate::Qwen,
        description: "Coding-tuned 7B; best fit for 16 GB Macs.",
    },
    ModelEntry {
        id: "qwen2.5-coder-14b",
        filename: "qwen2.5-coder-14b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-Coder-14B-Instruct-GGUF/resolve/main/qwen2.5-coder-14b-instruct-q4_k_m.gguf",
        approx_size_mb: 9_000,
        context: 32_768,
        template: ChatTemplate::Qwen,
        description: "Stronger coding model; needs ~24 GB unified RAM.",
    },
    ModelEntry {
        id: "qwen2.5-coder-32b",
        filename: "qwen2.5-coder-32b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-Coder-32B-Instruct-GGUF/resolve/main/qwen2.5-coder-32b-instruct-q4_k_m.gguf",
        approx_size_mb: 19_800,
        context: 32_768,
        template: ChatTemplate::Qwen,
        description: "Near-frontier coding quality; M-series with 48+ GB.",
    },
    ModelEntry {
        id: "llama-3.2-3b",
        filename: "llama-3.2-3b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        approx_size_mb: 2_000,
        context: 131_072,
        template: ChatTemplate::Llama3,
        description: "Small general-purpose model for log/alert summarisation.",
    },
];

pub fn lookup(id: &str) -> Option<&'static ModelEntry> {
    CATALOG.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_template_includes_system_and_assistant_marker() {
        let msgs = vec![
            ChatMessage::system("you are helpful"),
            ChatMessage::user("hi"),
        ];
        let p = ChatTemplate::Qwen.render(&msgs);
        assert!(p.contains("<|im_start|>system"));
        assert!(p.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn llama3_template_uses_header_tags() {
        let p = ChatTemplate::Llama3.render(&[ChatMessage::user("hello")]);
        assert!(p.contains("<|start_header_id|>user<|end_header_id|>"));
    }

    #[test]
    fn catalog_lookup_resolves_known_ids() {
        assert!(lookup("qwen2.5-coder-7b").is_some());
        assert!(lookup("nonexistent").is_none());
    }

    #[test]
    fn default_path_is_under_home_cave_models() {
        let p = default_model_path("foo.gguf");
        assert!(p.ends_with(".cave/models/foo.gguf") || p.ends_with(".cave\\models\\foo.gguf"));
    }
}
