// SPDX-License-Identifier: AGPL-3.0-or-later
//! OPA REST API data models (v1 API).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── OPA Data API ─────────────────────────────────────────────────────────────

/// POST /v1/data/{path} request body.
#[derive(Debug, Default, Deserialize)]
pub struct DataQueryRequest {
    pub input: Option<serde_json::Value>,
}

/// Query parameters for data endpoints.
#[derive(Debug, Default, Deserialize)]
pub struct DataQueryParams {
    pub pretty: Option<bool>,
    pub provenance: Option<bool>,
    pub explain: Option<String>, // "notes" | "fails" | "full" | "debug"
    pub metrics: Option<bool>,
    pub instrument: Option<bool>,
    pub strict_builtin_errors: Option<bool>,
}

/// GET|POST /v1/data/{path} response.
#[derive(Debug, Serialize)]
pub struct DataResponse {
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ProvenanceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<Vec<TraceEvent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<ApiWarning>,
}

/// PUT /v1/data/{path} request.
#[derive(Debug, Deserialize)]
pub struct PutDataRequest {
    #[serde(flatten)]
    pub value: serde_json::Value,
}

/// PATCH /v1/data/{path} request (JSON Patch, RFC 6902).
pub type PatchDataRequest = Vec<JsonPatchOp>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonPatchOp {
    pub op: String, // "add" | "remove" | "replace" | "move" | "copy" | "test"
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

// ─── OPA Policy API ───────────────────────────────────────────────────────────

/// A stored Rego policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPolicy {
    pub id: String,
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ast: Option<serde_json::Value>,
}

/// GET /v1/policies response.
#[derive(Debug, Serialize)]
pub struct ListPoliciesResponse {
    pub result: Vec<StoredPolicy>,
}

/// GET /v1/policies/{id} response.
#[derive(Debug, Serialize)]
pub struct GetPolicyResponse {
    pub result: StoredPolicy,
}

/// PUT /v1/policies/{id} request body (raw Rego text).
pub type PutPolicyRequest = String;

/// Response after creating/updating a policy.
#[derive(Debug, Serialize)]
pub struct PutPolicyResponse {
    pub result: StoredPolicy,
}

// ─── OPA Query API ────────────────────────────────────────────────────────────

/// GET /v1/query?q=... query parameters.
#[derive(Debug, Deserialize)]
pub struct AdHocQueryParams {
    pub q: String,
    pub pretty: Option<bool>,
    pub explain: Option<String>,
    pub metrics: Option<bool>,
}

/// POST /v1/query request body.
#[derive(Debug, Deserialize)]
pub struct AdHocQueryRequest {
    pub query: String,
    pub input: Option<serde_json::Value>,
    pub pretty: Option<bool>,
    pub explain: Option<String>,
    pub metrics: Option<bool>,
}

/// GET|POST /v1/query response.
#[derive(Debug, Serialize)]
pub struct QueryResponse {
    /// List of variable bindings, one per successful evaluation.
    pub result: Vec<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<Vec<TraceEvent>>,
}

// ─── OPA Compile (Partial Evaluation) API ─────────────────────────────────────

/// POST /v1/compile request.
#[derive(Debug, Deserialize)]
pub struct CompileRequest {
    pub query: String,
    pub input: Option<serde_json::Value>,
    /// Unknowns to treat as opaque during partial evaluation.
    pub unknowns: Option<Vec<String>>,
    pub options: Option<CompileOptions>,
}

#[derive(Debug, Deserialize)]
pub struct CompileOptions {
    pub disallow_unknown_functions: Option<bool>,
}

/// POST /v1/compile response.
#[derive(Debug, Serialize)]
pub struct CompileResponse {
    /// Remaining partial queries after evaluation.
    pub result: CompileResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct CompileResult {
    pub queries: Vec<Vec<serde_json::Value>>,
    pub support: Vec<serde_json::Value>,
}

// ─── OPA Status & Health API ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundles: Option<HashMap<String, BundleStatus>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugins: Option<HashMap<String, PluginStatus>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct BundleStatus {
    pub name: String,
    pub active_revision: Option<String>,
    pub last_successful_activation: Option<DateTime<Utc>>,
    pub last_successful_download: Option<DateTime<Utc>>,
    pub last_successful_request: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PluginStatus {
    pub state: String, // "OK" | "WARN" | "ERR" | "NOT_READY"
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub result: OpaStatus,
}

#[derive(Debug, Serialize)]
pub struct OpaStatus {
    pub labels: HashMap<String, String>,
    pub bundles: HashMap<String, BundleStatus>,
    pub plugins: HashMap<String, PluginStatus>,
    pub metrics: Option<serde_json::Value>,
}

// ─── Provenance & Trace ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ProvenanceInfo {
    pub version: String,
    pub build_commit: String,
    pub build_timestamp: String,
    pub build_hostname: String,
    pub bundles: HashMap<String, BundleProvenance>,
}

#[derive(Debug, Serialize)]
pub struct BundleProvenance {
    pub revision: String,
}

#[derive(Debug, Serialize)]
pub struct TraceEvent {
    pub op: String, // "enter" | "redo" | "eval" | "fail" | "exit" | "unify" | "save" | "note"
    pub query_id: u64,
    pub parent_id: u64,
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locals: Option<Vec<Local>>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Local {
    pub name: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ApiWarning {
    pub code: String,
    pub message: String,
}

// ─── Decision Log ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLogEntry {
    pub decision_id: String,
    pub path: String,
    pub input: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub requested_by: String,
    pub timestamp: DateTime<Utc>,
    pub metrics: Option<serde_json::Value>,
    pub bundle_name: Option<String>,
    pub revision: Option<String>,
}

// ─── Error response ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<PolicyViolation>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PolicyViolation {
    pub policy: String,
    pub rule: String,
    pub message: String,
    pub resource: Option<String>,
    pub severity: Option<String>,
}
