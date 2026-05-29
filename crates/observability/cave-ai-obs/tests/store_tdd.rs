// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD: AiObsStore latency percentile computation.

use cave_ai_obs::models::{LlmProvider, LlmRequest, RequestStatus};
use cave_ai_obs::store::AiObsStore;
use chrono::Utc;
use uuid::Uuid;

fn req(latency_ms: u64) -> LlmRequest {
    LlmRequest {
        id: Uuid::new_v4(),
        provider: LlmProvider::OpenAi,
        model: "gpt-4o".to_string(),
        user_id: Some("u1".to_string()),
        status: RequestStatus::Success,
        prompt_tokens: 10,
        completion_tokens: 20,
        cost_usd: 0.001,
        latency_ms,
        created_at: Utc::now(),
    }
}

#[test]
fn store_percentiles_p95() {
    let store = AiObsStore::new();
    // Append latencies 1..=100 ms.
    for ms in 1..=100u64 {
        store.append(req(ms));
    }

    let stats = store.compute_latency_stats();

    // sorted latencies = [1, 2, ..., 100]; index (100 * 0.95) = 95 -> value 96.
    assert_eq!(stats.p95_ms, 96, "expected 95th percentile = 96");
    // sanity on the other percentiles for the known distribution.
    assert_eq!(stats.p50_ms, 51, "expected 50th percentile = 51");
    assert_eq!(stats.p99_ms, 100, "expected 99th percentile = 100");
    assert_eq!(stats.max_ms, 100);
}
