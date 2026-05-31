// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: faithful port of OPA v1/tester Runner (internal/cmd/test core).
//!
//! Upstream: open-policy-agent/opa v1.16.2 `v1/tester/runner.go`.
//!   - `TestPrefix = "test_"`, `SkipTestPrefix = "todo_test_"`.
//!   - PASS iff the test rule is defined and evaluates to boolean `true`.
//!   - FAIL on undefined (empty result) OR a non-`true` value (false / non-bool).
//!   - SKIP when the rule name begins with `todo_test_`.
//!
//! The pure test-execution engine is a library capability (it consumes the
//! in-crate Rego evaluator); only the `opa test` CLI flag-parsing belongs to
//! cave-cli. This closes the over-broad `internal/cmd/test` scope-cut.

use cave_policy::rego::tester::{Runner, TestResult};
use cave_policy::rego::PolicyEngine;

fn load(src: &str) -> PolicyEngine {
    let mut e = PolicyEngine::new();
    e.load_module("unit.rego", src).expect("module parses");
    e
}

fn by_name<'a>(results: &'a [TestResult], name: &str) -> &'a TestResult {
    results
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no test result named {name}"))
}

/// A `test_*` rule whose body holds (yields `true`) PASSES.
#[test]
fn passing_test_rule_passes() {
    let e = load(
        r#"
        package example

        test_addition_is_correct {
            1 + 1 == 2
        }
        "#,
    );
    let results = Runner::new().run(&e);
    let r = by_name(&results, "test_addition_is_correct");
    assert_eq!(r.package, "example");
    assert!(r.pass(), "rule yielding true must PASS: {r:?}");
    assert!(!r.fail && !r.skip && r.error.is_none());
}

/// An *undefined* result (body fails → rule undefined) FAILS — not an error.
#[test]
fn undefined_test_rule_fails() {
    let e = load(
        r#"
        package example

        test_one_equals_two {
            1 == 2
        }
        "#,
    );
    let results = Runner::new().run(&e);
    let r = by_name(&results, "test_one_equals_two");
    assert!(r.fail, "undefined result must FAIL: {r:?}");
    assert!(!r.pass());
}

/// A complete rule that evaluates to boolean `false` FAILS.
#[test]
fn false_valued_test_rule_fails() {
    let e = load(
        r#"
        package example

        default test_explicitly_false = false

        test_explicitly_false {
            1 == 2
        }
        "#,
    );
    let results = Runner::new().run(&e);
    let r = by_name(&results, "test_explicitly_false");
    assert!(r.fail, "false-valued result must FAIL: {r:?}");
}

/// `todo_test_*` rules are SKIPPED (not run), regardless of body.
#[test]
fn todo_prefixed_rule_is_skipped() {
    let e = load(
        r#"
        package example

        todo_test_not_ready {
            1 == 1
        }
        "#,
    );
    let results = Runner::new().run(&e);
    let r = by_name(&results, "todo_test_not_ready");
    assert!(r.skip, "todo_test_ rule must be SKIPPED: {r:?}");
    assert!(!r.pass());
    assert!(!r.fail);
}

/// Non-test rules are ignored entirely (no result emitted for them).
#[test]
fn non_test_rules_are_ignored() {
    let e = load(
        r#"
        package example

        allow {
            1 == 1
        }

        test_real {
            allow
        }
        "#,
    );
    let results = Runner::new().run(&e);
    assert_eq!(results.len(), 1, "only the test_ rule yields a result");
    assert_eq!(results[0].name, "test_real");
    assert!(results[0].pass());
}

/// Package path is reported with dotted notation for nested packages.
#[test]
fn nested_package_path_is_reported() {
    let e = load(
        r#"
        package authz.rbac

        test_in_subpackage {
            true
        }
        "#,
    );
    let results = Runner::new().run(&e);
    let r = by_name(&results, "test_in_subpackage");
    assert_eq!(r.package, "authz.rbac");
    assert!(r.pass());
}

/// A `data.<pkg>.test_*` name filter restricts which tests run.
#[test]
fn name_filter_restricts_execution() {
    let e = load(
        r#"
        package example

        test_keep {
            true
        }

        test_drop {
            true
        }
        "#,
    );
    let results = Runner::new()
        .with_name_filter("keep")
        .run(&e);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "test_keep");
}

/// Summary aggregates pass/fail/skip across the whole run.
#[test]
fn summary_aggregates_counts() {
    let e = load(
        r#"
        package example

        test_a { true }
        test_b { 1 == 2 }
        todo_test_c { true }
        "#,
    );
    let results = Runner::new().run(&e);
    let summary = cave_policy::rego::tester::summarize(&results);
    assert_eq!(summary.total, 3);
    assert_eq!(summary.passed, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.skipped, 1);
    assert!(!summary.all_passed(), "a failure means not all_passed");
}
