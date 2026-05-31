// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: faithful port of OPA's `format` package core (the `opa fmt`
//! pretty-printer engine), closing the over-broad `internal/cmd/fmt` scope-cut.
//!
//! Upstream: open-policy-agent/opa v1.16.2 `format/format.go`.
//!
//! The `opa fmt` *CLI* (file globbing, `-w` in-place write, diff output) is
//! scope-cut to cave-cli; the canonical-printing CORE — producing a single
//! deterministic rendering of a parsed module — is library code. We port that
//! core's normalization rules:
//!   * `package` line, blank line, sorted `import` block, blank line, then
//!     rules separated by single blank lines;
//!   * tab indentation for rule bodies;
//!   * exactly one space around `=`, `:=`, `==`, `!=`, `<`, `<=`, `>`, `>=`;
//!   * `[a, b]` / `{a, b}` / `{"k": v}` collection spacing.
//!
//! The defining property of `opa fmt` is **idempotence** (`fmt(fmt(x)) ==
//! fmt(x)`) and **re-parseability** (the output is always valid Rego). Both are
//! asserted here in addition to exact canonical snapshots.

use cave_policy::rego::format::{format_module, format_source};
use cave_policy::rego::parser::parse_module;

/// A messy single-rule module is normalized: package/blank/rule layout, tab
/// indent, and one space around the comparison operator.
#[test]
fn normalizes_layout_and_spacing() {
    let messy = "package example\n\n\nallow{input.x==1}";
    let out = format_source(messy).expect("formats");
    assert_eq!(out, "package example\n\nallow {\n\tinput.x == 1\n}\n");
}

/// Imports are emitted as their own blank-line-separated block, sorted by path.
#[test]
fn sorts_and_groups_imports() {
    let messy = "package p\nimport data.zeta\nimport data.alpha\nallow { true }";
    let out = format_source(messy).expect("formats");
    assert_eq!(
        out,
        "package p\n\nimport data.alpha\nimport data.zeta\n\nallow {\n\ttrue\n}\n"
    );
}

/// A `default` rule renders on one line with no body, using `:=`.
#[test]
fn default_rule_one_line() {
    let out = format_source("package p\ndefault   allow=false").expect("formats");
    assert_eq!(out, "package p\n\ndefault allow := false\n");
}

/// Multiple body expressions each land on their own tab-indented line.
#[test]
fn multi_expr_body_one_per_line() {
    let messy = "package p\nallow { input.a==1;input.b==2 }";
    let out = format_source(messy).expect("formats");
    assert_eq!(
        out,
        "package p\n\nallow {\n\tinput.a == 1\n\tinput.b == 2\n}\n"
    );
}

/// `:=` assignment inside a body is preserved and spaced.
#[test]
fn assignment_in_body_uses_walrus() {
    let messy = "package p\nr { x:=1 }";
    let out = format_source(messy).expect("formats");
    assert_eq!(out, "package p\n\nr {\n\tx := 1\n}\n");
}

/// Rendering is idempotent and always re-parses for a representative corpus.
#[test]
fn idempotent_and_reparseable() {
    let corpus = [
        "package example\nallow{input.x==1}",
        "package a.b.c\nimport data.foo\ndefault allow=false\nallow{input.role==\"admin\"}",
        "package p\ndeny[msg]{input.bad;msg:=\"no\"}",
        "package p\nf(x){x>5}",
    ];
    for src in corpus {
        let m = parse_module(src).expect("source parses");
        let once = format_module(&m);
        // Re-parse the formatted output: it must be valid Rego.
        let m2 = parse_module(&once).expect("formatted output re-parses");
        let twice = format_module(&m2);
        assert_eq!(once, twice, "fmt must be idempotent for: {src}");
    }
}

/// Trailing newline is always present (POSIX text-file convention, matching
/// `opa fmt`).
#[test]
fn always_ends_with_newline() {
    let out = format_source("package p\nallow { true }").expect("formats");
    assert!(out.ends_with('\n'));
    assert!(!out.ends_with("\n\n"));
}
