// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plan-and-execute loop — OpenJarvis composable on-device pattern.
//!
//! Glues the existing [`crate::planner::Plan`] decomposition to the
//! [`crate::tool::ToolRegistry`]: each [`crate::planner::PlanStep`] is
//! dispatched to its named tool, and the per-step outcomes are recorded in
//! an [`ExecutionTrace`]. The reserved `respond` pseudo-tool is not a real
//! registry entry — it records the step rationale as the agent's final
//! natural-language response, exactly as the heuristic planner emits it.
//!
//! Guard rails: a `max_steps` ceiling (runaway-plan backstop) and a
//! `stop_on_error` switch (halt the loop at the first failing tool, or push
//! through and let the trace report partial failure).

use serde::{Deserialize, Serialize};

use crate::planner::Plan;
use crate::tool::ToolRegistry;

/// The reserved pseudo-tool name a plan uses for "answer the user directly".
pub const RESPOND_TOOL: &str = "respond";

/// Result of one executed [`crate::planner::PlanStep`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutcome {
    pub tool: String,
    pub ok: bool,
    pub output: String,
}

/// Record of a whole plan execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub goal: String,
    pub steps: Vec<StepOutcome>,
    /// True when every planned step was attempted (not truncated by the
    /// step ceiling or an early stop-on-error halt).
    pub completed: bool,
}

impl ExecutionTrace {
    /// True when the plan completed and every step's tool succeeded.
    pub fn succeeded(&self) -> bool {
        self.completed && self.steps.iter().all(|s| s.ok)
    }

    /// The last `respond` step's output — the agent's final answer.
    pub fn final_response(&self) -> Option<&str> {
        self.steps
            .iter()
            .rev()
            .find(|s| s.tool == RESPOND_TOOL && s.ok)
            .map(|s| s.output.as_str())
    }
}

/// Drives a [`Plan`] against a [`ToolRegistry`].
#[derive(Debug, Clone)]
pub struct PlanExecutor {
    max_steps: usize,
    stop_on_error: bool,
}

impl Default for PlanExecutor {
    fn default() -> Self {
        Self {
            max_steps: 32,
            stop_on_error: true,
        }
    }
}

impl PlanExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_steps(mut self, n: usize) -> Self {
        self.max_steps = n;
        self
    }

    pub fn stop_on_error(mut self, stop: bool) -> Self {
        self.stop_on_error = stop;
        self
    }

    /// Execute every step in order. The `respond` pseudo-tool records its
    /// rationale as output; all other steps dispatch to the registry.
    pub fn execute(&self, plan: &Plan, tools: &ToolRegistry) -> ExecutionTrace {
        let mut steps = Vec::new();
        let mut completed = true;
        for (i, step) in plan.steps.iter().enumerate() {
            if i >= self.max_steps {
                completed = false;
                break;
            }
            let outcome = if step.tool == RESPOND_TOOL {
                StepOutcome {
                    tool: RESPOND_TOOL.to_string(),
                    ok: true,
                    output: step.rationale.clone(),
                }
            } else {
                let args = serde_json::Value::Object(step.args.clone());
                match tools.invoke(&step.tool, &args) {
                    Ok(res) => StepOutcome {
                        tool: step.tool.clone(),
                        ok: res.ok,
                        output: res.output,
                    },
                    Err(e) => StepOutcome {
                        tool: step.tool.clone(),
                        ok: false,
                        output: e.to_string(),
                    },
                }
            };
            let failed = !outcome.ok;
            steps.push(outcome);
            if failed && self.stop_on_error {
                completed = false;
                break;
            }
        }
        ExecutionTrace {
            goal: plan.goal.clone(),
            steps,
            completed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{Plan, PlanStep};
    use crate::tool::{ToolEntry, ToolRegistry, ToolResult};
    use std::sync::Arc;

    fn registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(ToolEntry::new(
            "echo",
            "core",
            "echo args back",
            serde_json::json!({}),
            Arc::new(|args: &serde_json::Value| Ok(ToolResult::ok(args.to_string()))),
        ));
        r
    }

    #[test]
    fn executes_tool_then_respond() {
        let plan = Plan::new("g")
            .push(PlanStep::new("echo", "echo it").with_arg("x", serde_json::json!(1)))
            .push(PlanStep::new("respond", "here is your answer"));
        let trace = PlanExecutor::new().execute(&plan, &registry());
        assert_eq!(trace.steps.len(), 2);
        assert!(trace.completed);
        assert!(trace.succeeded());
        assert_eq!(trace.final_response(), Some("here is your answer"));
    }

    #[test]
    fn unknown_tool_fails_and_stops_by_default() {
        let plan = Plan::new("g")
            .push(PlanStep::new("nope", "missing tool"))
            .push(PlanStep::new("respond", "unreached"));
        let trace = PlanExecutor::new().execute(&plan, &registry());
        assert_eq!(trace.steps.len(), 1, "loop halts at the failing step");
        assert!(!trace.steps[0].ok);
        assert!(!trace.completed);
        assert!(!trace.succeeded());
    }

    #[test]
    fn stop_on_error_false_pushes_through_failure() {
        let plan = Plan::new("g")
            .push(PlanStep::new("nope", "missing tool"))
            .push(PlanStep::new("respond", "still respond"));
        let trace = PlanExecutor::new()
            .stop_on_error(false)
            .execute(&plan, &registry());
        assert_eq!(trace.steps.len(), 2);
        assert!(trace.completed);
        assert!(!trace.succeeded(), "one step failed");
        assert_eq!(trace.final_response(), Some("still respond"));
    }

    #[test]
    fn max_steps_truncates_runaway_plan() {
        let mut plan = Plan::new("g");
        for _ in 0..10 {
            plan = plan.push(PlanStep::new("echo", "loop"));
        }
        let trace = PlanExecutor::new().max_steps(3).execute(&plan, &registry());
        assert_eq!(trace.steps.len(), 3);
        assert!(!trace.completed, "hit the step ceiling before finishing");
    }

    #[test]
    fn empty_plan_completes_trivially() {
        let trace = PlanExecutor::new().execute(&Plan::new("g"), &registry());
        assert!(trace.steps.is_empty());
        assert!(trace.completed);
        assert!(trace.succeeded());
        assert!(trace.final_response().is_none());
    }

    #[test]
    fn respond_step_records_rationale_as_output() {
        let plan = Plan::new("g").push(PlanStep::new("respond", "the sky is blue"));
        let trace = PlanExecutor::new().execute(&plan, &registry());
        assert_eq!(trace.steps[0].tool, "respond");
        assert_eq!(trace.steps[0].output, "the sky is blue");
        assert!(trace.steps[0].ok);
    }
}
