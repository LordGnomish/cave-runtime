// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Output types for a parity report.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Full parity report for one CAVE module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityReport {
    pub module: String,
    /// Human-readable upstream reference, e.g. `"etcd-io/etcd @ v3.5.13"`.
    pub upstream_ref: String,
    pub measured_at: DateTime<Utc>,
    pub file_parity: ParityMetric,
    pub function_parity: ParityMetric,
    pub test_parity: ParityMetric,
    pub surface_parity: ParityMetric,
    /// Weighted average of the four metrics (0.0 – 1.0).
    pub overall: f32,
    /// Number of `todo!` / `unimplemented!` occurrences in the source tree.
    pub stubs_detected: u32,
    /// List of upstream items that are not yet covered locally.
    pub gaps: Vec<GapItem>,
}

/// A single parity dimension (matched / total items and derived score).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityMetric {
    /// Fraction of upstream items covered (0.0 – 1.0).
    pub score: f32,
    pub matched: u32,
    pub total: u32,
}

/// One gap — an upstream item that is not yet covered locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapItem {
    pub kind: GapKind,
    /// Name / path of the upstream item.
    pub upstream: String,
    /// Expected local name / path (from the manifest).
    pub local: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    File,
    Function,
    Test,
    Surface,
}
