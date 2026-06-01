// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plan-and-execute loop — the core agentic control flow. A [`Planner`]
//! decomposes a natural-language goal into ordered [`Step`]s; an [`Executor`]
//! drives each step's bound tool through a state machine, retrying transient
//! failures and either skipping or aborting once retries are exhausted.
//!
//! OpenJarvis upstream: `jarvis/agent/plan_execute.py`. The LLM-backed planner
//! that turns a goal into a tool-bound DAG is scope-cut (no live model in this
//! crate); the heuristic connector-split planner ports the offline fallback.

use crate::error::AgentError;
use crate::tool::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Lifecycle of a single plan step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    /// Not yet attempted.
    Pending,
    /// Currently executing (set transiently during a run).
    Running,
    /// Completed successfully.
    Done,
    /// Exhausted its retries (or was unresolved).
    Failed,
}

/// One unit of a plan: a description, an optional bound tool + arguments, and
/// the runtime bookkeeping the executor maintains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// 0-based position in the plan.
    pub id: usize,
    /// Human-readable description (from decomposition).
    pub description: String,
    /// The tool to invoke, if bound. Unbound steps are treated as unresolved.
    pub tool: Option<String>,
    /// Arguments passed to the tool.
    pub args: Value,
    /// Current lifecycle state.
    pub state: StepState,
    /// Number of handler invocations attempted (including the successful one).
    pub attempts: u32,
    /// The tool's output, once the step is [`StepState::Done`].
    pub output: Option<Value>,
}

impl Step {
    /// A pending, unbound step.
    pub fn new(id: usize, description: impl Into<String>) -> Self {
        Self {
            id,
            description: description.into(),
            tool: None,
            args: Value::Null,
            state: StepState::Pending,
            attempts: 0,
            output: None,
        }
    }

    /// Bind a tool and arguments to this step (builder style).
    pub fn with_tool(mut self, tool: impl Into<String>, args: Value) -> Self {
        self.tool = Some(tool.into());
        self.args = args;
        self
    }
}

/// An ordered set of steps working toward a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// The original goal text.
    pub goal: String,
    /// The ordered steps.
    pub steps: Vec<Step>,
}

impl Plan {
    /// Steps still awaiting execution.
    pub fn pending(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|s| s.state == StepState::Pending)
    }
}

/// Turns goals into plans.
pub struct Planner;

impl Planner {
    /// Decompose a goal by splitting on the ordering connectors `then`,
    /// `and then`, and comma. Each fragment becomes a pending, unbound step.
    pub fn decompose(goal: &str) -> Plan {
        let normalized = goal.replace(" and then ", " then ");
        let fragments: Vec<String> = normalized
            .split(" then ")
            .flat_map(|seg| seg.split(','))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let frags = if fragments.is_empty() {
            vec![goal.trim().to_string()]
        } else {
            fragments
        };
        let steps = frags
            .into_iter()
            .enumerate()
            .map(|(id, description)| Step::new(id, description))
            .collect();
        Plan {
            goal: goal.to_string(),
            steps,
        }
    }
}

/// What to do once a step's retries are exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnExhausted {
    /// Mark the step failed and continue to the next step.
    Skip,
    /// Mark the step failed and halt the whole plan.
    Abort,
}

/// Retry / failure handling for an execution run.
#[derive(Debug, Clone, Copy)]
pub struct ReplanPolicy {
    /// Number of *retries* after the first attempt (so total attempts =
    /// `max_retries + 1`).
    pub max_retries: u32,
    /// What to do when a step still fails after all retries.
    pub on_exhausted: OnExhausted,
}

impl Default for ReplanPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            on_exhausted: OnExhausted::Skip,
        }
    }
}

/// The aggregate result of running a plan.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Outcome {
    /// Steps that reached [`StepState::Done`].
    pub completed: usize,
    /// Steps that failed and were *not* skipped past (i.e. caused an abort).
    pub failed: usize,
    /// Steps that failed but the run continued (Skip policy / unresolved).
    pub skipped: usize,
    /// Whether the run halted early under [`OnExhausted::Abort`].
    pub aborted: bool,
}

/// Drives plans to completion.
pub struct Executor;

impl Executor {
    /// Run every pending step in order. Each step's bound tool is invoked up to
    /// `max_retries + 1` times; on persistent failure the policy decides
    /// whether to skip or abort. Unbound steps are unresolved → treated as a
    /// skip-or-abort per policy.
    pub fn run(plan: &mut Plan, registry: &ToolRegistry, policy: &ReplanPolicy) -> Outcome {
        let mut outcome = Outcome::default();
        for step in &mut plan.steps {
            if step.state != StepState::Pending {
                continue;
            }
            step.state = StepState::Running;
            let result = Self::run_step(step, registry, policy);
            match result {
                Ok(output) => {
                    step.state = StepState::Done;
                    step.output = Some(output);
                    outcome.completed += 1;
                }
                Err(_) => {
                    step.state = StepState::Failed;
                    match policy.on_exhausted {
                        OnExhausted::Skip => outcome.skipped += 1,
                        OnExhausted::Abort => {
                            outcome.failed += 1;
                            outcome.aborted = true;
                            break;
                        }
                    }
                }
            }
        }
        outcome
    }

    /// Execute one step with retries. Returns the tool output or the last error.
    fn run_step(
        step: &mut Step,
        registry: &ToolRegistry,
        policy: &ReplanPolicy,
    ) -> Result<Value, AgentError> {
        let Some(tool) = step.tool.clone() else {
            return Err(AgentError::Plan(format!(
                "step {} has no bound tool",
                step.id
            )));
        };
        let mut last_err = AgentError::Plan("never attempted".into());
        for _ in 0..=policy.max_retries {
            step.attempts += 1;
            match registry.invoke(&tool, &step.args) {
                Ok(v) => return Ok(v),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_trims_whitespace() {
        let p = Planner::decompose("  a then  b  ");
        assert_eq!(p.steps[0].description, "a");
        assert_eq!(p.steps[1].description, "b");
    }

    #[test]
    fn pending_iterator_filters_done() {
        let mut p = Planner::decompose("a then b");
        p.steps[0].state = StepState::Done;
        assert_eq!(p.pending().count(), 1);
    }
}
