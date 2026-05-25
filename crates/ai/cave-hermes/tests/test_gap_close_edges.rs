// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gap-close edge-case sweep for cave-hermes.
//!
//! These tests probe boundaries the inline `#[cfg(test)]` blocks under
//! `src/` don't already cover: failure modes, serde round-trips, state
//! transitions, and numerical edges around the embedding / cosine
//! pipeline. Nothing here depends on cave-hermes internals beyond the
//! public API.
//!
//! Conventions:
//! * One `#[test]` per behaviour.
//! * No shared mutable globals (the `web_fetch` fetcher slot in
//!   `tools_builtin` is intentionally avoided here to side-step the
//!   inline-test mutex).
//! * Network-free.

use std::sync::Arc;

use cave_hermes::error::HermesError;
use cave_hermes::gateway::{
    AnthropicStubGateway, CompletionRequest, CompletionResponse, LlmGateway, OllamaGateway,
};
use cave_hermes::memory::{FileStore, InMemoryStore, MemoryProvider, MemoryRecord, SqliteStore};
use cave_hermes::planner::{HeuristicPlanner, LlmPlanner, Plan, PlanStep, Planner};
use cave_hermes::prompt::{
    AnthropicPrompt, OllamaPrompt, OpenAiPrompt, OpenRouterPrompt, PromptContext, ProviderKind,
    ProviderPrompt, ToolDescriptor,
};
use cave_hermes::recall::{
    Embedder, EmbeddingRecall, HashEmbedder, HashRecall, RecallEngine, cosine_similarity,
};
use cave_hermes::router::{ModelProfile, ModelRouter, ModelTier, TaskComplexity};
use cave_hermes::session::{Event, EventKind, SessionStore};
use cave_hermes::tool::{ToolEntry, ToolRegistry, ToolResult};
use cave_hermes::workflow::{Checkpoint, Step, Workflow, WorkflowStatus};

// ───────────────────────── prompt formatting edges ─────────────────────────

#[test]
fn anthropic_assemble_only_persona_no_tools_no_memory() {
    let cx = PromptContext::new("careful editor", "fix typos");
    let out = AnthropicPrompt::new().assemble(&cx).unwrap();
    assert!(out.contains("<persona>"));
    assert!(out.contains("careful editor"));
    assert!(!out.contains("<tools>"));
    assert!(!out.contains("<memory-context>"));
    assert!(out.contains("<task>"));
}

#[test]
fn anthropic_assemble_persona_whitespace_only_is_elided() {
    let cx = PromptContext::new("   \t\n  ", "do the thing");
    let out = AnthropicPrompt::new().assemble(&cx).unwrap();
    assert!(!out.contains("<persona>"));
    assert!(out.contains("<task>"));
}

#[test]
fn openai_assemble_emits_required_array_when_schema_has_required() {
    let cx = PromptContext::new("p", "t").with_tools(vec![ToolDescriptor {
        name: "x".into(),
        description: "d".into(),
        schema: serde_json::json!({"type": "object", "required": ["a", "b"]}),
    }]);
    let out = OpenAiPrompt::new().assemble(&cx).unwrap();
    assert!(out.contains("\"required\""));
    assert!(out.contains("\"a\""));
    assert!(out.contains("\"b\""));
}

#[test]
fn openrouter_passthrough_marker_is_only_at_position_zero() {
    let cx = PromptContext::new("p", "t");
    let out = OpenRouterPrompt::new().assemble(&cx).unwrap();
    let first = out.find("[openrouter-passthrough]").unwrap();
    assert_eq!(first, 0);
    // Marker must appear exactly once.
    assert_eq!(
        out.matches("[openrouter-passthrough]").count(),
        1,
        "marker repeated: {out}"
    );
}

#[test]
fn ollama_numbers_tools_sequentially_from_one() {
    let cx = PromptContext::new("p", "t").with_tools(vec![
        ToolDescriptor {
            name: "alpha".into(),
            description: "a".into(),
            schema: serde_json::json!({}),
        },
        ToolDescriptor {
            name: "beta".into(),
            description: "b".into(),
            schema: serde_json::json!({}),
        },
        ToolDescriptor {
            name: "gamma".into(),
            description: "c".into(),
            schema: serde_json::json!({}),
        },
    ]);
    let out = OllamaPrompt::new().assemble(&cx).unwrap();
    assert!(out.contains("1. alpha"));
    assert!(out.contains("2. beta"));
    assert!(out.contains("3. gamma"));
    // No 0-index or 4-index leaks.
    assert!(!out.contains("0. "));
    assert!(!out.contains("4. "));
}

#[test]
fn task_with_leading_and_trailing_whitespace_is_trimmed_in_body() {
    let cx = PromptContext::new("p", "   indented task   ");
    let out = OpenAiPrompt::new().assemble(&cx).unwrap();
    // Task: line should carry the trimmed form.
    assert!(out.contains("Task:\nindented task"));
    assert!(!out.contains("Task:\n   indented"));
}

#[test]
fn anthropic_tool_description_xml_escapes_ampersand() {
    let cx = PromptContext::new("p", "t").with_tools(vec![ToolDescriptor {
        name: "esc".into(),
        description: "uses & < > in description".into(),
        schema: serde_json::json!({}),
    }]);
    let out = AnthropicPrompt::new().assemble(&cx).unwrap();
    assert!(out.contains("uses &amp; &lt; &gt; in description"));
}

// ───────────────────────── memory SqliteStore CRUD ─────────────────────────

#[test]
fn sqlite_store_serde_record_roundtrip_preserves_all_fields() {
    let s = SqliteStore::in_memory().unwrap();
    let mut r = MemoryRecord::new("id-001", "scope-A", "the body with unicode: αβγ");
    r.created_at = "2026-05-20T00:00:00Z".into();
    s.put(r.clone()).unwrap();
    let got = s.get("id-001").unwrap().unwrap();
    assert_eq!(got.id, r.id);
    assert_eq!(got.scope, r.scope);
    assert_eq!(got.body, r.body);
    assert_eq!(got.created_at, r.created_at);
}

#[test]
fn sqlite_store_delete_missing_returns_false_not_error() {
    let s = SqliteStore::in_memory().unwrap();
    assert!(!s.delete("does-not-exist").unwrap());
    assert!(s.is_empty().unwrap());
}

#[test]
fn sqlite_store_get_missing_returns_none_not_error() {
    let s = SqliteStore::in_memory().unwrap();
    assert!(s.get("nope").unwrap().is_none());
}

#[test]
fn sqlite_store_list_scope_empty_when_scope_unknown() {
    let s = SqliteStore::in_memory().unwrap();
    s.put(MemoryRecord::new("k", "scope-known", "v")).unwrap();
    assert_eq!(s.list_scope("scope-unknown").unwrap().len(), 0);
    assert_eq!(s.list_scope("scope-known").unwrap().len(), 1);
}

#[test]
fn sqlite_store_upsert_preserves_count() {
    let s = SqliteStore::in_memory().unwrap();
    for _ in 0..10 {
        s.put(MemoryRecord::new("only-id", "s", "v")).unwrap();
    }
    assert_eq!(s.len().unwrap(), 1);
}

#[test]
fn sqlite_store_build_system_prompt_empty_scope_returns_empty_string() {
    let s = SqliteStore::in_memory().unwrap();
    s.put(MemoryRecord::new("k", "other", "x")).unwrap();
    assert_eq!(s.build_system_prompt("nope").unwrap(), "");
}

#[test]
fn file_store_handles_empty_file_as_empty_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.json");
    std::fs::write(&path, "").unwrap();
    let s = FileStore::open(&path).unwrap();
    assert!(s.is_empty().unwrap());
}

#[test]
fn in_memory_store_overwrite_preserves_singleton() {
    let s = InMemoryStore::new();
    s.put(MemoryRecord::new("k", "s", "v1")).unwrap();
    s.put(MemoryRecord::new("k", "s", "v2")).unwrap();
    assert_eq!(s.len().unwrap(), 1);
    assert_eq!(s.get("k").unwrap().unwrap().body, "v2");
}

// ─────────────────── EmbeddingRecall cosine + HashEmbedder ──────────────────

#[test]
fn hash_embedder_different_inputs_produce_different_vectors() {
    let e = HashEmbedder::new(128);
    let a = e.embed("rust tokio runtime");
    let b = e.embed("python flask servers");
    assert_ne!(a, b);
}

#[test]
fn hash_embedder_default_dim_is_128() {
    let e = HashEmbedder::default();
    assert_eq!(e.dim(), 128);
    assert_eq!(e.embed("hello").len(), 128);
}

#[test]
#[should_panic]
fn hash_embedder_zero_dim_panics_at_construction() {
    let _ = HashEmbedder::new(0);
}

#[test]
fn cosine_similarity_negative_correlated_returns_minus_one() {
    let a = vec![1.0f32, 0.0, 0.0];
    let b = vec![-1.0f32, 0.0, 0.0];
    let s = cosine_similarity(&a, &b);
    assert!((s + 1.0).abs() < 1e-6, "expected -1, got {s}");
}

#[test]
fn cosine_similarity_zero_vectors_returns_zero() {
    let a = vec![0.0f32, 0.0, 0.0];
    let b = vec![0.0f32, 0.0, 0.0];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

#[test]
fn embedding_recall_topk_truncates_to_k() {
    let r = EmbeddingRecall::with_hash_embedder();
    r.index(&[
        MemoryRecord::new("k1", "s", "rust tokio runtime alpha"),
        MemoryRecord::new("k2", "s", "rust tokio runtime beta"),
        MemoryRecord::new("k3", "s", "rust tokio runtime gamma"),
        MemoryRecord::new("k4", "s", "rust tokio runtime delta"),
    ])
    .unwrap();
    let hits = r.query("rust tokio", 2).unwrap();
    assert!(hits.len() <= 2);
}

#[test]
fn hash_recall_only_stopwords_yields_no_index_entry() {
    let r = HashRecall::new();
    // tokenise filters stopwords; body of pure stopwords ends empty.
    r.index(&[MemoryRecord::new("k", "s", "the a an and or but of")])
        .unwrap();
    assert_eq!(r.len().unwrap(), 0);
}

// ───────────────────────── gateway request shapes ─────────────────────────

#[test]
fn completion_request_serde_preserves_custom_max_tokens_and_stop() {
    let mut req = CompletionRequest::new("m", "sys", "user");
    req.max_tokens = 4096;
    req.stop = vec!["\n\n".into(), "STOP".into()];
    let raw = serde_json::to_string(&req).unwrap();
    let back: CompletionRequest = serde_json::from_str(&raw).unwrap();
    assert_eq!(back.max_tokens, 4096);
    assert_eq!(back.stop, vec!["\n\n".to_string(), "STOP".to_string()]);
}

#[test]
fn completion_request_deserialise_omitted_fields_get_defaults() {
    let raw = r#"{"model":"m","system":"s","user":"u"}"#;
    let back: CompletionRequest = serde_json::from_str(raw).unwrap();
    assert_eq!(back.max_tokens, 2048);
    assert!(back.stop.is_empty());
}

#[test]
fn completion_response_serde_preserves_provider_tag() {
    let resp = CompletionResponse {
        text: "hi".into(),
        provider: ProviderKind::Ollama,
        model: "qwen".into(),
        tokens: 3,
        latency_ms: 12,
    };
    let raw = serde_json::to_string(&resp).unwrap();
    let back: CompletionResponse = serde_json::from_str(&raw).unwrap();
    assert_eq!(back.provider, ProviderKind::Ollama);
    assert_eq!(back.tokens, 3);
    assert_eq!(back.latency_ms, 12);
}

#[tokio::test]
async fn anthropic_stub_default_is_echo_mode() {
    let gw = AnthropicStubGateway::default();
    let req = CompletionRequest::new("m", "sys", "what");
    let resp = gw.complete(&req).await.unwrap();
    assert!(resp.text.starts_with("[anthropic-stub] "));
    assert!(resp.text.contains("what"));
}

#[tokio::test]
async fn anthropic_stub_with_canned_overrides_user_message() {
    let gw = AnthropicStubGateway::with_canned("CANNED");
    let req_a = CompletionRequest::new("m", "s", "x");
    let req_b = CompletionRequest::new("m", "s", "y");
    let resp_a = gw.complete(&req_a).await.unwrap();
    let resp_b = gw.complete(&req_b).await.unwrap();
    assert_eq!(resp_a.text, "CANNED");
    assert_eq!(resp_b.text, "CANNED");
}

#[tokio::test]
async fn anthropic_stub_kind_is_anthropic() {
    let gw = AnthropicStubGateway::echo();
    assert_eq!(gw.kind(), ProviderKind::Anthropic);
}

#[test]
fn ollama_gateway_kind_is_ollama() {
    let g = OllamaGateway::localhost().unwrap();
    assert_eq!(g.kind(), ProviderKind::Ollama);
}

// ─────────────────────── workflow state transitions ────────────────────────

#[test]
fn workflow_start_after_complete_returns_checkpoint_missing() {
    let mut wf = Workflow::new("wf", vec![Step::new("only")]);
    wf.start_step().unwrap();
    wf.finish_step(None).unwrap();
    assert!(wf.is_complete());
    let err = wf.start_step().unwrap_err();
    assert!(matches!(err, HermesError::CheckpointMissing(_)));
}

#[test]
fn workflow_revision_monotone_under_mixed_ops() {
    let mut wf = Workflow::new(
        "wf",
        vec![Step::new("a"), Step::new("b"), Step::new("c")],
    );
    let r0 = wf.revision;
    wf.start_step().unwrap();
    wf.finish_step(None).unwrap();
    let r1 = wf.revision;
    wf.start_step().unwrap();
    wf.fail_step("boom").unwrap();
    let r2 = wf.revision;
    assert!(r1 > r0);
    assert!(r2 > r1);
}

#[test]
fn workflow_checkpoint_roundtrip_preserves_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let mut wf = Workflow::new("wf-x", vec![Step::new("only")]).with_max_retries(5);
    wf.start_step().unwrap();
    wf.fail_step("transient").unwrap();
    wf.start_step().unwrap();
    let saved_attempts = wf.steps[0].attempts;
    assert_eq!(saved_attempts, 2);
    wf.checkpoint().save(dir.path()).unwrap();
    let loaded = Checkpoint::load(dir.path(), "wf-x").unwrap();
    assert_eq!(loaded.data.steps[0].attempts, saved_attempts);
}

#[test]
fn workflow_unstick_resets_attempts_counter_to_zero() {
    let mut wf = Workflow::new("wf", vec![Step::new("only")]).with_max_retries(0);
    wf.start_step().unwrap();
    wf.fail_step("perm").unwrap();
    assert!(matches!(wf.steps[0].status, WorkflowStatus::Stuck(_)));
    assert_eq!(wf.steps[0].attempts, 1);
    wf.unstick().unwrap();
    assert_eq!(wf.steps[0].attempts, 0);
    assert_eq!(wf.steps[0].status, WorkflowStatus::Pending);
}

#[test]
fn workflow_status_serde_roundtrip_for_all_variants() {
    for s in [
        WorkflowStatus::Pending,
        WorkflowStatus::Running,
        WorkflowStatus::Done,
        WorkflowStatus::Failed("reason1".into()),
        WorkflowStatus::Stuck("reason2".into()),
    ] {
        let raw = serde_json::to_string(&s).unwrap();
        let back: WorkflowStatus = serde_json::from_str(&raw).unwrap();
        assert_eq!(s, back);
    }
}

// ─────────────────────── router edge transitions ───────────────────────────

#[test]
fn router_route_degrades_top_then_mid_then_local() {
    let r = ModelRouter::tiered_default();
    r.mark_throttled("claude-opus-4-7");
    r.mark_throttled("claude-sonnet-4-6");
    let complex = "Build a Raft state machine.\n```rust\nfn foo() {}\n```";
    let d = r.route(complex).unwrap();
    assert_eq!(d.model.tier, ModelTier::Local);
    assert_eq!(d.complexity, TaskComplexity::Complex);
}

#[test]
fn router_with_only_top_tier_returns_top_for_trivial() {
    let mut r = ModelRouter::new();
    r.register(ModelProfile::new("only-top", "p", ModelTier::Top, 100));
    // Trivial → wants Local; Local is empty, Mid empty, falls back to Top? No —
    // degradation only goes downward. Trivial → Local → empty → error.
    let err = r.route("hi").unwrap_err();
    assert!(matches!(err, HermesError::RouterEmpty(_)));
}

#[test]
fn router_complex_with_only_local_succeeds_via_degradation() {
    let mut r = ModelRouter::new();
    r.register(ModelProfile::new("only-local", "p", ModelTier::Local, 100));
    let d = r
        .route("Write a fn that does X.\n```rust\nfn foo() {}\n```")
        .unwrap();
    assert_eq!(d.model.tier, ModelTier::Local);
    assert_eq!(d.complexity, TaskComplexity::Complex);
}

#[test]
fn model_tier_ordering_is_local_lt_mid_lt_top() {
    assert!(ModelTier::Local < ModelTier::Mid);
    assert!(ModelTier::Mid < ModelTier::Top);
}

#[test]
fn task_complexity_estimate_long_prose_is_complex() {
    // ~450 words of plain text, no code → over the 400-token threshold.
    let blob: String = std::iter::repeat_n("word", 450)
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(TaskComplexity::estimate(&blob), TaskComplexity::Complex);
}

// ─────────────────────── planner / plan edges ──────────────────────────────

#[test]
fn plan_step_with_arg_chains_multiple_args() {
    let s = PlanStep::new("bash", "r")
        .with_arg("a", serde_json::json!(1))
        .with_arg("b", serde_json::json!("two"));
    assert_eq!(s.args.len(), 2);
    assert_eq!(s.args.get("a").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(s.args.get("b").and_then(|v| v.as_str()), Some("two"));
}

#[test]
fn heuristic_planner_quoted_path_after_file_keyword() {
    let p = HeuristicPlanner::new()
        .plan("please read file \"/tmp/with space.md\"")
        .unwrap();
    assert_eq!(p.steps[0].tool, "file_read");
    // The implementation strips at the matching quote.
    assert_eq!(
        p.steps[0].args.get("path").and_then(|v| v.as_str()),
        Some("/tmp/with space.md")
    );
}

#[test]
fn llm_planner_rejects_invalid_json() {
    let p = LlmPlanner::new(Arc::new(|_| Ok("this is not json".into())));
    let err = p.plan("g").unwrap_err();
    match err {
        HermesError::PlannerRejected(reason) => assert!(reason.contains("invalid plan JSON")),
        e => panic!("expected PlannerRejected, got {e}"),
    }
}

// ─────────────────────── tool registry behaviour ───────────────────────────

#[test]
fn tool_registry_iter_returns_alphabetical_order() {
    let mut r = ToolRegistry::new();
    for name in ["mango", "apple", "banana"] {
        r.register(ToolEntry::new(
            name,
            "core",
            "",
            serde_json::json!({}),
            Arc::new(|_| Ok(ToolResult::ok("x"))),
        ));
    }
    let order: Vec<_> = r.iter().map(|t| t.name.clone()).collect();
    assert_eq!(order, vec!["apple", "banana", "mango"]);
}

#[test]
fn tool_result_with_meta_chains_multiple_keys() {
    let r = ToolResult::ok("body")
        .with_meta("a", "1")
        .with_meta("b", "2");
    assert_eq!(r.meta.get("a").map(String::as_str), Some("1"));
    assert_eq!(r.meta.get("b").map(String::as_str), Some("2"));
    assert!(r.ok);
}

#[test]
fn tool_result_serde_roundtrip_preserves_meta() {
    let r = ToolResult::ok("hi").with_meta("k", "v");
    let raw = serde_json::to_string(&r).unwrap();
    let back: ToolResult = serde_json::from_str(&raw).unwrap();
    assert!(back.ok);
    assert_eq!(back.output, "hi");
    assert_eq!(back.meta.get("k").map(String::as_str), Some("v"));
}

// ─────────────────────── session log edges ─────────────────────────────────

#[test]
fn session_event_serde_roundtrip_preserves_kind_and_payload() {
    let e = Event::new(EventKind::ToolCall, serde_json::json!({"k": 1}));
    let raw = serde_json::to_string(&e).unwrap();
    let back: Event = serde_json::from_str(&raw).unwrap();
    assert_eq!(back.kind, EventKind::ToolCall);
    assert_eq!(back.payload, serde_json::json!({"k": 1}));
    assert_eq!(back.id, e.id);
}

#[test]
fn session_replay_skips_blank_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log.jsonl");
    let ev = Event::new(EventKind::UserTurn, serde_json::json!({"x": 1}));
    let body = format!("\n\n{}\n\n", serde_json::to_string(&ev).unwrap());
    std::fs::write(&path, body).unwrap();
    let restored = SessionStore::replay(&path).unwrap();
    assert_eq!(restored.len(), 1);
}

#[test]
fn session_of_kind_returns_empty_when_none_match() {
    let s = SessionStore::new();
    s.append(Event::new(EventKind::UserTurn, serde_json::json!({})))
        .unwrap();
    assert!(s.of_kind(EventKind::Error).is_empty());
    assert!(s.of_kind(EventKind::Checkpoint).is_empty());
}

#[test]
fn event_kind_serde_roundtrip_for_all_variants() {
    for k in [
        EventKind::UserTurn,
        EventKind::AssistantTurn,
        EventKind::ToolCall,
        EventKind::ToolResult,
        EventKind::PlanCreated,
        EventKind::Checkpoint,
        EventKind::Recall,
        EventKind::Error,
    ] {
        let raw = serde_json::to_string(&k).unwrap();
        let back: EventKind = serde_json::from_str(&raw).unwrap();
        assert_eq!(k, back);
    }
}

// ─────────────────────── plan serde + error display ────────────────────────

#[test]
fn hermes_error_display_includes_tool_name() {
    let e = HermesError::ToolNotFound {
        name: "ghost".into(),
    };
    let s = format!("{e}");
    assert!(s.contains("ghost"));
}

#[test]
fn plan_with_multiple_steps_serde_roundtrip() {
    let plan = Plan::new("multi")
        .push(PlanStep::new("a", "r1").with_arg("x", serde_json::json!(true)))
        .push(PlanStep::new("b", "r2"));
    let raw = serde_json::to_string(&plan).unwrap();
    let back: Plan = serde_json::from_str(&raw).unwrap();
    assert_eq!(back.steps.len(), 2);
    assert_eq!(back, plan);
}
