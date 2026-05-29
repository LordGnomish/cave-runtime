// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core domain models for LLM trace/span/generation observability (Langfuse-compatible).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of an LLM trace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    Success,
    Error,
    Pending,
}

impl Default for TraceStatus {
    fn default() -> Self {
        TraceStatus::Success
    }
}

/// A top-level Langfuse trace: one end-to-end LLM interaction or pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub id: Uuid,
    pub name: String,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub metadata: serde_json::Value,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub status: TraceStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

/// A span within a trace (sub-step, retrieval, tool call, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub name: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub metadata: serde_json::Value,
    pub latency_ms: Option<u64>,
}

/// A generation is a span specifically tied to an LLM call, with token/cost details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Generation {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub name: String,
    pub model: String,
    pub model_parameters: serde_json::Value,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
}

/// Source of an evaluation score.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScoreSource {
    Human,
    Model,
    Api,
}

/// A score attached to a trace or generation (quality, relevance, safety, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub generation_id: Option<Uuid>,
    pub name: String,
    /// Numeric score value (e.g. 0.0–1.0).
    pub value: f64,
    pub comment: Option<String>,
    pub source: ScoreSource,
    pub created_at: DateTime<Utc>,
}

/// Summary of a session (a group of traces for one user conversation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub trace_count: usize,
    pub first_trace_at: DateTime<Utc>,
    pub last_trace_at: DateTime<Utc>,
    pub user_ids: Vec<String>,
}

/// A versioned prompt template (Langfuse Prompt Management).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub id: Uuid,
    pub name: String,
    pub version: u32,
    /// Template content with `{{variable}}` placeholders.
    pub content: String,
    pub variables: Vec<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}
