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

pub mod baseline;
pub mod config;
pub mod decoders;
pub mod detect;
pub mod finding;
pub mod git_walker;
pub mod protect;
pub mod report;
pub mod rule;
pub mod rule_pack;
pub mod stopwords;

pub use baseline::{Baseline, BaselineFile};
pub use config::{Allowlist, Config, ExtendConfig};
pub use decoders::{Decoder, DecoderChain, detect_with_decoders};
pub use detect::Detector;
pub use finding::{Finding, redact};
pub use protect::{ProtectOutcome, protect_staged_blobs, protect_staged_with};
pub use report::{write_csv, write_json, write_junit, write_sarif};
pub use rule::{Rule, builtin_rules};
pub use rule_pack::{load_pack_str, merge_packs, RulePackError, RulePackSpec, RuleSpec};
pub use stopwords::{default_stopwords, filter_with_stopwords};
