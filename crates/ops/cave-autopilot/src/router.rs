// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! L1 router (Mellum2) decision layer.
//!
//! The router's job is cheap triage: given an under-complete subsystem, decide
//! *which upstream surface to port next*, *how big a context window* the coder
//! tier (L2) should run with, and whether the work is an architectural call
//! that must escalate straight to a human (L4) instead of being auto-coded.
//!
//! The prompt asks the local model for a strict JSON object; parsing is a pure,
//! fully unit-tested function so the routing contract is verifiable without a
//! live model. When the model omits or garbles the context size we fall back to
//! a [`default_context_tokens`] heuristic keyed on how far from parity the
//! subsystem is — the further behind, the larger the window.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_mentions_json_and_router_role() {
        let s = router_system_prompt();
        assert!(s.contains("JSON"));
        assert!(s.to_lowercase().contains("surface"));
    }

    #[test]
    fn build_prompt_carries_subsystem_and_upstream() {
        let p = build_router_prompt("cave-etcd", 0.52, Some("etcd-io/etcd"));
        assert!(p.contains("cave-etcd"));
        assert!(p.contains("etcd-io/etcd"));
        assert!(p.contains("0.52"));
    }

    #[test]
    fn default_context_grows_as_completion_falls() {
        // Closer to parity -> smaller window; far behind -> larger.
        let near = default_context_tokens(0.9);
        let far = default_context_tokens(0.3);
        assert!(far > near);
        // Always within the sane band.
        assert!((4096..=65536).contains(&near));
        assert!((4096..=65536).contains(&far));
    }

    #[test]
    fn parse_full_decision() {
        let raw = r#"Here is my plan:
        ```json
        {"surface":"raft leader election","context_tokens":32768,"needs_human":false,"rationale":"core data-plane gap"}
        ```"#;
        let d = parse_router_decision(raw, 0.5).unwrap();
        assert_eq!(d.surface, "raft leader election");
        assert_eq!(d.context_tokens, 32768);
        assert!(!d.needs_human);
        assert!(d.rationale.contains("data-plane"));
    }

    #[test]
    fn parse_clamps_absurd_context_and_defaults_missing() {
        // Over-large context is clamped to the ceiling.
        let big = parse_router_decision(
            r#"{"surface":"x","context_tokens":9999999,"needs_human":false,"rationale":"r"}"#,
            0.5,
        )
        .unwrap();
        assert_eq!(big.context_tokens, 65536);
        // Missing context_tokens -> heuristic from completion.
        let miss = parse_router_decision(
            r#"{"surface":"x","needs_human":false,"rationale":"r"}"#,
            0.3,
        )
        .unwrap();
        assert_eq!(miss.context_tokens, default_context_tokens(0.3));
    }

    #[test]
    fn parse_flags_human_escalation() {
        let d = parse_router_decision(
            r#"{"surface":"redesign storage engine","context_tokens":16384,"needs_human":true,"rationale":"arch call"}"#,
            0.5,
        )
        .unwrap();
        assert!(d.needs_human);
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_router_decision("the model rambled with no json", 0.5).is_err());
        // Empty surface is not a usable decision.
        assert!(parse_router_decision(
            r#"{"surface":"","needs_human":false,"rationale":"r"}"#,
            0.5
        )
        .is_err());
    }
}
