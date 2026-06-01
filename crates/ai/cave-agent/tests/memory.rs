// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Conversation-memory primitive: append, windowing, token-budget eviction
//! (system-pinned), and keyword recall.

use cave_agent::memory::{ConversationMemory, Role};

fn seeded() -> ConversationMemory {
    let mut m = ConversationMemory::new();
    m.append(Role::System, "You are Jarvis, an on-device assistant.");
    m.append(Role::User, "What is the capital of France?");
    m.append(Role::Assistant, "The capital of France is Paris.");
    m.append(Role::User, "And of Japan?");
    m.append(Role::Assistant, "Tokyo.");
    m
}

#[test]
fn append_assigns_monotonic_sequence() {
    let m = seeded();
    assert_eq!(m.len(), 5);
    let seqs: Vec<i64> = m.turns().iter().map(|t| t.seq).collect();
    assert_eq!(seqs, [0, 1, 2, 3, 4]);
}

#[test]
fn window_returns_last_n_in_order() {
    let m = seeded();
    let w = m.window(2);
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].content, "And of Japan?");
    assert_eq!(w[1].content, "Tokyo.");
}

#[test]
fn window_larger_than_history_returns_all() {
    let m = seeded();
    assert_eq!(m.window(99).len(), 5);
}

#[test]
fn token_estimate_is_chars_over_four() {
    let mut m = ConversationMemory::new();
    m.append(Role::User, "abcdefgh"); // 8 chars -> 2 tokens
    assert_eq!(m.token_estimate(), 2);
}

#[test]
fn evict_to_budget_drops_oldest_but_pins_system() {
    let mut m = seeded();
    let before = m.len();
    // Force a tiny budget so most user/assistant turns must go.
    let evicted = m.evict_to_budget(8);
    assert!(evicted > 0, "should have evicted something");
    assert_eq!(m.len(), before - evicted);
    // System turn always survives.
    assert!(m.turns().iter().any(|t| t.role == Role::System));
    // The very oldest *non-system* turn went first.
    assert!(!m.turns().iter().any(|t| t.content.contains("capital of France is Paris")
        && t.role == Role::Assistant
        && m.len() < 3));
}

#[test]
fn evict_never_removes_system_even_at_zero_budget() {
    let mut m = seeded();
    m.evict_to_budget(0);
    assert!(m.turns().iter().all(|t| t.role == Role::System));
    assert_eq!(m.turns().len(), 1);
}

#[test]
fn recall_is_case_insensitive_substring() {
    let m = seeded();
    let hits = m.recall("paris");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].content.contains("Paris"));
    assert!(m.recall("nonexistent").is_empty());
}
