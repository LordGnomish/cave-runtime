// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Data models for JSON API responses.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub sql: String,
    pub params: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub server_version: String,
    pub uptime: u64,
    pub databases: usize,
    pub tables_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub row_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub not_null: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainResponse {
    pub plan: String,
}

/// Request for a cost-based seqscan estimate against `costsize.c`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimateRequest {
    pub pages: u64,
    pub tuples: f64,
    /// Number of distinct values of the qualified column, if known
    /// (drives `eq_sel = 1/ndistinct`).
    pub ndistinct: Option<f64>,
}

/// A `costsize.c`-flavoured seqscan cost estimate plus the selectivity the
/// optimizer would assign an equality qual on the column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimateResponse {
    pub startup_cost: f64,
    pub total_cost: f64,
    pub eq_selectivity: f64,
    pub estimated_rows: f64,
}
