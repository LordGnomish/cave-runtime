// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Batch tool execution.
//!
//! Runs a list of [`BatchCall`]s against a [`ToolRegistry`] in one pass.
//! Calls may declare dependencies (`after`), forming a DAG that is
//! topologically ordered before execution; cycles and references to
//! unknown ids are rejected up front. Two [`BatchMode`]s control failure
//! handling:
//!
//! * [`BatchMode::FailFast`] — the first failing call aborts the batch; all
//!   not-yet-run calls are [`CallStatus::Skipped`].
//! * [`BatchMode::ContinueOnError`] — every independent call runs; a call
//!   whose dependency failed (or was skipped) is itself skipped.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

use crate::error::{Result, ToolError};
use crate::tool::{ToolRegistry, ToolResult};

/// One call within a batch.
#[derive(Debug, Clone)]
pub struct BatchCall {
    pub id: String,
    pub tool: String,
    pub args: Value,
    pub depends_on: Vec<String>,
}

impl BatchCall {
    pub fn new(id: impl Into<String>, tool: impl Into<String>, args: Value) -> Self {
        Self {
            id: id.into(),
            tool: tool.into(),
            args,
            depends_on: Vec::new(),
        }
    }

    /// Declare that this call must run after the given call ids.
    pub fn after<I, S>(mut self, deps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.depends_on = deps.into_iter().map(Into::into).collect();
        self
    }
}

/// Failure-handling strategy for a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchMode {
    /// Abort the batch on the first failure.
    FailFast,
    /// Run every independent call; skip dependents of failures.
    ContinueOnError,
}

/// Per-call result.
#[derive(Debug, Clone)]
pub enum CallStatus {
    Ok(ToolResult),
    Failed(String),
    Skipped(String),
}

impl CallStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, CallStatus::Ok(_))
    }
}

/// Outcome of a single call, carrying its id and tool for correlation.
#[derive(Debug, Clone)]
pub struct BatchOutcome {
    pub id: String,
    pub tool: String,
    pub status: CallStatus,
}

/// Executes batches against a borrowed registry.
pub struct BatchExecutor<'a> {
    registry: &'a ToolRegistry,
}

impl<'a> BatchExecutor<'a> {
    pub fn new(registry: &'a ToolRegistry) -> Self {
        Self { registry }
    }

    /// Run a batch. Returns outcomes in *execution* (topological) order, or
    /// an error if the dependency graph is invalid (cycle or unknown id).
    pub fn run(&self, calls: &[BatchCall], mode: BatchMode) -> Result<Vec<BatchOutcome>> {
        let order = topo_order(calls)?;
        let by_id: BTreeMap<&str, &BatchCall> =
            calls.iter().map(|c| (c.id.as_str(), c)).collect();

        let mut status: BTreeMap<String, CallStatus> = BTreeMap::new();
        let mut aborted = false;

        for id in &order {
            let call = by_id[id.as_str()];

            // A dependency that failed or was skipped poisons this call.
            let mut blocked_by = None;
            for dep in &call.depends_on {
                match status.get(dep) {
                    Some(CallStatus::Ok(_)) => {}
                    Some(_) => {
                        blocked_by = Some(dep.clone());
                        break;
                    }
                    None => {} // dependency hasn't run (e.g. fail-fast abort)
                }
            }

            let outcome = if aborted {
                CallStatus::Skipped("batch aborted by earlier failure".into())
            } else if let Some(dep) = blocked_by {
                CallStatus::Skipped(format!("dependency `{dep}` did not succeed"))
            } else {
                match self.registry.invoke_validated(&call.tool, &call.args) {
                    Ok(res) => CallStatus::Ok(res),
                    Err(e) => {
                        if mode == BatchMode::FailFast {
                            aborted = true;
                        }
                        CallStatus::Failed(e.to_string())
                    }
                }
            };
            status.insert(id.clone(), outcome);
        }

        Ok(order
            .into_iter()
            .map(|id| BatchOutcome {
                tool: by_id[id.as_str()].tool.clone(),
                status: status.remove(&id).unwrap(),
                id,
            })
            .collect())
    }
}

/// Kahn topological sort over the dependency DAG. Errors on unknown
/// dependency ids and on cycles.
fn topo_order(calls: &[BatchCall]) -> Result<Vec<String>> {
    let ids: BTreeSet<&str> = calls.iter().map(|c| c.id.as_str()).collect();
    if ids.len() != calls.len() {
        return Err(ToolError::Protocol("duplicate call id in batch".into()));
    }

    let mut indegree: BTreeMap<&str, usize> = calls.iter().map(|c| (c.id.as_str(), 0)).collect();
    let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for c in calls {
        for dep in &c.depends_on {
            if !ids.contains(dep.as_str()) {
                return Err(ToolError::Protocol(format!(
                    "call `{}` depends on unknown id `{dep}`",
                    c.id
                )));
            }
            // edge dep -> c (dep must precede c)
            edges.entry(dep.as_str()).or_default().push(c.id.as_str());
            *indegree.get_mut(c.id.as_str()).unwrap() += 1;
        }
    }

    // Seed with zero-indegree nodes in declaration order (stable output).
    let mut queue: VecDeque<&str> = calls
        .iter()
        .map(|c| c.id.as_str())
        .filter(|id| indegree[id] == 0)
        .collect();

    let mut order = Vec::with_capacity(calls.len());
    while let Some(node) = queue.pop_front() {
        order.push(node.to_string());
        if let Some(succs) = edges.get(node) {
            for &s in succs {
                let d = indegree.get_mut(s).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push_back(s);
                }
            }
        }
    }

    if order.len() != calls.len() {
        return Err(ToolError::Protocol(
            "dependency cycle detected in batch".into(),
        ));
    }
    Ok(order)
}
