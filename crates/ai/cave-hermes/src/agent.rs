// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Agent executor loop.
//!
//! Ports the core run loop of Hermes' `agent/run_agent.py`. Upstream
//! tangles the loop with credential discovery, streaming UI, and skill
//! preprocessing; we extract the *think → act → observe* spine and wire
//! it to the subsystems this crate already ships:
//!
//! ```text
//!   goal ──▶ planner.plan ──▶ Plan(steps)
//!                               │
//!            ┌──────────────────┴───────────────────┐
//!            ▼ for each step                         │
//!     tool == "respond"?  ── yes ─▶ synthesise final response
//!            │ no                                    │
//!            ▼                                       │
//!     tools.invoke(tool, args) ─▶ ToolResult ───────┤
//!            │                                       │
//!            ├─ persist output to MemoryProvider     │
//!            ├─ index output into RecallEngine       │
//!            └─ journal ToolCall / ToolResult        │
//!                                                    ▼
//!                                          AgentRun { steps, final_response }
//! ```
//!
//! Every transition is appended to the [`SessionStore`] event log so a
//! run is fully replayable. A tool that errors (unknown tool, bad
//! arguments, I/O failure) aborts the loop with [`HermesError::AgentFailed`]
//! after journalling an [`EventKind::Error`]; a tool that merely reports
//! `ok == false` (e.g. a non-zero shell exit) does *not* abort — that
//! outcome is data, and the loop carries it forward exactly as upstream
//! does.
//!
//! The loop is deterministic and synchronous: the planner and tool
//! registry own all I/O policy, so the executor needs no async runtime.

use serde_json::{Value, json};

use crate::HermesRuntime;
use crate::error::{HermesError, Result};
use crate::memory::{MemoryProvider, MemoryRecord};
use crate::recall::RecallHit;
use crate::session::{Event, EventKind, SessionStore};

/// The pseudo-tool that terminates a plan by emitting the final answer.
/// Mirrors upstream's `respond` sentinel — it is never a registered tool.
const RESPOND: &str = "respond";

/// Outcome of a single executed plan step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepOutcome {
    /// Tool name (or [`RESPOND`] for the terminal answer step).
    pub tool: String,
    /// Whether the step produced a successful result. For tool steps this
    /// is the underlying [`crate::tool::ToolResult::ok`]; the `respond`
    /// step is always `true`.
    pub ok: bool,
    /// Textual output: tool output, or the synthesised final answer.
    pub output: String,
}

/// The result of running an agent loop to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRun {
    pub goal: String,
    pub steps: Vec<StepOutcome>,
    pub final_response: String,
}

/// Drives a [`HermesRuntime`] through a plan-execute-observe loop.
///
/// Owns the runtime so memory / recall / session state accumulates across
/// successive [`run`](AgentExecutor::run) calls — the same agent instance
/// remembers what earlier turns produced.
pub struct AgentExecutor {
    runtime: HermesRuntime,
    /// Memory + session scope all records produced by this executor are
    /// filed under. Defaults to `"default"`.
    scope: String,
    /// Upper bound on plan steps the loop will execute before bailing with
    /// [`HermesError::AgentFailed`]. Guards against runaway plans.
    max_steps: usize,
    /// Monotonic counter feeding unique memory-record ids.
    step_seq: u64,
}

impl AgentExecutor {
    /// Wrap a runtime with default scope (`"default"`) and a 32-step budget.
    pub fn new(runtime: HermesRuntime) -> Self {
        Self {
            runtime,
            scope: "default".to_string(),
            max_steps: 32,
            step_seq: 0,
        }
    }

    /// Override the memory / session scope.
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Override the per-run step budget.
    pub fn with_max_steps(mut self, n: usize) -> Self {
        self.max_steps = n;
        self
    }

    /// Borrow the underlying runtime (read-only).
    pub fn runtime(&self) -> &HermesRuntime {
        &self.runtime
    }

    /// Borrow the session event log.
    pub fn session(&self) -> &SessionStore {
        &self.runtime.session
    }

    /// Borrow the memory provider.
    pub fn memory(&self) -> &dyn MemoryProvider {
        self.runtime.memory.as_ref()
    }

    /// Query the recall index seeded by prior tool outputs.
    pub fn recall_query(&self, query: &str, k: usize) -> Result<Vec<RecallHit>> {
        self.runtime.recall.query(query, k)
    }

    /// Run the loop for `goal`, returning the per-step trace and final
    /// response. State (memory, recall, session) is retained on `self`.
    pub fn run(&mut self, goal: &str) -> Result<AgentRun> {
        let plan = self.runtime.planner.plan(goal)?;
        self.runtime.session.append(Event::new(
            EventKind::PlanCreated,
            json!({ "goal": goal, "steps": plan.steps.len() }),
        ))?;

        let mut steps = Vec::with_capacity(plan.steps.len());
        let mut last_output = String::new();
        let mut final_response = String::new();

        for (i, pstep) in plan.steps.iter().enumerate() {
            if i + 1 > self.max_steps {
                return Err(HermesError::AgentFailed(format!(
                    "step budget exhausted (max_steps={})",
                    self.max_steps
                )));
            }

            if pstep.tool == RESPOND {
                let text = if last_output.is_empty() {
                    pstep.rationale.clone()
                } else {
                    last_output.clone()
                };
                self.runtime.session.append(Event::new(
                    EventKind::AssistantTurn,
                    json!({ "text": text }),
                ))?;
                steps.push(StepOutcome {
                    tool: RESPOND.to_string(),
                    ok: true,
                    output: text.clone(),
                });
                final_response = text;
                continue;
            }

            let args = Value::Object(pstep.args.clone());
            self.runtime.session.append(Event::new(
                EventKind::ToolCall,
                json!({ "tool": pstep.tool, "args": args.clone() }),
            ))?;

            match self.runtime.tools.invoke(&pstep.tool, &args) {
                Ok(tr) => {
                    self.runtime.session.append(Event::new(
                        EventKind::ToolResult,
                        json!({ "tool": pstep.tool, "ok": tr.ok, "output": tr.output }),
                    ))?;
                    self.step_seq += 1;
                    let rec = MemoryRecord::new(
                        format!("{}:{}:{}", self.scope, self.step_seq, pstep.tool),
                        &self.scope,
                        tr.output.clone(),
                    );
                    self.runtime.memory.put(rec.clone())?;
                    self.runtime.recall.index(std::slice::from_ref(&rec))?;
                    last_output = tr.output.clone();
                    steps.push(StepOutcome {
                        tool: pstep.tool.clone(),
                        ok: tr.ok,
                        output: tr.output,
                    });
                }
                Err(e) => {
                    self.runtime.session.append(Event::new(
                        EventKind::Error,
                        json!({ "tool": pstep.tool, "error": e.to_string() }),
                    ))?;
                    return Err(HermesError::AgentFailed(format!(
                        "tool '{}' failed: {e}",
                        pstep.tool
                    )));
                }
            }
        }

        if final_response.is_empty() {
            final_response = last_output;
        }

        Ok(AgentRun {
            goal: goal.to_string(),
            steps,
            final_response,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_runtime;

    #[test]
    fn new_executor_defaults_to_default_scope_and_budget() {
        let exec = AgentExecutor::new(default_runtime());
        assert_eq!(exec.scope, "default");
        assert_eq!(exec.max_steps, 32);
        assert!(exec.session().is_empty());
    }

    #[test]
    fn non_zero_shell_exit_does_not_abort_loop() {
        // `exit 7` returns ok=false but is *data*, not an executor error.
        let mut exec = AgentExecutor::new(default_runtime());
        let run = exec.run("run exit 7").unwrap();
        assert_eq!(run.steps[0].tool, "bash");
        assert!(!run.steps[0].ok, "non-zero exit should report ok=false");
        // The loop still reached the respond step.
        assert!(run.steps.iter().any(|s| s.tool == "respond"));
    }

    #[test]
    fn second_run_recalls_prior_output_into_the_loop() {
        // The executor retains memory/recall across run() calls. A later turn
        // whose goal overlaps an earlier turn's output should pull that prior
        // context back into the loop and journal a Recall event — the "observe
        // what we already know" half of think -> act -> observe.
        let mut exec = AgentExecutor::new(default_runtime());
        exec.run("run echo alphatoken bravotoken charlietoken")
            .unwrap();
        assert_eq!(
            exec.session().of_kind(EventKind::Recall).len(),
            0,
            "nothing to recall on the first turn"
        );
        exec.run("run echo alphatoken bravotoken charlietoken")
            .unwrap();
        assert!(
            !exec.session().of_kind(EventKind::Recall).is_empty(),
            "second turn should recall the first turn's output"
        );
    }

    #[test]
    fn with_scope_routes_memory_records() {
        let mut exec = AgentExecutor::new(default_runtime()).with_scope("alpha");
        exec.run("run echo scoped").unwrap();
        assert!(!exec.memory().list_scope("alpha").unwrap().is_empty());
        assert!(exec.memory().list_scope("beta").unwrap().is_empty());
    }
}
