// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for traces, spans, generations, scores, and prompt templates.

use crate::trace_models::{Generation, PromptTemplate, Score, Session, Span, Trace};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

// Re-export TraceStatus for convenience.
pub use crate::trace_models::TraceStatus;

const MAX_RECORDS: usize = 50_000;

/// Central in-memory store for all AI observability records.
#[derive(Default)]
pub struct TraceStore {
    traces: RwLock<HashMap<Uuid, Trace>>,
    spans: RwLock<Vec<Span>>,
    generations: RwLock<Vec<Generation>>,
    scores: RwLock<Vec<Score>>,
    prompt_templates: RwLock<Vec<PromptTemplate>>,
}

impl TraceStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ─── Traces ───────────────────────────────────────────────────────────

    /// Insert or replace a trace (upsert by id).
    pub fn upsert_trace(&self, trace: Trace) {
        let mut traces = self.traces.write().unwrap();
        if traces.len() >= MAX_RECORDS && !traces.contains_key(&trace.id) {
            // evict oldest by created_at
            if let Some(oldest_id) = traces
                .values()
                .min_by_key(|t| t.created_at)
                .map(|t| t.id)
            {
                traces.remove(&oldest_id);
            }
        }
        traces.insert(trace.id, trace);
    }

    pub fn get_trace(&self, id: &Uuid) -> Option<Trace> {
        self.traces.read().unwrap().get(id).cloned()
    }

    /// List traces with optional filters; returns newest-first up to `limit`.
    pub fn list_traces(
        &self,
        user_id: Option<&str>,
        session_id: Option<&str>,
        tag: Option<&str>,
        limit: usize,
    ) -> Vec<Trace> {
        let traces = self.traces.read().unwrap();
        let mut results: Vec<Trace> = traces
            .values()
            .filter(|t| {
                if let Some(u) = user_id {
                    if t.user_id.as_deref() != Some(u) {
                        return false;
                    }
                }
                if let Some(s) = session_id {
                    if t.session_id.as_deref() != Some(s) {
                        return false;
                    }
                }
                if let Some(tg) = tag {
                    if !t.tags.iter().any(|x| x == tg) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        results.truncate(limit);
        results
    }

    // ─── Sessions ─────────────────────────────────────────────────────────

    /// Compute a session summary from the traces in the store.
    pub fn get_session(&self, session_id: &str) -> Option<Session> {
        let traces = self.traces.read().unwrap();
        let session_traces: Vec<&Trace> = traces
            .values()
            .filter(|t| t.session_id.as_deref() == Some(session_id))
            .collect();
        if session_traces.is_empty() {
            return None;
        }
        let first = session_traces.iter().map(|t| t.created_at).min().unwrap();
        let last = session_traces.iter().map(|t| t.created_at).max().unwrap();
        let mut user_ids: Vec<String> = session_traces
            .iter()
            .filter_map(|t| t.user_id.clone())
            .collect();
        user_ids.sort();
        user_ids.dedup();
        Some(Session {
            session_id: session_id.to_string(),
            trace_count: session_traces.len(),
            first_trace_at: first,
            last_trace_at: last,
            user_ids,
        })
    }

    // ─── Spans ────────────────────────────────────────────────────────────

    /// Insert or replace a span (upsert by id).
    pub fn upsert_span(&self, span: Span) {
        let mut spans = self.spans.write().unwrap();
        if let Some(pos) = spans.iter().position(|s| s.id == span.id) {
            spans[pos] = span;
        } else {
            if spans.len() >= MAX_RECORDS {
                spans.remove(0);
            }
            spans.push(span);
        }
    }

    pub fn get_spans_for_trace(&self, trace_id: &Uuid) -> Vec<Span> {
        self.spans
            .read()
            .unwrap()
            .iter()
            .filter(|s| s.trace_id == *trace_id)
            .cloned()
            .collect()
    }

    // ─── Generations ──────────────────────────────────────────────────────

    /// Insert or replace a generation (upsert by id).
    pub fn upsert_generation(&self, generation: Generation) {
        let mut gens = self.generations.write().unwrap();
        if let Some(pos) = gens.iter().position(|g| g.id == generation.id) {
            gens[pos] = generation;
        } else {
            if gens.len() >= MAX_RECORDS {
                gens.remove(0);
            }
            gens.push(generation);
        }
    }

    pub fn get_generations_for_trace(&self, trace_id: &Uuid) -> Vec<Generation> {
        self.generations
            .read()
            .unwrap()
            .iter()
            .filter(|g| g.trace_id == *trace_id)
            .cloned()
            .collect()
    }

    /// Return all generations across all traces.
    pub fn all_generations(&self) -> Vec<Generation> {
        self.generations.read().unwrap().clone()
    }

    // ─── Scores ───────────────────────────────────────────────────────────

    /// Insert or replace a score (upsert by id).
    pub fn upsert_score(&self, score: Score) {
        let mut scores = self.scores.write().unwrap();
        if let Some(pos) = scores.iter().position(|s| s.id == score.id) {
            scores[pos] = score;
        } else {
            if scores.len() >= MAX_RECORDS {
                scores.remove(0);
            }
            scores.push(score);
        }
    }

    pub fn get_scores_for_trace(&self, trace_id: &Uuid) -> Vec<Score> {
        self.scores
            .read()
            .unwrap()
            .iter()
            .filter(|s| s.trace_id == *trace_id)
            .cloned()
            .collect()
    }

    pub fn get_scores_by_name(&self, trace_id: &Uuid, name: &str) -> Vec<Score> {
        self.scores
            .read()
            .unwrap()
            .iter()
            .filter(|s| s.trace_id == *trace_id && s.name == name)
            .cloned()
            .collect()
    }

    // ─── Prompt Templates ─────────────────────────────────────────────────

    /// Insert or replace a prompt template (upsert by name+version).
    pub fn upsert_prompt_template(&self, tmpl: PromptTemplate) {
        let mut templates = self.prompt_templates.write().unwrap();
        if let Some(pos) = templates
            .iter()
            .position(|t| t.name == tmpl.name && t.version == tmpl.version)
        {
            templates[pos] = tmpl;
        } else {
            templates.push(tmpl);
        }
    }

    pub fn get_prompt_template(&self, name: &str, version: u32) -> Option<PromptTemplate> {
        self.prompt_templates
            .read()
            .unwrap()
            .iter()
            .find(|t| t.name == name && t.version == version)
            .cloned()
    }

    pub fn get_active_prompt(&self, name: &str) -> Option<PromptTemplate> {
        self.prompt_templates
            .read()
            .unwrap()
            .iter()
            .filter(|t| t.name == name && t.is_active)
            .max_by_key(|t| t.version)
            .cloned()
    }

    pub fn list_prompt_versions(&self, name: &str) -> Vec<PromptTemplate> {
        let mut versions: Vec<PromptTemplate> = self
            .prompt_templates
            .read()
            .unwrap()
            .iter()
            .filter(|t| t.name == name)
            .cloned()
            .collect();
        versions.sort_by_key(|t| t.version);
        versions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_models::{ScoreSource, TraceStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_trace(user_id: Option<&str>, session_id: Option<&str>) -> Trace {
        Trace {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            user_id: user_id.map(|s| s.to_string()),
            session_id: session_id.map(|s| s.to_string()),
            metadata: serde_json::Value::Null,
            input: None,
            output: None,
            status: TraceStatus::Success,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tags: vec![],
        }
    }

    #[test]
    fn test_trace_store_basic() {
        let store = TraceStore::new();
        let trace = make_trace(Some("alice"), Some("s1"));
        let id = trace.id;
        store.upsert_trace(trace);
        assert!(store.get_trace(&id).is_some());
        assert!(store.get_trace(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_list_by_session() {
        let store = TraceStore::new();
        store.upsert_trace(make_trace(None, Some("s1")));
        store.upsert_trace(make_trace(None, Some("s1")));
        store.upsert_trace(make_trace(None, Some("s2")));
        assert_eq!(store.list_traces(None, Some("s1"), None, 100).len(), 2);
        assert_eq!(store.list_traces(None, Some("s2"), None, 100).len(), 1);
    }

    #[test]
    fn test_session_summary_no_traces() {
        let store = TraceStore::new();
        assert!(store.get_session("nonexistent").is_none());
    }
}
