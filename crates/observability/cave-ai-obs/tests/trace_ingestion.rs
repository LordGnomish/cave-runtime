// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for trace/span/generation ingestion core.

use cave_ai_obs::trace_store::{TraceStore, TraceStatus};
use cave_ai_obs::trace_models::{Trace, Span, Generation, Score, PromptTemplate};
use chrono::Utc;
use uuid::Uuid;

// ─── Trace ingestion ───────────────────────────────────────────────────────

#[test]
fn test_create_trace_roundtrip() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    let trace = Trace {
        id: trace_id,
        name: "chat-completion".to_string(),
        user_id: Some("user-123".to_string()),
        session_id: Some("sess-abc".to_string()),
        metadata: serde_json::json!({"env": "prod"}),
        input: Some(serde_json::json!({"prompt": "Hello"})),
        output: Some(serde_json::json!({"reply": "Hi!"})),
        status: TraceStatus::Success,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tags: vec!["prod".to_string()],
    };
    store.upsert_trace(trace.clone());
    let retrieved = store.get_trace(&trace_id).expect("trace must exist");
    assert_eq!(retrieved.id, trace_id);
    assert_eq!(retrieved.name, "chat-completion");
    assert_eq!(retrieved.user_id.as_deref(), Some("user-123"));
}

#[test]
fn test_trace_overwrite_upsert() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    let mut trace = Trace {
        id: trace_id,
        name: "original".to_string(),
        user_id: None,
        session_id: None,
        metadata: serde_json::Value::Null,
        input: None,
        output: None,
        status: TraceStatus::Success,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tags: vec![],
    };
    store.upsert_trace(trace.clone());
    trace.name = "updated".to_string();
    store.upsert_trace(trace.clone());
    let retrieved = store.get_trace(&trace_id).unwrap();
    assert_eq!(retrieved.name, "updated");
    // should not duplicate
    assert_eq!(store.list_traces(None, None, None, 100).len(), 1);
}

#[test]
fn test_trace_missing_returns_none() {
    let store = TraceStore::new();
    assert!(store.get_trace(&Uuid::new_v4()).is_none());
}

// ─── Span tracking ────────────────────────────────────────────────────────

#[test]
fn test_create_and_get_span() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    let span = Span {
        id: Uuid::new_v4(),
        trace_id,
        parent_span_id: None,
        name: "retrieval".to_string(),
        start_time: Utc::now(),
        end_time: None,
        input: Some(serde_json::json!({"query": "foo"})),
        output: None,
        metadata: serde_json::Value::Null,
        latency_ms: None,
    };
    store.upsert_span(span.clone());
    let spans = store.get_spans_for_trace(&trace_id);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "retrieval");
}

#[test]
fn test_span_parent_child() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    let parent_id = Uuid::new_v4();
    let parent = Span {
        id: parent_id,
        trace_id,
        parent_span_id: None,
        name: "parent".to_string(),
        start_time: Utc::now(),
        end_time: None,
        input: None,
        output: None,
        metadata: serde_json::Value::Null,
        latency_ms: None,
    };
    let child = Span {
        id: Uuid::new_v4(),
        trace_id,
        parent_span_id: Some(parent_id),
        name: "child".to_string(),
        start_time: Utc::now(),
        end_time: None,
        input: None,
        output: None,
        metadata: serde_json::Value::Null,
        latency_ms: None,
    };
    store.upsert_span(parent);
    store.upsert_span(child);
    let spans = store.get_spans_for_trace(&trace_id);
    assert_eq!(spans.len(), 2);
    let child_span = spans.iter().find(|s| s.parent_span_id.is_some()).unwrap();
    assert_eq!(child_span.parent_span_id, Some(parent_id));
}

// ─── Generation tracking ──────────────────────────────────────────────────

#[test]
fn test_create_generation() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    let generation = Generation {
        id: Uuid::new_v4(),
        trace_id,
        parent_span_id: None,
        name: "llm-call".to_string(),
        model: "gpt-4o".to_string(),
        model_parameters: serde_json::json!({"temperature": 0.7, "max_tokens": 512}),
        prompt_tokens: 120,
        completion_tokens: 80,
        total_tokens: 200,
        input: serde_json::json!([{"role": "user", "content": "Hello"}]),
        output: serde_json::json!({"role": "assistant", "content": "Hi!"}),
        cost_usd: 0.0042,
        latency_ms: 350,
        start_time: Utc::now(),
        end_time: None,
        metadata: serde_json::Value::Null,
    };
    store.upsert_generation(generation.clone());
    let gens = store.get_generations_for_trace(&trace_id);
    assert_eq!(gens.len(), 1);
    assert_eq!(gens[0].model, "gpt-4o");
    assert_eq!(gens[0].total_tokens, 200);
}

#[test]
fn test_generation_cost_accuracy() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    for _ in 0..3 {
        let generation = Generation {
            id: Uuid::new_v4(),
            trace_id,
            parent_span_id: None,
            name: "call".to_string(),
            model: "claude-3-sonnet".to_string(),
            model_parameters: serde_json::Value::Null,
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            input: serde_json::Value::Null,
            output: serde_json::Value::Null,
            cost_usd: 0.01,
            latency_ms: 200,
            start_time: Utc::now(),
            end_time: None,
            metadata: serde_json::Value::Null,
        };
        store.upsert_generation(generation);
    }
    let gens = store.get_generations_for_trace(&trace_id);
    let total_cost: f64 = gens.iter().map(|g| g.cost_usd).sum();
    assert!((total_cost - 0.03).abs() < 1e-9);
}

// ─── Score ingestion ──────────────────────────────────────────────────────

#[test]
fn test_create_score() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    let score = Score {
        id: Uuid::new_v4(),
        trace_id,
        generation_id: None,
        name: "quality".to_string(),
        value: 0.92,
        comment: Some("very relevant".to_string()),
        source: cave_ai_obs::trace_models::ScoreSource::Human,
        created_at: Utc::now(),
    };
    store.upsert_score(score.clone());
    let scores = store.get_scores_for_trace(&trace_id);
    assert_eq!(scores.len(), 1);
    assert!((scores[0].value - 0.92).abs() < 1e-9);
}

#[test]
fn test_score_filtering_by_name() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    for name in &["quality", "relevance", "quality"] {
        store.upsert_score(Score {
            id: Uuid::new_v4(),
            trace_id,
            generation_id: None,
            name: name.to_string(),
            value: 0.8,
            comment: None,
            source: cave_ai_obs::trace_models::ScoreSource::Model,
            created_at: Utc::now(),
        });
    }
    let quality_scores = store.get_scores_by_name(&trace_id, "quality");
    assert_eq!(quality_scores.len(), 2);
}

// ─── Session grouping ─────────────────────────────────────────────────────

#[test]
fn test_session_groups_traces() {
    let store = TraceStore::new();
    let session_id = "sess-xyz".to_string();
    for _ in 0..3 {
        store.upsert_trace(Trace {
            id: Uuid::new_v4(),
            name: "turn".to_string(),
            user_id: None,
            session_id: Some(session_id.clone()),
            metadata: serde_json::Value::Null,
            input: None,
            output: None,
            status: TraceStatus::Success,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tags: vec![],
        });
    }
    let session = store.get_session(&session_id).expect("session exists");
    assert_eq!(session.trace_count, 3);
    assert_eq!(session.session_id, session_id);
}

#[test]
fn test_list_traces_filtered_by_user() {
    let store = TraceStore::new();
    for uid in &["alice", "alice", "bob"] {
        store.upsert_trace(Trace {
            id: Uuid::new_v4(),
            name: "query".to_string(),
            user_id: Some(uid.to_string()),
            session_id: None,
            metadata: serde_json::Value::Null,
            input: None,
            output: None,
            status: TraceStatus::Success,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tags: vec![],
        });
    }
    let alice_traces = store.list_traces(Some("alice"), None, None, 100);
    assert_eq!(alice_traces.len(), 2);
}

// ─── Prompt template management ───────────────────────────────────────────

#[test]
fn test_prompt_template_crud() {
    let store = TraceStore::new();
    let tmpl = PromptTemplate {
        id: Uuid::new_v4(),
        name: "system-prompt-v1".to_string(),
        version: 1,
        content: "You are a helpful assistant. Context: {{context}}".to_string(),
        variables: vec!["context".to_string()],
        is_active: true,
        created_at: Utc::now(),
    };
    store.upsert_prompt_template(tmpl.clone());
    let retrieved = store.get_prompt_template("system-prompt-v1", 1).expect("must exist");
    assert_eq!(retrieved.content, tmpl.content);
    assert_eq!(retrieved.variables, vec!["context".to_string()]);
}

#[test]
fn test_prompt_template_versioning() {
    let store = TraceStore::new();
    for version in 1u32..=3 {
        store.upsert_prompt_template(PromptTemplate {
            id: Uuid::new_v4(),
            name: "my-prompt".to_string(),
            version,
            content: format!("Version {} content", version),
            variables: vec![],
            is_active: version == 3,
            created_at: Utc::now(),
        });
    }
    let versions = store.list_prompt_versions("my-prompt");
    assert_eq!(versions.len(), 3);
    let active = store.get_active_prompt("my-prompt").expect("active version");
    assert_eq!(active.version, 3);
}

#[test]
fn test_prompt_template_variable_render() {
    let tmpl = PromptTemplate {
        id: Uuid::new_v4(),
        name: "greet".to_string(),
        version: 1,
        content: "Hello, {{name}}! You are in {{city}}.".to_string(),
        variables: vec!["name".to_string(), "city".to_string()],
        is_active: true,
        created_at: Utc::now(),
    };
    let mut vars = std::collections::HashMap::new();
    vars.insert("name".to_string(), "Alice".to_string());
    vars.insert("city".to_string(), "Berlin".to_string());
    let rendered = cave_ai_obs::prompt::render_template(&tmpl, &vars).expect("renders");
    assert_eq!(rendered, "Hello, Alice! You are in Berlin.");
}

#[test]
fn test_prompt_render_missing_variable_error() {
    let tmpl = PromptTemplate {
        id: Uuid::new_v4(),
        name: "greet".to_string(),
        version: 1,
        content: "Hello, {{name}}!".to_string(),
        variables: vec!["name".to_string()],
        is_active: true,
        created_at: Utc::now(),
    };
    let vars = std::collections::HashMap::new(); // missing "name"
    let result = cave_ai_obs::prompt::render_template(&tmpl, &vars);
    assert!(result.is_err());
}
