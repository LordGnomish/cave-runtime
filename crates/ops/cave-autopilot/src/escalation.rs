// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tiered LLM escalation ladder.
//!
//! The decision of *which tier handles the next code-gen attempt* is pure
//! policy and lives here, isolated from the clients that execute it:
//!
//! * **L1 — router** (Mellum2): analyses the task, decides which surface to
//!   port and how much context to feed L2. Cheap; runs once per task.
//! * **L2 — local coder** (Qwen3-Coder-Next): writes the failing test, then the
//!   implementation. Gets [`max_local_retries`](LadderState::max_local_retries)
//!   shots before we give up on local.
//! * **L3 — Claude API**: invoked only after local retries are spent, and only
//!   while the daily token budget has room.
//! * **L4 — human (Burak)**: architectural/strategic calls, or when every
//!   automated tier is exhausted.

/// Which executor handles the next attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    L1Router,
    L2Coder,
    L3Claude,
    L4Human,
}

impl Tier {
    /// Stable label for metrics / reports.
    pub fn label(&self) -> &'static str {
        match self {
            Tier::L1Router => "l1_router",
            Tier::L2Coder => "l2_coder",
            Tier::L3Claude => "l3_claude",
            Tier::L4Human => "l4_human",
        }
    }
}

/// Everything the ladder needs to pick a tier. Snapshotted per decision.
#[derive(Debug, Clone, Copy)]
pub struct LadderState {
    /// Local-LLM attempts already spent on this task.
    pub attempts: u32,
    pub max_local_retries: u32,
    pub claude_tokens_used_today: u64,
    pub claude_daily_budget: u64,
    /// `ANTHROPIC_API_KEY` present.
    pub claude_key_available: bool,
    /// Ollama reachable with a usable coding model.
    pub local_llm_available: bool,
    /// The router (or a prior tier) flagged this as needing a human decision.
    pub needs_architectural_decision: bool,
}

impl LadderState {
    /// Claude is usable: key present *and* budget not yet spent.
    pub fn claude_usable(&self) -> bool {
        self.claude_key_available && self.claude_tokens_used_today < self.claude_daily_budget
    }

    /// Daily Claude budget is spent — daemon should drop to local-only mode.
    pub fn claude_budget_exhausted(&self) -> bool {
        self.claude_tokens_used_today >= self.claude_daily_budget
    }

    /// Local coder still has retries left and is reachable.
    pub fn local_retries_remain(&self) -> bool {
        self.local_llm_available && self.attempts < self.max_local_retries
    }
}

/// Outcome of a ladder decision: the tier plus a human-readable reason that
/// flows into the daily report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscalationDecision {
    pub tier: Tier,
    pub reason: String,
}

/// Decide the tier for the next code-gen attempt. Order matters:
///
/// 1. explicit human-decision flag → L4
/// 2. local retries remain → L2
/// 3. Claude usable (key + budget) → L3
/// 4. nothing automated left → L4
pub fn decide(state: &LadderState) -> EscalationDecision {
    if state.needs_architectural_decision {
        return EscalationDecision {
            tier: Tier::L4Human,
            reason: "architectural/strategic decision required".to_string(),
        };
    }
    if state.local_retries_remain() {
        return EscalationDecision {
            tier: Tier::L2Coder,
            reason: format!(
                "local coder attempt {}/{}",
                state.attempts + 1,
                state.max_local_retries
            ),
        };
    }
    if state.claude_usable() {
        return EscalationDecision {
            tier: Tier::L3Claude,
            reason: format!(
                "local retries exhausted ({}); escalating to Claude (budget {}/{})",
                state.attempts, state.claude_tokens_used_today, state.claude_daily_budget
            ),
        };
    }
    // Nothing automated can make progress.
    let why = if !state.local_llm_available && !state.claude_key_available {
        "no LLM backend available (Ollama down, no Claude key)"
    } else if state.claude_budget_exhausted() {
        "local retries exhausted and Claude daily budget spent"
    } else {
        "all automated tiers exhausted"
    };
    EscalationDecision {
        tier: Tier::L4Human,
        reason: why.to_string(),
    }
}

/// The tier that should perform the initial routing/analysis pass for a fresh
/// task: L1 locally when Ollama is up, otherwise Claude can route, otherwise a
/// human must triage.
pub fn routing_tier(state: &LadderState) -> Tier {
    if state.local_llm_available {
        Tier::L1Router
    } else if state.claude_usable() {
        Tier::L3Claude
    } else {
        Tier::L4Human
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> LadderState {
        LadderState {
            attempts: 0,
            max_local_retries: 5,
            claude_tokens_used_today: 0,
            claude_daily_budget: 1_000_000,
            claude_key_available: true,
            local_llm_available: true,
            needs_architectural_decision: false,
        }
    }

    #[test]
    fn fresh_task_uses_local_coder() {
        assert_eq!(decide(&base()).tier, Tier::L2Coder);
    }

    #[test]
    fn exhausted_local_escalates_to_claude() {
        let mut s = base();
        s.attempts = 5;
        assert_eq!(decide(&s).tier, Tier::L3Claude);
    }

    #[test]
    fn architectural_flag_goes_straight_to_human() {
        let mut s = base();
        s.needs_architectural_decision = true;
        assert_eq!(decide(&s).tier, Tier::L4Human);
    }

    #[test]
    fn claude_over_budget_after_local_exhausted_goes_to_human() {
        let mut s = base();
        s.attempts = 5;
        s.claude_tokens_used_today = 1_000_000; // == budget
        assert!(s.claude_budget_exhausted());
        let d = decide(&s);
        assert_eq!(d.tier, Tier::L4Human);
        assert!(d.reason.contains("budget spent"));
    }

    #[test]
    fn no_backends_at_all_goes_to_human() {
        let mut s = base();
        s.local_llm_available = false;
        s.claude_key_available = false;
        assert_eq!(decide(&s).tier, Tier::L4Human);
    }

    #[test]
    fn local_down_but_claude_ok_escalates() {
        let mut s = base();
        s.local_llm_available = false;
        // No local retries possible -> Claude picks it up.
        assert_eq!(decide(&s).tier, Tier::L3Claude);
    }

    #[test]
    fn routing_prefers_local_then_claude_then_human() {
        let mut s = base();
        assert_eq!(routing_tier(&s), Tier::L1Router);
        s.local_llm_available = false;
        assert_eq!(routing_tier(&s), Tier::L3Claude);
        s.claude_key_available = false;
        assert_eq!(routing_tier(&s), Tier::L4Human);
    }
}
