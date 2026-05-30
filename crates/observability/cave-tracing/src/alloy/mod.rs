// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Grafana Alloy engine reimplementation.
//!
//! Upstream: <https://github.com/grafana/alloy> (Apache-2.0), pinned v1.5.0.
//!
//! Alloy is configured in the Alloy configuration syntax (historically called
//! "River"): a declarative, HCL-like language whose
//! `component.kind "label" { attr = expr ... }` blocks describe a directed
//! acyclic graph of [`component`]s that the controller evaluates and runs.
//!
//! This module ports that engine, lowest layer first:
//!
//! ```text
//! source bytes ─ scanner ─ tokens ─ parser ─ ast ─ controller ─ component graph
//!                  │                            │                     │
//!               token.rs                    parser.rs              graph.rs
//!                                            ast.rs               component.rs
//! ```
//!
//! Each layer is line-ported from the corresponding Alloy Go package under a
//! strict RED→GREEN TDD discipline; see `tests/alloy_*_tdd.rs`.

/// Lexical tokens of the Alloy configuration syntax (`syntax/token`).
pub mod token;

/// Lexical scanner / lexer (`syntax/scanner`).
pub mod scanner;

/// Abstract syntax tree (`syntax/ast`).
pub mod ast;

/// Recursive-descent parser (`syntax/parser`).
pub mod parser;
