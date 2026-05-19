// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-gitleaks — Gitleaks-parity secret scanner.
//!
//! Upstream: <https://github.com/gitleaks/gitleaks> pinned `v8.29.1`.
//! Cave-gitleaks ports the in-scope MVP surface line-by-line in Rust:
//! TOML config loader, rule engine (regex + entropy + keyword pre-filter +
//! allowlist), finding redaction, JSON + SARIF reporters, working-tree and
//! git-history walkers built on `libgit2` via the `git2` crate.
//!
//! Out of scope for this scaffold (deferred to follow-up rays):
//! - `protect` subcommand (pre-commit / pre-push staged-blob enforcement)
//! - persistent baseline + redact files
//! - CSV / JUnit / template reporters
//! - decoded-payload chains (base64 / gzip auto-decode)
//! - GitHub Action integration

pub mod config;
pub mod detect;
pub mod finding;
pub mod git_walker;
pub mod report;
pub mod rule;

pub use config::{Allowlist, Config};
pub use detect::Detector;
pub use finding::{redact, Finding};
pub use report::{write_json, write_sarif};
pub use rule::{builtin_rules, Rule};
