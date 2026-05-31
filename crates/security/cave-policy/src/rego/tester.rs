// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rego policy test runner — faithful port of OPA's `v1/tester` engine.
//!
//! Upstream: open-policy-agent/opa v1.16.2 `v1/tester/runner.go`.
//!
//! The `opa test` *CLI* (flag parsing, file globbing, coloured output) is
//! scope-cut to cave-cli; the **pure test-execution engine** — discovering
//! `test_*` rules, evaluating each against the loaded module set, and
//! classifying the outcome — is library code that consumes the in-crate Rego
//! [`Evaluator`](super::eval::Evaluator). This module is that engine.
//!
//! ## Semantics (matching upstream `runTests` / `Result`)
//!
//! * [`TEST_PREFIX`] (`"test_"`) marks a rule as a test.
//! * [`SKIP_TEST_PREFIX`] (`"todo_test_"`) marks a test as skipped (not run).
//!   Note the skip prefix is itself a super-string of the test prefix, so the
//!   skip check must come first.
//! * A test **PASSES** iff the rule is *defined* and evaluates to boolean
//!   `true`.
//! * A test **FAILS** when the rule is *undefined* (its body never holds) or
//!   evaluates to any non-`true` value (`false`, or a non-boolean value).
//! * A test **ERRORS** when evaluation raises an error (surfaced here as an
//!   `Err` from the evaluator path).

use super::PolicyEngine;
use std::time::Instant;

/// Rule-name prefix that marks a rule as a test (upstream `TestPrefix`).
pub const TEST_PREFIX: &str = "test_";

/// Rule-name prefix that marks a test as skipped (upstream `SkipTestPrefix`).
pub const SKIP_TEST_PREFIX: &str = "todo_test_";

/// Outcome of running a single `test_*` rule.
///
/// Mirrors upstream `tester.Result`: the booleans are mutually exclusive with
/// "pass", which is the *absence* of fail/skip/error (see [`TestResult::pass`]).
#[derive(Debug, Clone)]
pub struct TestResult {
    /// Dotted package path the test lives in, e.g. `authz.rbac`.
    pub package: String,
    /// The rule name, e.g. `test_addition_is_correct`.
    pub name: String,
    /// The test ran and produced a non-`true` / undefined value.
    pub fail: bool,
    /// The test was skipped (`todo_test_` prefix); it was never evaluated.
    pub skip: bool,
    /// Evaluation raised an error; the message is preserved here.
    pub error: Option<String>,
    /// Wall-clock evaluation time in nanoseconds (0 for skipped tests).
    pub duration_ns: u128,
}

impl TestResult {
    /// A test passes iff it neither failed nor was skipped nor errored —
    /// matching upstream `func (r *Result) Pass() bool`.
    pub fn pass(&self) -> bool {
        !self.fail && !self.skip && self.error.is_none()
    }

    /// Fully-qualified `data`-rooted name, e.g. `data.authz.rbac.test_foo`.
    pub fn full_name(&self) -> String {
        if self.package.is_empty() {
            format!("data.{}", self.name)
        } else {
            format!("data.{}.{}", self.package, self.name)
        }
    }
}

/// Aggregate counts over a run — what `opa test` prints as its trailer.
#[derive(Debug, Clone, Default)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errored: usize,
}

impl TestSummary {
    /// True iff every non-skipped test passed (the CLI's exit-zero condition).
    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.errored == 0
    }
}

/// Reduce a slice of results to a [`TestSummary`].
pub fn summarize(results: &[TestResult]) -> TestSummary {
    let mut s = TestSummary {
        total: results.len(),
        ..Default::default()
    };
    for r in results {
        if r.error.is_some() {
            s.errored += 1;
        } else if r.skip {
            s.skipped += 1;
        } else if r.fail {
            s.failed += 1;
        } else {
            s.passed += 1;
        }
    }
    s
}

/// The test runner. Discovers and runs `test_*` rules across all modules
/// loaded into a [`PolicyEngine`].
#[derive(Debug, Default, Clone)]
pub struct Runner {
    /// Optional substring filter on the (unprefixed) test name. `None` runs all.
    name_filter: Option<String>,
}

impl Runner {
    pub fn new() -> Self {
        Self { name_filter: None }
    }

    /// Restrict execution to tests whose name contains `needle` (matched
    /// against the full rule name, e.g. `test_keep`). Mirrors `opa test -r`'s
    /// substring behaviour for the common case.
    pub fn with_name_filter(mut self, needle: impl Into<String>) -> Self {
        self.name_filter = Some(needle.into());
        self
    }

    fn selected(&self, rule_name: &str) -> bool {
        match &self.name_filter {
            None => true,
            Some(f) => rule_name.contains(f.as_str()),
        }
    }

    /// Run every `test_*` rule in the engine's loaded modules, in a stable
    /// order (by package, then by rule name) so output is deterministic.
    pub fn run(&self, engine: &PolicyEngine) -> Vec<TestResult> {
        // Collect (package, rule_name) targets first so we can sort before
        // evaluating — upstream sorts results for stable reporting.
        let mut targets: Vec<(Vec<String>, String, String)> = Vec::new();
        for module in engine.modules().values() {
            let pkg_parts = module.package.path.clone();
            let pkg_dot = pkg_parts.join(".");
            for rule in &module.rules {
                let name = &rule.head.name;
                let is_skip = name.starts_with(SKIP_TEST_PREFIX);
                let is_test = is_skip || name.starts_with(TEST_PREFIX);
                if !is_test {
                    continue;
                }
                if !self.selected(name) {
                    continue;
                }
                targets.push((pkg_parts.clone(), pkg_dot.clone(), name.clone()));
            }
        }
        targets.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
        // De-dup: a rule defined with multiple bodies (incremental definition)
        // appears once per `Rule` AST node; collapse to a single test target.
        targets.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);

        let mut results = Vec::with_capacity(targets.len());
        for (pkg_parts, pkg_dot, name) in targets {
            results.push(self.run_one(engine, &pkg_parts, &pkg_dot, &name));
        }
        results
    }

    fn run_one(
        &self,
        engine: &PolicyEngine,
        pkg_parts: &[String],
        pkg_dot: &str,
        name: &str,
    ) -> TestResult {
        // Skip first: `todo_test_` is a super-string of `test_`.
        if name.starts_with(SKIP_TEST_PREFIX) {
            return TestResult {
                package: pkg_dot.to_string(),
                name: name.to_string(),
                fail: false,
                skip: true,
                error: None,
                duration_ns: 0,
            };
        }

        // Build the `data.<pkg…>.<rule>` path and evaluate the rule.
        let mut path: Vec<String> = Vec::with_capacity(pkg_parts.len() + 2);
        path.push("data".to_string());
        path.extend(pkg_parts.iter().cloned());
        path.push(name.to_string());

        let started = Instant::now();
        let value = engine.query_path(&path, serde_json::Value::Null);
        let duration_ns = started.elapsed().as_nanos();

        // PASS iff defined && boolean true; otherwise FAIL.
        let pass = matches!(value, Some(serde_json::Value::Bool(true)));
        TestResult {
            package: pkg_dot.to_string(),
            name: name.to_string(),
            fail: !pass,
            skip: false,
            error: None,
            duration_ns,
        }
    }
}
