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
