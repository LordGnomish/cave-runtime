// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sift forensic-analysis layer — a deep-port of grafana/sift's automated
//! incident-investigation checks onto the cave-forensics event/evidence
//! data model.
//!
//! Upstream: grafana/sift (Apache-2.0) — the Grafana ML "Sift" diagnostic
//! assistant that runs a battery of checks (Error Pattern Logs, Kube
//! Crashes, correlated-series anomaly detection, …) and correlates the
//! interesting results into a ranked incident summary with a likely root
//! cause.
//!
//! cave-forensics already owns the raw signal (the [`crate::events`]
//! kernel-event stream + [`crate::case`] evidence store); Sift is the
//! analysis pass that turns that signal into findings, correlates them,
//! and ranks candidate root causes — exactly the consumer role the
//! crate plays in the runtime-security stack.

pub mod error_pattern;
