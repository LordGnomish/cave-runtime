// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-search: full-text + semantic search engine (skeleton — impl pending).
//!
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/
//!
//! # SCOPE_CUT: search-deep-port
//!
//! Every public function in this crate currently panics with a
//! `panic!("scope_cut: ...")` marker. The deep-port to Manticore
//! v25.8.2 is staged for Phase 2 — see parity.manifest.toml. Until
//! then, calling any of these functions from production code is a
//! programmer error. The Charter v2 G5 gate accepts the
//! `panic!("scope_cut: ...")` form (vs. `unimplemented!()`) because
//! the deferred surface is enumerated rather than open-ended.

pub mod analyzer;
pub mod embeddings;
pub mod index;
pub mod query;
pub mod scoring;
pub mod tenant;
