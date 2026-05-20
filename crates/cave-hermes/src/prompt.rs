// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provider-specific system-prompt assembly.
//!
//! Hermes Agent's [`agent/prompt_builder.py`] ships one canonical system
//! prompt with per-provider mangling: Anthropic wants XML-tagged tool
//! descriptions and recalled-memory blocks, OpenAI wants
//! JSON-schema-shaped tool descriptions and plain-text memory recall,
//! Ollama is the lowest common denominator (plain text everywhere), and
//! OpenRouter forwards whichever upstream the caller chose — we treat
//! OpenRouter as OpenAI-flavoured by default.
//!
//! This module ports the *assembly* layer only: the canonical
//! [`PromptContext`] is provider-agnostic, and each
//! [`ProviderPrompt`] impl knows how to spell out:
//!
//! 1. The persona / role preamble.
//! 2. The tool catalogue, in the provider's preferred shape.
//! 3. The recalled-memory block, fenced so the model can ignore it.
//!
//! The 1.5 kLOC of upstream prompt-mangling that handles model-specific
//! quirks (auth-claim mangling, header overrides, context-window
//! warnings, etc.) lives in [`crate::router::ModelProfile`] and the
//! gateway layer; this module is *only* prompt text.

use serde::{Deserialize, Serialize};

use crate::error::HermesError;
use crate::memory::MemoryRecord;
use crate::tool::ToolEntry;

/// Tags every provider we support.
///
/// Hermes upstream additionally enumerates Gemini / Mistral / Groq /
/// Cohere / DeepSeek; those all fall under one of the four buckets we
/// model here (OpenAI-schema for Gemini & Mistral & Cohere, Ollama-text
/// for self-hosted Groq/DeepSeek inference). We keep the enum narrow
/// rather than introducing a knob-per-vendor — Hermes' own routing
/// table does the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
    OpenRouter,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Ollama => "ollama",
            ProviderKind::OpenRouter => "openrouter",
        }
    }
}

/// Inputs to [`ProviderPrompt::assemble`].
///
/// Construction is intentionally builder-shaped: callers can ship a
/// partial context (no tools, no memory) and the assembler will still
/// produce a valid prompt — empty sections are elided.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    pub persona: String,
    pub task: String,
    pub tools: Vec<ToolDescriptor>,
    pub memory: Vec<MemoryRecord>,
}

/// Trim of [`ToolEntry`] used by the assembler — we only need the
/// rendering surface, not the executor closure.
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

impl From<&ToolEntry> for ToolDescriptor {
    fn from(t: &ToolEntry) -> Self {
        Self {
            name: t.name.clone(),
            description: t.description.clone(),
            schema: t.schema.clone(),
        }
    }
}

impl PromptContext {
    pub fn new(persona: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            persona: persona.into(),
            task: task.into(),
            tools: Vec::new(),
            memory: Vec::new(),
        }
    }

    pub fn with_tools(mut self, tools: Vec<ToolDescriptor>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_memory(mut self, memory: Vec<MemoryRecord>) -> Self {
        self.memory = memory;
        self
    }
}

/// Per-provider prompt assembler. Implementors render
/// [`PromptContext`] into the raw system-prompt text expected by their
/// provider's chat API.
pub trait ProviderPrompt: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn assemble(&self, cx: &PromptContext) -> Result<String, HermesError>;
}

// ── Anthropic ────────────────────────────────────────────────────────────────
//
// Anthropic's official guidance recommends XML-style tags for both tool
// descriptions and recalled memory; the model is trained to attend to
// fenced regions. We follow the same conventions Hermes does
// (`<tools>`, `<tool>`, `<memory-context>`).

#[derive(Default)]
pub struct AnthropicPrompt;

impl AnthropicPrompt {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderPrompt for AnthropicPrompt {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn assemble(&self, cx: &PromptContext) -> Result<String, HermesError> {
        if cx.task.trim().is_empty() {
            return Err(HermesError::PlannerRejected("empty task".into()));
        }
        let mut out = String::new();
        if !cx.persona.trim().is_empty() {
            out.push_str(&format!("<persona>\n{}\n</persona>\n", cx.persona.trim()));
        }
        if !cx.tools.is_empty() {
            out.push_str("<tools>\n");
            for t in &cx.tools {
                let schema_pretty =
                    serde_json::to_string_pretty(&t.schema).map_err(HermesError::Json)?;
                out.push_str(&format!(
                    "  <tool name=\"{}\">\n    <description>{}</description>\n    <schema>{}</schema>\n  </tool>\n",
                    t.name,
                    xml_escape(&t.description),
                    schema_pretty,
                ));
            }
            out.push_str("</tools>\n");
        }
        if !cx.memory.is_empty() {
            out.push_str("<memory-context>\n");
            for r in &cx.memory {
                out.push_str(&format!(
                    "  <fact id=\"{}\">{}</fact>\n",
                    r.id,
                    xml_escape(&r.body)
                ));
            }
            out.push_str("</memory-context>\n");
        }
        out.push_str(&format!(
            "<task>\n{}\n</task>\n",
            xml_escape(cx.task.trim())
        ));
        Ok(out)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ── OpenAI ───────────────────────────────────────────────────────────────────
//
// OpenAI's `tools` payload is JSON-typed; we surface tool catalogues as
// JSON arrays. Recalled memory goes into a plain-text "Context:"
// section since the chat API has no fence convention.

#[derive(Default)]
pub struct OpenAiPrompt;

impl OpenAiPrompt {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderPrompt for OpenAiPrompt {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    fn assemble(&self, cx: &PromptContext) -> Result<String, HermesError> {
        if cx.task.trim().is_empty() {
            return Err(HermesError::PlannerRejected("empty task".into()));
        }
        let mut out = String::new();
        if !cx.persona.trim().is_empty() {
            out.push_str(&format!("System persona: {}\n\n", cx.persona.trim()));
        }
        if !cx.tools.is_empty() {
            let tools_payload: Vec<serde_json::Value> = cx
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.schema,
                        },
                    })
                })
                .collect();
            let rendered =
                serde_json::to_string_pretty(&tools_payload).map_err(HermesError::Json)?;
            out.push_str(&format!("Available tools (JSON):\n{}\n\n", rendered));
        }
        if !cx.memory.is_empty() {
            out.push_str("Context (recalled memory; treat as background, not new user input):\n");
            for r in &cx.memory {
                out.push_str(&format!("  - [{}] {}\n", r.id, r.body));
            }
            out.push('\n');
        }
        out.push_str(&format!("Task:\n{}\n", cx.task.trim()));
        Ok(out)
    }
}

// ── Ollama ───────────────────────────────────────────────────────────────────
//
// Ollama hosts arbitrary local models — many of which were *not*
// fine-tuned on either XML tags or OpenAI's tool envelope. The safe
// shape is plain text with numbered tool descriptions and a leading
// "MEMORY:" block.

#[derive(Default)]
pub struct OllamaPrompt;

impl OllamaPrompt {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderPrompt for OllamaPrompt {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    fn assemble(&self, cx: &PromptContext) -> Result<String, HermesError> {
        if cx.task.trim().is_empty() {
            return Err(HermesError::PlannerRejected("empty task".into()));
        }
        let mut out = String::new();
        if !cx.persona.trim().is_empty() {
            out.push_str(&format!("ROLE: {}\n\n", cx.persona.trim()));
        }
        if !cx.tools.is_empty() {
            out.push_str("TOOLS:\n");
            for (i, t) in cx.tools.iter().enumerate() {
                out.push_str(&format!("  {}. {} — {}\n", i + 1, t.name, t.description,));
            }
            out.push('\n');
        }
        if !cx.memory.is_empty() {
            out.push_str("MEMORY:\n");
            for r in &cx.memory {
                out.push_str(&format!("  - {}\n", r.body));
            }
            out.push('\n');
        }
        out.push_str(&format!("TASK:\n{}\n", cx.task.trim()));
        Ok(out)
    }
}

// ── OpenRouter ───────────────────────────────────────────────────────────────
//
// OpenRouter is a federated front-end; its on-wire format is whatever
// the routed-to model expects. We default to the OpenAI shape (which
// OpenRouter normalises to anyway) and forward through an internal
// [`OpenAiPrompt`].

pub struct OpenRouterPrompt {
    inner: OpenAiPrompt,
}

impl Default for OpenRouterPrompt {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenRouterPrompt {
    pub fn new() -> Self {
        Self {
            inner: OpenAiPrompt::new(),
        }
    }
}

impl ProviderPrompt for OpenRouterPrompt {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenRouter
    }

    fn assemble(&self, cx: &PromptContext) -> Result<String, HermesError> {
        // Same wire-shape as OpenAI; the marker line is what
        // OpenRouter's request envelope inspects.
        let mut body = self.inner.assemble(cx)?;
        body.insert_str(0, "[openrouter-passthrough]\n");
        Ok(body)
    }
}

/// Pick the right assembler for a [`ProviderKind`].
pub fn for_kind(kind: ProviderKind) -> Box<dyn ProviderPrompt> {
    match kind {
        ProviderKind::Anthropic => Box::new(AnthropicPrompt::new()),
        ProviderKind::OpenAi => Box::new(OpenAiPrompt::new()),
        ProviderKind::Ollama => Box::new(OllamaPrompt::new()),
        ProviderKind::OpenRouter => Box::new(OpenRouterPrompt::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryRecord;

    fn cx() -> PromptContext {
        PromptContext::new("a helpful researcher", "summarise the doc")
            .with_tools(vec![ToolDescriptor {
                name: "web_fetch".into(),
                description: "fetch a URL".into(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {"url": {"type": "string"}},
                    "required": ["url"]
                }),
            }])
            .with_memory(vec![MemoryRecord::new(
                "f1",
                "session",
                "user prefers concise answers",
            )])
    }

    #[test]
    fn anthropic_emits_xml_tags() {
        let p = AnthropicPrompt::new();
        let out = p.assemble(&cx()).unwrap();
        assert!(out.contains("<persona>"));
        assert!(out.contains("<tools>"));
        assert!(out.contains("<tool name=\"web_fetch\">"));
        assert!(out.contains("<memory-context>"));
        assert!(out.contains("<fact id=\"f1\">"));
        assert!(out.contains("<task>"));
    }

    #[test]
    fn anthropic_escapes_xml_specials() {
        let mut c = cx();
        c.task = "render <b>bold</b> & escaped".into();
        c.memory = vec![MemoryRecord::new("m", "s", "<oops>")];
        let out = AnthropicPrompt::new().assemble(&c).unwrap();
        assert!(out.contains("&lt;b&gt;bold&lt;/b&gt; &amp; escaped"));
        assert!(out.contains("&lt;oops&gt;"));
        // Raw <task> fences are emitted by us, not escaped from input.
        assert!(out.contains("<task>"));
    }

    #[test]
    fn openai_emits_json_tools_block() {
        let p = OpenAiPrompt::new();
        let out = p.assemble(&cx()).unwrap();
        assert!(out.contains("Available tools (JSON):"));
        assert!(out.contains("\"type\": \"function\""));
        assert!(out.contains("\"name\": \"web_fetch\""));
        assert!(out.contains("\"required\""));
        assert!(out.contains("Context (recalled memory"));
        assert!(out.contains("Task:"));
    }

    #[test]
    fn ollama_emits_plain_text_blocks() {
        let p = OllamaPrompt::new();
        let out = p.assemble(&cx()).unwrap();
        assert!(out.contains("ROLE:"));
        assert!(out.contains("TOOLS:"));
        assert!(out.contains("1. web_fetch"));
        assert!(out.contains("MEMORY:"));
        assert!(out.contains("TASK:"));
        // No XML and no JSON braces in the body itself.
        assert!(!out.contains("<tools>"));
        assert!(!out.contains("\"type\": \"function\""));
    }

    #[test]
    fn openrouter_prefixes_passthrough_marker() {
        let p = OpenRouterPrompt::new();
        let out = p.assemble(&cx()).unwrap();
        assert!(out.starts_with("[openrouter-passthrough]\n"));
        // and inherits OpenAI shape below.
        assert!(out.contains("Available tools (JSON):"));
    }

    #[test]
    fn empty_task_is_rejected_per_provider() {
        let cx = PromptContext::new("p", "   ");
        assert!(AnthropicPrompt::new().assemble(&cx).is_err());
        assert!(OpenAiPrompt::new().assemble(&cx).is_err());
        assert!(OllamaPrompt::new().assemble(&cx).is_err());
        assert!(OpenRouterPrompt::new().assemble(&cx).is_err());
    }

    #[test]
    fn missing_persona_and_tools_still_assembles() {
        let cx = PromptContext::new("", "do the thing");
        let out = OpenAiPrompt::new().assemble(&cx).unwrap();
        assert!(!out.contains("System persona"));
        assert!(!out.contains("Available tools"));
        assert!(out.contains("Task:\ndo the thing"));
    }

    #[test]
    fn for_kind_returns_matching_provider() {
        for k in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::Ollama,
            ProviderKind::OpenRouter,
        ] {
            assert_eq!(for_kind(k).kind(), k);
        }
    }

    #[test]
    fn provider_kind_as_str_is_stable() {
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert_eq!(ProviderKind::Ollama.as_str(), "ollama");
        assert_eq!(ProviderKind::OpenRouter.as_str(), "openrouter");
    }

    #[test]
    fn provider_kind_serde_roundtrip() {
        for k in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::Ollama,
            ProviderKind::OpenRouter,
        ] {
            let raw = serde_json::to_string(&k).unwrap();
            let back: ProviderKind = serde_json::from_str(&raw).unwrap();
            assert_eq!(k, back);
        }
    }
}
