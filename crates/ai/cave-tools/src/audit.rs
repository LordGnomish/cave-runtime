// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Append-only, tamper-evident invocation history.
//!
//! Every tool call can be recorded as an [`InvocationRecord`] carrying the
//! caller, the tool, a SHA-256 hash of the arguments (never the raw args —
//! they may hold secrets), the outcome, and a UTC timestamp. Records form a
//! hash chain (`entry_hash = sha256(prev_hash ‖ fields)`), so any
//! after-the-fact mutation is detectable via [`AuditLog::verify`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use serde_json::Value;

use crate::error::ToolError;
use crate::tool::ToolResult;

/// 64-hex genesis hash linking the first record.
const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Classification of how an invocation ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum RecordOutcome {
    /// Tool ran and returned a non-error result.
    Success,
    /// Tool ran but reported a domain error (`ToolResult.is_error`).
    ToolError(String),
    /// The call was rejected before/at execution (not-found, invalid args,
    /// permission denied, sandbox violation, protocol error).
    Rejected(String),
}

impl RecordOutcome {
    /// Derive an outcome from the result of an invocation.
    pub fn from_result(res: &crate::Result<ToolResult>) -> Self {
        match res {
            Ok(r) if r.is_error => RecordOutcome::ToolError(r.text_output()),
            Ok(_) => RecordOutcome::Success,
            Err(e) => RecordOutcome::Rejected(format!("{}: {}", e.code(), e)),
        }
    }

    fn tag(&self) -> &'static str {
        match self {
            RecordOutcome::Success => "success",
            RecordOutcome::ToolError(_) => "tool_error",
            RecordOutcome::Rejected(_) => "rejected",
        }
    }

    fn detail(&self) -> &str {
        match self {
            RecordOutcome::Success => "",
            RecordOutcome::ToolError(d) | RecordOutcome::Rejected(d) => d,
        }
    }
}

/// One immutable audit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub id: String,
    pub user: String,
    pub tool: String,
    /// SHA-256 (hex) of the canonical JSON arguments.
    pub args_hash: String,
    pub outcome: RecordOutcome,
    pub ts: DateTime<Utc>,
    /// Hash of the previous entry (genesis for the first).
    pub prev_hash: String,
    /// Hash of this entry's fields chained onto `prev_hash`.
    pub entry_hash: String,
}

impl InvocationRecord {
    /// Recompute this record's `entry_hash` from its fields and `prev_hash`.
    fn compute_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.prev_hash.as_bytes());
        h.update(b"\x1f");
        h.update(self.id.as_bytes());
        h.update(b"\x1f");
        h.update(self.user.as_bytes());
        h.update(b"\x1f");
        h.update(self.tool.as_bytes());
        h.update(b"\x1f");
        h.update(self.args_hash.as_bytes());
        h.update(b"\x1f");
        h.update(self.outcome.tag().as_bytes());
        h.update(b"\x1f");
        h.update(self.outcome.detail().as_bytes());
        h.update(b"\x1f");
        h.update(self.ts.to_rfc3339().as_bytes());
        hex::encode(h.finalize())
    }
}

/// Hash-chained append-only log of tool invocations.
#[derive(Debug, Clone, Default)]
pub struct AuditLog {
    entries: Vec<InvocationRecord>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[InvocationRecord] {
        &self.entries
    }

    /// Mutable access — intended for tests / administrative redaction.
    /// Mutating an entry invalidates the hash chain (see [`verify`](Self::verify)).
    pub fn entries_mut(&mut self) -> &mut [InvocationRecord] {
        &mut self.entries
    }

    fn hash_args(args: &Value) -> String {
        // Canonical form: serde_json sorts object keys when the `preserve_order`
        // feature is off (the workspace default), giving a stable encoding.
        let canon = serde_json::to_string(args).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(canon.as_bytes());
        hex::encode(h.finalize())
    }

    /// Append a record and return a reference to it.
    pub fn record(
        &mut self,
        user: impl Into<String>,
        tool: impl Into<String>,
        args: &Value,
        outcome: RecordOutcome,
    ) -> &InvocationRecord {
        let prev_hash = self
            .entries
            .last()
            .map(|e| e.entry_hash.clone())
            .unwrap_or_else(|| GENESIS.to_string());
        let mut rec = InvocationRecord {
            id: uuid::Uuid::new_v4().to_string(),
            user: user.into(),
            tool: tool.into(),
            args_hash: Self::hash_args(args),
            outcome,
            ts: Utc::now(),
            prev_hash,
            entry_hash: String::new(),
        };
        rec.entry_hash = rec.compute_hash();
        self.entries.push(rec);
        self.entries.last().unwrap()
    }

    /// All records for a given tool, oldest first.
    pub fn by_tool<'a>(&'a self, tool: &'a str) -> impl Iterator<Item = &'a InvocationRecord> {
        self.entries.iter().filter(move |e| e.tool == tool)
    }

    /// All records for a given user, oldest first.
    pub fn by_user<'a>(&'a self, user: &'a str) -> impl Iterator<Item = &'a InvocationRecord> {
        self.entries.iter().filter(move |e| e.user == user)
    }

    /// Verify the hash chain end-to-end. Returns `Ok(())` if intact, or
    /// `Err(index)` of the first entry whose recomputed hash or back-link
    /// does not match.
    pub fn verify(&self) -> Result<(), usize> {
        let mut expected_prev = GENESIS.to_string();
        for (i, e) in self.entries.iter().enumerate() {
            if e.prev_hash != expected_prev {
                return Err(i);
            }
            if e.compute_hash() != e.entry_hash {
                return Err(i);
            }
            expected_prev = e.entry_hash.clone();
        }
        Ok(())
    }
}

impl From<&ToolError> for RecordOutcome {
    fn from(e: &ToolError) -> Self {
        RecordOutcome::Rejected(format!("{}: {}", e.code(), e))
    }
}
