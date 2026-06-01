// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-agent orchestration — the "amele" worker pattern.
//!
//! Ports the coordinator half of Hermes' `acp_adapter/` + `acp_registry/`
//! (Agent Communication Protocol). Where [`crate::agent::AgentExecutor`] is
//! a *single* agent running one goal, the [`Orchestrator`] is a coordinator
//! that fans a larger goal out across a pool of isolated worker agents —
//! Burak's "amele" (laborer) pattern: many cheap workers, each handed one
//! chunk of work, results collected centrally.
//!
//! ```text
//!            goal / Vec<Subtask>
//!                   │  decompose + topological sort (deps)
//!                   ▼
//!   coordinator ──┬─▶ worker 0  (fresh HermesRuntime → AgentExecutor)
//!                 ├─▶ worker 1   …round-robin over the pool…
//!                 └─▶ worker n
//!                   │  collect WorkerResult per subtask
//!                   ▼
//!            OrchestrationReport { results, completed, failed }
//! ```
//!
//! Each worker gets its *own* runtime from a caller-supplied
//! [`RuntimeFactory`], so memory / recall / session state never leaks
//! between subtasks — the same isolation upstream's per-agent ACP sessions
//! provide. Dependencies are declared per subtask and resolved with a
//! deterministic topological sort (Kahn's algorithm); a cycle, a duplicate
//! id, or a dangling dependency is rejected up front with
//! [`HermesError::Orchestration`]. A subtask whose dependency failed is
//! skipped (not run) and reported as failed; with [`Orchestrator::fail_fast`]
//! the coordinator stops at the first failure instead.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::HermesRuntime;
use crate::agent::AgentExecutor;
use crate::error::{HermesError, Result};

/// Builds a fresh, isolated runtime for a worker. Called once per executed
/// subtask. `Arc<dyn Fn>` so the orchestrator is cheap to clone/share.
pub type RuntimeFactory = Arc<dyn Fn() -> HermesRuntime + Send + Sync>;

/// One unit of delegated work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtask {
    pub id: String,
    pub goal: String,
    /// Ids of subtasks that must complete successfully before this one runs.
    pub deps: Vec<String>,
}

impl Subtask {
    pub fn new(id: impl Into<String>, goal: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            goal: goal.into(),
            deps: Vec::new(),
        }
    }

    /// Declare that this subtask depends on `dep`.
    pub fn after(mut self, dep: impl Into<String>) -> Self {
        self.deps.push(dep.into());
        self
    }
}

/// Outcome of a single worker running a single subtask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerResult {
    pub subtask_id: String,
    /// Index of the pool worker that handled this subtask (round-robin).
    pub worker: usize,
    pub ok: bool,
    /// Final agent response on success, or the failure / skip reason.
    pub output: String,
}

/// Aggregated result of an orchestration run. `results` is in execution
/// (topological) order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestrationReport {
    pub results: Vec<WorkerResult>,
    pub completed: usize,
    pub failed: usize,
}

impl OrchestrationReport {
    /// Look up a result by subtask id.
    pub fn get(&self, id: &str) -> Option<&WorkerResult> {
        self.results.iter().find(|r| r.subtask_id == id)
    }

    /// True when every executed subtask succeeded (and at least one ran).
    pub fn all_ok(&self) -> bool {
        self.failed == 0 && !self.results.is_empty()
    }
}

/// Coordinator that dispatches subtasks across a pool of worker agents.
pub struct Orchestrator {
    factory: RuntimeFactory,
    pool_size: usize,
    fail_fast: bool,
}

impl Orchestrator {
    /// New coordinator with a 4-worker pool and continue-on-failure policy.
    pub fn new(factory: RuntimeFactory) -> Self {
        Self {
            factory,
            pool_size: 4,
            fail_fast: false,
        }
    }

    /// Set the worker-pool size (round-robin assignment). Clamped to ≥ 1.
    pub fn with_pool_size(mut self, n: usize) -> Self {
        self.pool_size = n.max(1);
        self
    }

    /// When `true`, stop at the first failed subtask instead of continuing.
    pub fn fail_fast(mut self, yes: bool) -> Self {
        self.fail_fast = yes;
        self
    }

    /// Decompose a multi-line goal — one non-blank line per subtask, all
    /// independent — and run it. Convenience over [`run`](Self::run) for the
    /// common "here is a batch of chores, fan them out" case.
    pub fn run_goal(&self, goal: &str) -> Result<OrchestrationReport> {
        let subtasks: Vec<Subtask> = goal
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .enumerate()
            .map(|(i, line)| Subtask::new(format!("t{i}"), line))
            .collect();
        if subtasks.is_empty() {
            return Err(HermesError::Orchestration(
                "goal decomposed to zero subtasks".into(),
            ));
        }
        self.run(subtasks)
    }

    /// Run an explicit subtask graph. Validates ids + dependencies, sorts
    /// topologically, then dispatches each subtask to a worker.
    pub fn run(&self, subtasks: Vec<Subtask>) -> Result<OrchestrationReport> {
        self.validate(&subtasks)?;
        let order = topological_order(&subtasks)?;

        let mut results: Vec<WorkerResult> = Vec::with_capacity(order.len());
        let mut status: HashMap<String, bool> = HashMap::new();
        let mut next_worker = 0usize;

        for &idx in &order {
            let s = &subtasks[idx];
            let worker = next_worker % self.pool_size;
            next_worker += 1;

            // A subtask whose dependency failed is skipped, not run.
            if let Some(bad) = s
                .deps
                .iter()
                .find(|d| status.get(*d).copied() == Some(false))
            {
                let wr = WorkerResult {
                    subtask_id: s.id.clone(),
                    worker,
                    ok: false,
                    output: format!("skipped: dependency '{bad}' failed"),
                };
                status.insert(s.id.clone(), false);
                results.push(wr);
                continue;
            }

            let mut exec = AgentExecutor::new((self.factory)()).with_scope(&s.id);
            let (ok, output) = match exec.run(&s.goal) {
                Ok(run) => (run.steps.iter().all(|st| st.ok), run.final_response),
                Err(e) => (false, e.to_string()),
            };
            status.insert(s.id.clone(), ok);
            results.push(WorkerResult {
                subtask_id: s.id.clone(),
                worker,
                ok,
                output,
            });

            if self.fail_fast && !ok {
                break;
            }
        }

        let completed = results.iter().filter(|r| r.ok).count();
        let failed = results.iter().filter(|r| !r.ok).count();
        Ok(OrchestrationReport {
            results,
            completed,
            failed,
        })
    }

    fn validate(&self, subtasks: &[Subtask]) -> Result<()> {
        let mut ids = HashSet::with_capacity(subtasks.len());
        for s in subtasks {
            if !ids.insert(s.id.as_str()) {
                return Err(HermesError::Orchestration(format!(
                    "duplicate subtask id '{}'",
                    s.id
                )));
            }
        }
        for s in subtasks {
            for d in &s.deps {
                if !ids.contains(d.as_str()) {
                    return Err(HermesError::Orchestration(format!(
                        "subtask '{}' depends on unknown subtask '{}'",
                        s.id, d
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Kahn's algorithm. Returns subtask indices in dependency order, processing
/// ready nodes in their original input order for determinism. Errors on a
/// cycle.
fn topological_order(subtasks: &[Subtask]) -> Result<Vec<usize>> {
    let index: HashMap<&str, usize> = subtasks
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    let mut indegree = vec![0usize; subtasks.len()];
    // dependents[i] = subtasks that depend on subtask i
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); subtasks.len()];
    for (i, s) in subtasks.iter().enumerate() {
        for d in &s.deps {
            let di = index[d.as_str()];
            indegree[i] += 1;
            dependents[di].push(i);
        }
    }

    // Ready set scanned in input order to keep output deterministic.
    let mut order = Vec::with_capacity(subtasks.len());
    let mut done = vec![false; subtasks.len()];
    loop {
        let mut progressed = false;
        for i in 0..subtasks.len() {
            if !done[i] && indegree[i] == 0 {
                done[i] = true;
                order.push(i);
                progressed = true;
                for &dep in &dependents[i] {
                    indegree[dep] -= 1;
                }
            }
        }
        if order.len() == subtasks.len() {
            break;
        }
        if !progressed {
            return Err(HermesError::Orchestration(
                "dependency cycle detected among subtasks".into(),
            ));
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_runtime;

    fn factory() -> RuntimeFactory {
        Arc::new(default_runtime)
    }

    #[test]
    fn empty_subtasks_yields_empty_report() {
        let report = Orchestrator::new(factory()).run(vec![]).unwrap();
        assert_eq!(report.results.len(), 0);
        assert!(!report.all_ok(), "empty run is not 'all ok'");
    }

    #[test]
    fn pool_size_clamped_to_at_least_one() {
        let orch = Orchestrator::new(factory()).with_pool_size(0);
        // A zero pool would panic on `% 0`; clamping keeps run() safe.
        let report = orch.run(vec![Subtask::new("t", "run echo x")]).unwrap();
        assert_eq!(report.results[0].worker, 0);
    }

    #[test]
    fn topological_order_chain_is_linear() {
        let subs = vec![
            Subtask::new("c", "g").after("b"),
            Subtask::new("b", "g").after("a"),
            Subtask::new("a", "g"),
        ];
        let order = topological_order(&subs).unwrap();
        // a(idx2) → b(idx1) → c(idx0)
        assert_eq!(order, vec![2, 1, 0]);
    }

    /// Planner that turns any goal into a single `respond` step whose
    /// rationale *is* the goal — so a worker's `final_response` echoes back
    /// exactly the (possibly augmented) goal it was handed. Lets us observe
    /// what the coordinator actually fed each worker.
    struct EchoPlanner;
    impl crate::planner::Planner for EchoPlanner {
        fn plan(&self, goal: &str) -> Result<crate::planner::Plan> {
            Ok(crate::planner::Plan::new(goal)
                .push(crate::planner::PlanStep::new("respond", goal)))
        }
    }

    fn echo_factory() -> RuntimeFactory {
        Arc::new(|| {
            let mut rt = default_runtime();
            rt.planner = Box::new(EchoPlanner);
            rt
        })
    }

    #[test]
    fn dependent_worker_receives_dependency_output_as_context() {
        // a → b. The coordinator must thread a's output into b's goal so the
        // dependent worker can build on it (the amele coordinator's job).
        let subs = vec![
            Subtask::new("a", "PRODUCE_ALPHA"),
            Subtask::new("b", "consume").after("a"),
        ];
        let report = Orchestrator::new(echo_factory()).run(subs).unwrap();
        let b = report.get("b").expect("subtask b ran");
        assert!(
            b.output.contains("PRODUCE_ALPHA"),
            "dependent worker should see dependency output; got: {}",
            b.output
        );
    }
}
