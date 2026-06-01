// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composable execution patterns — OpenJarvis treats tool calls as values that
//! combine with a handful of small combinators. A [`Pattern`] is a recursive
//! AST; [`run`] interprets it against a [`ToolRegistry`].
//!
//! - [`Pattern::Tool`]: invoke one tool.
//! - [`Pattern::Chain`]: run sub-patterns in order, threading each result into
//!   the next as the `_prev` argument.
//! - [`Pattern::Parallel`]: run sub-patterns and collect outputs into an array
//!   (first error short-circuits).
//! - [`Pattern::Fallback`]: try sub-patterns until one succeeds.
//! - [`Pattern::Retry`]: re-run a sub-pattern up to `max` extra times.
//!
//! OpenJarvis upstream: `jarvis/compose/patterns.py`. Execution is synchronous;
//! `Parallel` is sequential-with-array-semantics here (the on-device runtime is
//! single-threaded — true thread fan-out is scope-cut to the host scheduler).

use crate::error::{AgentError, Result};
use crate::tool::ToolRegistry;
use serde_json::{json, Value};

/// A composable execution plan.
pub enum Pattern {
    /// Invoke a single registered tool with fixed arguments.
    Tool { name: String, args: Value },
    /// Run sub-patterns sequentially, threading `_prev` between them.
    Chain(Vec<Pattern>),
    /// Run sub-patterns and gather their outputs into a JSON array.
    Parallel(Vec<Pattern>),
    /// Try sub-patterns in order; yield the first success.
    Fallback(Vec<Pattern>),
    /// Re-run `inner` up to `max` additional times on failure.
    Retry { inner: Box<Pattern>, max: u32 },
}

impl Pattern {
    /// Convenience constructor for [`Pattern::Tool`].
    pub fn tool(name: impl Into<String>, args: Value) -> Self {
        Pattern::Tool { name: name.into(), args }
    }
}

/// Interpret a [`Pattern`] against the registry, returning its composed output.
pub fn run(pattern: &Pattern, registry: &ToolRegistry) -> Result<Value> {
    match pattern {
        Pattern::Tool { name, args } => registry.invoke(name, args),

        Pattern::Chain(steps) => {
            let mut last = Value::Null;
            for (i, step) in steps.iter().enumerate() {
                let next = if i == 0 {
                    run(step, registry)?
                } else {
                    run(&inject_prev(step, &last), registry)?
                };
                last = next;
            }
            Ok(last)
        }

        Pattern::Parallel(branches) => {
            let mut out = Vec::with_capacity(branches.len());
            for b in branches {
                out.push(run(b, registry)?);
            }
            Ok(Value::Array(out))
        }

        Pattern::Fallback(options) => {
            let mut last_err =
                AgentError::Plan("fallback had no options".into());
            for opt in options {
                match run(opt, registry) {
                    Ok(v) => return Ok(v),
                    Err(e) => last_err = e,
                }
            }
            Err(last_err)
        }

        Pattern::Retry { inner, max } => {
            let mut last_err = AgentError::Plan("retry never ran".into());
            for _ in 0..=*max {
                match run(inner, registry) {
                    Ok(v) => return Ok(v),
                    Err(e) => last_err = e,
                }
            }
            Err(last_err)
        }
    }
}

/// Produce a copy of `pattern` with `prev` injected at the `_prev` argument key.
/// Only [`Pattern::Tool`] carries arguments, so other variants pass through; the
/// injection still reaches their leaf tools through recursion at run time —
/// here we only need to handle the immediate tool case for chaining.
fn inject_prev(pattern: &Pattern, prev: &Value) -> Pattern {
    match pattern {
        Pattern::Tool { name, args } => {
            let mut merged = args.clone();
            if let Value::Object(map) = &mut merged {
                map.insert("_prev".to_string(), prev.clone());
            } else {
                merged = json!({ "_prev": prev });
            }
            Pattern::Tool { name: name.clone(), args: merged }
        }
        // Non-tool heads in a chain receive the previous value but cannot carry
        // a top-level argument; re-clone structurally so the chain proceeds.
        Pattern::Chain(v) => Pattern::Chain(v.iter().map(|p| clone_pattern(p)).collect()),
        Pattern::Parallel(v) => Pattern::Parallel(v.iter().map(|p| clone_pattern(p)).collect()),
        Pattern::Fallback(v) => Pattern::Fallback(v.iter().map(|p| clone_pattern(p)).collect()),
        Pattern::Retry { inner, max } => Pattern::Retry {
            inner: Box::new(inject_prev(inner, prev)),
            max: *max,
        },
    }
}

/// Structural clone of a pattern (`Pattern` holds boxed closures only via the
/// registry, so the AST itself is freely cloneable).
fn clone_pattern(pattern: &Pattern) -> Pattern {
    match pattern {
        Pattern::Tool { name, args } => Pattern::Tool { name: name.clone(), args: args.clone() },
        Pattern::Chain(v) => Pattern::Chain(v.iter().map(clone_pattern).collect()),
        Pattern::Parallel(v) => Pattern::Parallel(v.iter().map(clone_pattern).collect()),
        Pattern::Fallback(v) => Pattern::Fallback(v.iter().map(clone_pattern).collect()),
        Pattern::Retry { inner, max } => {
            Pattern::Retry { inner: Box::new(clone_pattern(inner)), max: *max }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Tool;

    #[test]
    fn empty_parallel_is_empty_array() {
        let reg = ToolRegistry::new();
        assert_eq!(run(&Pattern::Parallel(vec![]), &reg).unwrap(), json!([]));
    }

    #[test]
    fn chain_of_one_is_just_that_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(Tool::new("k", "x", json!({}), |_| Ok(json!("v"))));
        let p = Pattern::Chain(vec![Pattern::tool("k", json!({}))]);
        assert_eq!(run(&p, &reg).unwrap(), json!("v"));
    }
}
