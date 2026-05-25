// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Task decomposition.
//!
//! Hermes' planner lives in `agent/prompt_builder.py` mixed with
//! per-provider system-prompt assembly (~1.5 kLOC). Most of that is
//! provider-specific glue that doesn't survive the port; what does is
//! the decomposition pattern:
//!
//!   user goal → ordered [`PlanStep`] list → workflow.
//!
//! Two backends ship:
//!
//! * [`HeuristicPlanner`] — keyword-based router. Returns deterministic
//!   plans without needing a model. Good enough for built-in tasks
//!   ("read file X", "fetch URL Y", "run command Z") and for tests.
//! * [`LlmPlanner`] — thin wrapper that delegates to a caller-supplied
//!   completion fn. The fn is expected to return JSON conforming to
//!   [`Plan`]'s serde shape; failures bubble up as `HermesError::PlannerRejected`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub tool: String,
    pub rationale: String,
    #[serde(default)]
    pub args: serde_json::Map<String, serde_json::Value>,
}

impl PlanStep {
    pub fn new(tool: impl Into<String>, rationale: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            rationale: rationale.into(),
            args: serde_json::Map::new(),
        }
    }

    pub fn with_arg(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.args.insert(key.into(), value);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub steps: Vec<PlanStep>,
}

impl Plan {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            steps: Vec::new(),
        }
    }

    pub fn push(mut self, step: PlanStep) -> Self {
        self.steps.push(step);
        self
    }
}

pub trait Planner: Send + Sync {
    fn plan(&self, goal: &str) -> crate::error::Result<Plan>;
}

/// Keyword-heuristic planner. The "model" is a hand-written decision
/// tree — sufficient to cover the four built-in tools without depending
/// on an LLM. Used by [`crate::default_runtime`].
#[derive(Default)]
pub struct HeuristicPlanner;

impl HeuristicPlanner {
    pub fn new() -> Self {
        Self
    }
}

impl Planner for HeuristicPlanner {
    fn plan(&self, goal: &str) -> crate::error::Result<Plan> {
        let trimmed = goal.trim();
        if trimmed.is_empty() {
            return Err(HermesError::PlannerRejected("empty goal".into()));
        }
        let lower = trimmed.to_lowercase();
        let mut plan = Plan::new(trimmed);

        // URL → web_fetch
        if let Some(url) = pick_url(trimmed) {
            plan.steps.push(
                PlanStep::new("web_fetch", "fetch URL referenced in the goal")
                    .with_arg("url", serde_json::Value::String(url)),
            );
            plan.steps.push(PlanStep::new(
                "respond",
                "summarise the fetched body for the user",
            ));
            return Ok(plan);
        }

        // "read file X" → file_read
        if lower.contains("read")
            && lower.contains("file")
            && let Some(path) = pick_path_after(&lower, "file")
        {
            plan.steps.push(
                PlanStep::new("file_read", "read referenced file")
                    .with_arg("path", serde_json::Value::String(path)),
            );
            plan.steps
                .push(PlanStep::new("respond", "summarise the file contents"));
            return Ok(plan);
        }

        // "write to X" → file_write (deferred — needs both path and content)
        // "run X" / "execute X" → bash
        if lower.starts_with("run ") || lower.starts_with("execute ") {
            // Strip the verb and run the rest as a shell command.
            let cmd = trimmed
                .split_once(' ')
                .map(|(_, rest)| rest)
                .unwrap_or(trimmed);
            plan.steps.push(
                PlanStep::new("bash", "execute requested shell command")
                    .with_arg("command", serde_json::Value::String(cmd.to_string())),
            );
            plan.steps
                .push(PlanStep::new("respond", "summarise the command output"));
            return Ok(plan);
        }

        // Default: a single 'respond' step (no tools).
        plan.steps.push(PlanStep::new(
            "respond",
            "answer the user directly using model knowledge",
        ));
        Ok(plan)
    }
}

fn pick_url(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let needle = lower.find("http://").or_else(|| lower.find("https://"))?;
    let tail = &text[needle..];
    let end = tail
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '>')
        .unwrap_or(tail.len());
    Some(tail[..end].to_string())
}

fn pick_path_after(lower: &str, keyword: &str) -> Option<String> {
    let i = lower.find(keyword)?;
    let after = lower[i + keyword.len()..].trim_start();
    let trimmed = after.trim_start_matches([':', '=', ' ']);
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    let path: String = if first == '"' || first == '\'' {
        chars.take_while(|c| *c != first).collect()
    } else {
        trimmed.chars().take_while(|c| !c.is_whitespace()).collect()
    };
    if path.is_empty() { None } else { Some(path) }
}

/// Thin wrapper around a caller-supplied completion fn. The fn is given
/// the user goal and must return a JSON string parseable into [`Plan`].
pub type LlmCompleteFn = Arc<dyn Fn(&str) -> std::result::Result<String, String> + Send + Sync>;

pub struct LlmPlanner {
    complete: LlmCompleteFn,
}

impl LlmPlanner {
    pub fn new(complete: LlmCompleteFn) -> Self {
        Self { complete }
    }
}

impl Planner for LlmPlanner {
    fn plan(&self, goal: &str) -> crate::error::Result<Plan> {
        let raw = (self.complete)(goal).map_err(HermesError::PlannerRejected)?;
        let plan: Plan = serde_json::from_str(&raw)
            .map_err(|e| HermesError::PlannerRejected(format!("invalid plan JSON: {e}")))?;
        if plan.steps.is_empty() {
            return Err(HermesError::PlannerRejected(
                "planner returned zero steps".into(),
            ));
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_url_goal_emits_web_fetch_then_respond() {
        let p = HeuristicPlanner::new()
            .plan("look at https://example.com/foo")
            .unwrap();
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[0].tool, "web_fetch");
        assert_eq!(
            p.steps[0].args.get("url").unwrap().as_str(),
            Some("https://example.com/foo")
        );
        assert_eq!(p.steps[1].tool, "respond");
    }

    #[test]
    fn heuristic_read_file_emits_file_read() {
        let p = HeuristicPlanner::new()
            .plan("please read file /tmp/notes.md")
            .unwrap();
        assert_eq!(p.steps[0].tool, "file_read");
        assert_eq!(
            p.steps[0].args.get("path").unwrap().as_str(),
            Some("/tmp/notes.md")
        );
    }

    #[test]
    fn heuristic_run_command_emits_bash() {
        let p = HeuristicPlanner::new().plan("run ls -la /etc").unwrap();
        assert_eq!(p.steps[0].tool, "bash");
        assert_eq!(
            p.steps[0].args.get("command").unwrap().as_str(),
            Some("ls -la /etc")
        );
    }

    #[test]
    fn heuristic_unknown_goal_falls_back_to_respond() {
        let p = HeuristicPlanner::new().plan("tell me a joke").unwrap();
        assert_eq!(p.steps.len(), 1);
        assert_eq!(p.steps[0].tool, "respond");
    }

    #[test]
    fn heuristic_empty_goal_rejected() {
        let err = HeuristicPlanner::new().plan("   ").unwrap_err();
        assert!(matches!(err, HermesError::PlannerRejected(_)));
    }

    #[test]
    fn llm_planner_parses_valid_json() {
        let p = LlmPlanner::new(Arc::new(|_goal: &str| {
            Ok(r#"{"goal":"g","steps":[{"tool":"bash","rationale":"r","args":{}}]}"#.into())
        }));
        let plan = p.plan("anything").unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].tool, "bash");
    }

    #[test]
    fn llm_planner_rejects_zero_step_plan() {
        let p = LlmPlanner::new(Arc::new(|_g: &str| Ok(r#"{"goal":"g","steps":[]}"#.into())));
        let err = p.plan("g").unwrap_err();
        assert!(matches!(err, HermesError::PlannerRejected(_)));
    }

    #[test]
    fn llm_planner_propagates_completion_failure() {
        let p = LlmPlanner::new(Arc::new(|_g: &str| Err("model down".into())));
        let err = p.plan("g").unwrap_err();
        match err {
            HermesError::PlannerRejected(reason) => assert!(reason.contains("model down")),
            e => panic!("expected PlannerRejected, got {e}"),
        }
    }

    #[test]
    fn plan_serde_roundtrip() {
        let plan = Plan::new("goal").push(PlanStep::new("bash", "r"));
        let raw = serde_json::to_string(&plan).unwrap();
        let back: Plan = serde_json::from_str(&raw).unwrap();
        assert_eq!(plan, back);
    }
}
