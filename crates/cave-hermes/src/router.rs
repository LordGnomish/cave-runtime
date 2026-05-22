// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Model router.
//!
//! Ports the routing half of `providers/base.py` + `agent/rate_limit_tracker.py`.
//! The upstream `ProviderProfile` dataclass is huge (catalogs, auth quirks,
//! per-provider message munging, ...); we keep only the *routing*
//! decision here:
//!
//!   task complexity + per-tier rate-limit state → tier + model id.
//!
//! Anything that needs an HTTP client (catalog probes, OAuth device
//! flow, etc.) lives in cave-llm-gateway downstream. This module owns
//! the policy.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

/// Task-complexity bucket. Maps onto a model tier via [`ModelRouter::route`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskComplexity {
    Trivial,
    Standard,
    Complex,
}

impl TaskComplexity {
    /// Approximate complexity from the user prompt length + heuristics.
    /// Used when the caller has no out-of-band hint.
    pub fn estimate(prompt: &str) -> Self {
        let tokens_approx = prompt.split_whitespace().count();
        let multiline = prompt.lines().count() > 4;
        let has_code = prompt.contains("```")
            || prompt.contains("fn ")
            || prompt.contains("def ")
            || prompt.contains("class ");
        if tokens_approx < 30 && !has_code && !multiline {
            TaskComplexity::Trivial
        } else if tokens_approx > 400 || has_code {
            TaskComplexity::Complex
        } else {
            TaskComplexity::Standard
        }
    }
}

/// Model tier. Mirrors the cave-runtime convention: Qwen / Sonnet / Opus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum ModelTier {
    Local,
    Mid,
    Top,
}

impl std::fmt::Display for ModelTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelTier::Local => f.write_str("local"),
            ModelTier::Mid => f.write_str("mid"),
            ModelTier::Top => f.write_str("top"),
        }
    }
}

/// One registered model. Owns the routing-relevant fields only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    pub provider: String,
    pub tier: ModelTier,
    /// Soft request-per-minute cap. The router applies a slack window
    /// of half this value when deciding whether a tier is "burning".
    pub rpm_cap: u32,
}

impl ModelProfile {
    pub fn new(
        id: impl Into<String>,
        provider: impl Into<String>,
        tier: ModelTier,
        rpm_cap: u32,
    ) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            tier,
            rpm_cap,
        }
    }
}

/// What the router decided. The caller hands this to the LLM gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub model: ModelProfile,
    pub complexity: TaskComplexity,
    pub reason: String,
}

/// Rolling rate-limit counter. Tracks request timestamps in a fixed
/// window so the router can back off proactively.
#[derive(Debug, Default, Clone)]
struct RateWindow {
    samples: Vec<Instant>,
}

impl RateWindow {
    fn record(&mut self, now: Instant) {
        self.samples.push(now);
        self.evict(now);
    }

    fn evict(&mut self, now: Instant) {
        let cutoff = now - Duration::from_secs(60);
        self.samples.retain(|t| *t >= cutoff);
    }

    fn count(&mut self, now: Instant) -> usize {
        self.evict(now);
        self.samples.len()
    }
}

/// Routing engine. Stores model profiles keyed by tier and an in-memory
/// rate-limit counter per model id. `route` returns a [`RouteDecision`]
/// or a [`HermesError::RouterEmpty`] when the requested tier has no
/// registered models.
pub struct ModelRouter {
    by_tier: BTreeMap<ModelTier, Vec<ModelProfile>>,
    counters: parking_lot::Mutex<BTreeMap<String, RateWindow>>,
}

impl ModelRouter {
    pub fn new() -> Self {
        Self {
            by_tier: BTreeMap::new(),
            counters: parking_lot::Mutex::new(BTreeMap::new()),
        }
    }

    /// Bootstrap the router with cave-runtime's default tiers:
    /// * Local — `qwen3.6:35b-a3b-coding-mxfp8` via Ollama
    /// * Mid   — `claude-sonnet-4-6` via Anthropic
    /// * Top   — `claude-opus-4-7` via Anthropic
    pub fn tiered_default() -> Self {
        let mut r = Self::new();
        r.register(ModelProfile::new(
            "qwen3.6:35b-a3b-coding-mxfp8",
            "ollama",
            ModelTier::Local,
            600,
        ));
        r.register(ModelProfile::new(
            "claude-sonnet-4-6",
            "anthropic",
            ModelTier::Mid,
            120,
        ));
        r.register(ModelProfile::new(
            "claude-opus-4-7",
            "anthropic",
            ModelTier::Top,
            60,
        ));
        r
    }

    pub fn register(&mut self, m: ModelProfile) {
        self.by_tier.entry(m.tier).or_default().push(m);
    }

    pub fn tier_models(&self, tier: ModelTier) -> &[ModelProfile] {
        self.by_tier.get(&tier).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Total model count across every tier.
    pub fn len(&self) -> usize {
        self.by_tier.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pick a model for `prompt`. Tier is chosen from estimated
    /// complexity, then we pick the first model in the tier whose
    /// rate-limit window has slack. If the desired tier is saturated,
    /// we degrade one tier down (Top → Mid → Local).
    pub fn route(&self, prompt: &str) -> crate::error::Result<RouteDecision> {
        let complexity = TaskComplexity::estimate(prompt);
        let mut desired = match complexity {
            TaskComplexity::Trivial => ModelTier::Local,
            TaskComplexity::Standard => ModelTier::Mid,
            TaskComplexity::Complex => ModelTier::Top,
        };
        loop {
            if let Some(pick) = self.pick_in_tier(desired) {
                return Ok(RouteDecision {
                    model: pick.0,
                    complexity,
                    reason: pick.1,
                });
            }
            // Degrade or error.
            desired = match desired {
                ModelTier::Top => ModelTier::Mid,
                ModelTier::Mid => ModelTier::Local,
                ModelTier::Local => {
                    return Err(HermesError::RouterEmpty(complexity_to_tier(complexity)));
                }
            };
        }
    }

    fn pick_in_tier(&self, tier: ModelTier) -> Option<(ModelProfile, String)> {
        let candidates = self.by_tier.get(&tier)?;
        if candidates.is_empty() {
            return None;
        }
        let mut counters = self.counters.lock();
        let now = Instant::now();
        for m in candidates {
            let win = counters.entry(m.id.clone()).or_default();
            if win.count(now) < (m.rpm_cap / 2) as usize {
                win.record(now);
                let reason = format!(
                    "tier {} ok ({}rpm slack)",
                    tier,
                    (m.rpm_cap / 2) - win.count(now) as u32
                );
                return Some((m.clone(), reason));
            }
        }
        None
    }

    /// Mark a model as having hit a 429 / rate limit. Inflates its
    /// counter to its cap so the router will degrade until the window
    /// rolls.
    pub fn mark_throttled(&self, model_id: &str) {
        let mut counters = self.counters.lock();
        let win = counters.entry(model_id.to_string()).or_default();
        let now = Instant::now();
        for _ in 0..1024 {
            win.samples.push(now);
        }
    }
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
    }
}

fn complexity_to_tier(c: TaskComplexity) -> ModelTier {
    match c {
        TaskComplexity::Trivial => ModelTier::Local,
        TaskComplexity::Standard => ModelTier::Mid,
        TaskComplexity::Complex => ModelTier::Top,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_buckets_match_intuition() {
        assert_eq!(TaskComplexity::estimate("hi"), TaskComplexity::Trivial);
        assert_eq!(
            TaskComplexity::estimate("Please write a fn that does X. fn foo() { let bar = 1; }"),
            TaskComplexity::Complex,
            "code blocks should promote to Complex"
        );
        // Mid-length narrative (no code, single line) → Standard.
        let medium = "Tell me about the history of compilers and how the field evolved from early FORTRAN compilers in the late nineteen-fifties through the LALR family popularised by Yacc and the GLR variants that handle ambiguous grammars; cover the rise of LLVM and its impact on backend portability, but explain it as prose without showing code or pseudocode at all please.";
        assert_eq!(TaskComplexity::estimate(medium), TaskComplexity::Standard);
    }

    #[test]
    fn tiered_default_has_three_models() {
        let r = ModelRouter::tiered_default();
        assert_eq!(r.len(), 3);
        assert!(!r.tier_models(ModelTier::Local).is_empty());
        assert!(!r.tier_models(ModelTier::Mid).is_empty());
        assert!(!r.tier_models(ModelTier::Top).is_empty());
    }

    #[test]
    fn route_trivial_picks_local_tier() {
        let r = ModelRouter::tiered_default();
        let d = r.route("hi").unwrap();
        assert_eq!(d.model.tier, ModelTier::Local);
        assert_eq!(d.complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn route_complex_picks_top_tier() {
        let r = ModelRouter::tiered_default();
        let d = r
            .route("Write a Rust trait + 3 implementations.\n```rust\nfn foo() {}\n```")
            .unwrap();
        assert_eq!(d.model.tier, ModelTier::Top);
    }

    #[test]
    fn route_degrades_when_top_is_throttled() {
        let r = ModelRouter::tiered_default();
        r.mark_throttled("claude-opus-4-7");
        let d = r
            .route("Build a Raft state machine.\n```rust\nfn foo() {}\n```")
            .unwrap();
        assert_ne!(d.model.tier, ModelTier::Top);
    }

    #[test]
    fn route_errors_when_router_empty() {
        let r = ModelRouter::new();
        let err = r.route("anything").unwrap_err();
        assert!(matches!(err, HermesError::RouterEmpty(_)));
    }

    #[test]
    fn register_groups_by_tier() {
        let mut r = ModelRouter::new();
        r.register(ModelProfile::new("a", "p", ModelTier::Local, 100));
        r.register(ModelProfile::new("b", "p", ModelTier::Local, 100));
        assert_eq!(r.tier_models(ModelTier::Local).len(), 2);
    }
}
