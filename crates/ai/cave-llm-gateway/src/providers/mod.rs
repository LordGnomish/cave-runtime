// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Concrete LLM provider implementations split per upstream backend.
//!
//! Each submodule implements [`crate::provider::LlmProvider`] for one wire
//! protocol. The split lets us cover all six MVP backends without bloating
//! `provider.rs`.

pub mod cohere;
pub mod google;
pub mod huggingface;
pub mod llama_cpp;
pub mod mistral;
pub mod mlx;
pub mod ollama;
pub mod replicate;
pub mod together;
