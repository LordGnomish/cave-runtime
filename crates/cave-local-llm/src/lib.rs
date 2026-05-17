// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-local-llm — offline draft-generation daemon using Ollama.
//!
//! Phase 1: Ollama client, manifest reader, draft format, metrics, CLI binary.
//! Phase 3: Priority queue, scheduler guardrails, 24/7 daemon loop.

/// Re-exports the daemon module for managing the main application lifecycle.
pub mod daemon;

/// Re-exports the draft module for handling LLM draft generation formats.
pub mod draft;

/// Re-exports the manifest module for reading configuration and model manifests.
pub mod manifest;

/// Re-exports the metrics module for collecting and exposing system metrics.
pub mod metrics;

/// Re-exports the Ollama client module for communicating with the local LLM server.
pub mod ollama;

/// Re-exports the queue module for managing priority-based task scheduling.
pub mod queue;

/// Re-exports the scheduler module for enforcing guardrails and execution policies.
pub mod scheduler;
