//! cave-local-llm — offline draft-generation daemon using Ollama.
//!
//! Phase 1: Ollama client, manifest reader, draft format, metrics, CLI binary.
//! Phase 3: Priority queue, scheduler guardrails, 24/7 daemon loop.

pub mod daemon;
pub mod draft;
pub mod manifest;
pub mod metrics;
pub mod ollama;
pub mod queue;
pub mod scheduler;
