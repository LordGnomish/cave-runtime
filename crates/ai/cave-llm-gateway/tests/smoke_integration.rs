// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Three-prong smoke test. Always runs (no `#[ignore]`):
//!   1. Capability-router scoring is deterministic + ordered.
//!   2. Anthropic provider health probe completes (return value
//!      unchecked — Anthropic doesn't have a cheap ping endpoint so the
//!      probe is allowed to be optimistically true).
//!   3. Ollama health probe against `http://127.0.0.1:11434` is
//!      attempted; *no assertion* on the result so the test still
//!      passes on machines without Ollama.
//!
//! The "live chat" smoke is opt-in via `CAVE_LLM_GATEWAY_SMOKE_OLLAMA=1`
//! (gated so CI never tries to talk to a port that isn't there).

use cave_llm_gateway::capability::{seeded_router, CapabilityRequest, Locality};
use cave_llm_gateway::provider::{LlmProvider, ProviderConfig, ProviderType};
use cave_llm_gateway::providers::ollama::OllamaProvider;
use cave_llm_gateway::provider::AnthropicProvider;
use cave_llm_gateway::openai::{ChatCompletionRequest, ChatMessage};

#[tokio::test]
async fn smoke_capability_router_scoring_orders_consistently() {
    let r = seeded_router();
    // Cheap-but-good case: prefer local, no SaaS cost, must do tools.
    let req = CapabilityRequest {
        need_tools: true,
        preferred_locality: Some(Locality::Local),
        ..Default::default()
    };
    let ranked = r.rank(&req);
    assert!(!ranked.is_empty(), "router must return at least one pick");
    // Every pick supports tools (hard requirement).
    for s in &ranked {
        assert!(s.cap.supports_tools, "non-tool model leaked through filter");
    }
    // Top pick must be local (soft preference + locality bonus).
    let top = ranked.first().unwrap();
    assert_eq!(
        top.cap.locality,
        Locality::Local,
        "top pick must be Local when preferred_locality=Local; got {:?} ({})",
        top.cap.locality,
        top.cap.model
    );
    // Scores are monotonically non-increasing.
    let scores: Vec<_> = ranked.iter().map(|s| s.score).collect();
    for w in scores.windows(2) {
        assert!(w[0] >= w[1], "scores not non-increasing: {:?}", scores);
    }
}

#[tokio::test]
async fn smoke_anthropic_health_probe_does_not_panic() {
    let cfg = ProviderConfig {
        name: "anthropic-smoke".into(),
        provider_type: ProviderType::Anthropic,
        base_url: "https://api.anthropic.com".into(),
        // Probe path doesn't actually hit the API per the provider impl;
        // we still pass a stub key.
        api_key: Some("smoke-test".into()),
        timeout_secs: 5,
        max_retries: 0,
        weight: 1,
        enabled: true,
    };
    let p = AnthropicProvider::new(cfg);
    let _ = p.health_check().await; // result is allowed to be true or false
    // smoke: we only assert no panic on the path.
    assert_eq!(p.name(), "anthropic-smoke");
}

#[tokio::test]
async fn smoke_ollama_health_probe_attempt_does_not_panic() {
    let cfg = ProviderConfig {
        name: "ollama-smoke".into(),
        provider_type: ProviderType::Ollama,
        base_url: "http://127.0.0.1:11434".into(),
        api_key: None,
        timeout_secs: 2,
        max_retries: 0,
        weight: 1,
        enabled: true,
    };
    let p = OllamaProvider::new(cfg);
    let _ = p.health_check().await;
    assert_eq!(p.name(), "ollama-smoke");
}

/// Opt-in live chat against `http://127.0.0.1:11434`. Set
/// `CAVE_LLM_GATEWAY_SMOKE_OLLAMA=1` and ensure the box has the model
/// referenced below. Falls back to the first model returned by
/// `/api/tags` so the test works against arbitrary local catalogs.
#[tokio::test]
async fn smoke_ollama_chat_live() {
    if std::env::var("CAVE_LLM_GATEWAY_SMOKE_OLLAMA").ok().as_deref() != Some("1") {
        eprintln!("skipping live-ollama chat smoke (set CAVE_LLM_GATEWAY_SMOKE_OLLAMA=1)");
        return;
    }
    // Discover a model first.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let tags: serde_json::Value = client
        .get("http://127.0.0.1:11434/api/tags")
        .send()
        .await
        .expect("ollama /api/tags must be reachable when smoke is opt-in")
        .json()
        .await
        .expect("ollama /api/tags returns json");
    let model = tags["models"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|m| m["name"].as_str())
        .expect("at least one model must be available on the Ollama box")
        .to_string();

    let cfg = ProviderConfig {
        name: "ollama-smoke-live".into(),
        provider_type: ProviderType::Ollama,
        base_url: "http://127.0.0.1:11434".into(),
        api_key: None,
        timeout_secs: 180,
        max_retries: 0,
        weight: 1,
        enabled: true,
    };
    let p = OllamaProvider::new(cfg);
    let req = ChatCompletionRequest {
        model: model.clone(),
        messages: vec![ChatMessage::user(
            "Reply with exactly the three letters: ack",
        )],
        temperature: Some(0.0),
        top_p: None,
        max_tokens: Some(256),
        stream: Some(false),
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        n: None,
        user: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        seed: None,
        logprobs: None,
    };
    let resp = p.complete(&req).await.expect("live ollama chat must succeed");
    let text = resp
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.content.as_text())
        .unwrap_or("")
        .to_string();
    assert!(!text.is_empty(), "live ollama returned empty text");
    eprintln!("ollama live smoke: model={} reply={:?}", model, text);
    assert_eq!(resp.model, model);
}
