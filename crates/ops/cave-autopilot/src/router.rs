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

use crate::codegen::extract_json_block;
use crate::error::{AutopilotError, Result};
use serde::Deserialize;

/// Lower / upper bounds for the coder's context window (tokens).
const CTX_MIN: u32 = 4096;
const CTX_MAX: u32 = 65536;

/// The router's structured verdict for one subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterDecision {
    /// Which upstream surface to port next (one cohesive, well-scoped chunk).
    pub surface: String,
    /// Context window the L2 coder should run with.
    pub context_tokens: u32,
    /// True when this is an architectural/strategic call → escalate to L4 human
    /// rather than auto-coding it.
    pub needs_human: bool,
    /// Short justification, surfaced in the daily report.
    pub rationale: String,
}

/// Raw shape the model is asked to emit. `context_tokens` is optional so we can
/// fall back to the completion-keyed heuristic when the model omits it.
#[derive(Debug, Deserialize)]
struct RawDecision {
    #[serde(default)]
    surface: String,
    #[serde(default)]
    context_tokens: Option<u32>,
    #[serde(default)]
    needs_human: bool,
    #[serde(default)]
    rationale: String,
}

/// System prompt pinning the router to a terse JSON contract.
pub fn router_system_prompt() -> &'static str {
    "You are the routing tier of an autonomous Rust porting pipeline. \
     Given an under-complete subsystem, choose the single highest-value upstream \
     surface to port next. Reply with ONLY a JSON object: \
     {\"surface\":\"<short name>\",\"context_tokens\":<int>,\"needs_human\":<bool>,\"rationale\":\"<one line>\"}. \
     Set needs_human=true only for architectural/strategic decisions that should \
     not be auto-coded. Pick context_tokens proportional to the surface size."
}

/// Build the per-task routing prompt.
pub fn build_router_prompt(subsystem: &str, completion: f64, upstream: Option<&str>) -> String {
    let up = upstream.unwrap_or("unknown");
    format!(
        "Subsystem `{subsystem}` is {completion:.2} complete (upstream {up}). \
         Pick the next surface to port and estimate the context window the coder needs."
    )
}

/// Heuristic context window keyed on distance-from-parity: linearly interpolate
/// between [`CTX_MIN`] (at/above parity) and [`CTX_MAX`] (greenfield). Further
/// behind ⇒ larger window.
pub fn default_context_tokens(completion: f64) -> u32 {
    let gap = (1.0 - completion).clamp(0.0, 1.0);
    let span = (CTX_MAX - CTX_MIN) as f64;
    let raw = CTX_MIN as f64 + gap * span;
    (raw.round() as u32).clamp(CTX_MIN, CTX_MAX)
}

/// Parse the model's routing output into a [`RouterDecision`]. `completion`
/// drives the context fallback when the model omits `context_tokens`.
pub fn parse_router_decision(raw: &str, completion: f64) -> Result<RouterDecision> {
    let json = extract_json_block(raw)
        .ok_or_else(|| AutopilotError::Llm("no JSON router decision found".into()))?;
    let r: RawDecision = serde_json::from_str(&json)
        .map_err(|e| AutopilotError::Llm(format!("router decision parse: {e}")))?;
    if r.surface.trim().is_empty() {
        return Err(AutopilotError::Llm("router decision has empty surface".into()));
    }
    let context_tokens = r
        .context_tokens
        .map(|t| t.clamp(CTX_MIN, CTX_MAX))
        .unwrap_or_else(|| default_context_tokens(completion));
    Ok(RouterDecision {
        surface: r.surface.trim().to_string(),
        context_tokens,
        needs_human: r.needs_human,
        rationale: r.rationale.trim().to_string(),
    })
}

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
