// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audit log — structured, append-only.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub operation: String,
    pub path: String,
    pub token_id: Option<String>,
    pub remote_addr: Option<String>,
    pub response_code: u16,
    pub error: Option<String>,
}

impl AuditEntry {
    pub fn new(
        operation: impl Into<String>,
        path: impl Into<String>,
        token_id: Option<String>,
        remote_addr: Option<String>,
        response_code: u16,
        error: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            operation: operation.into(),
            path: path.into(),
            token_id,
            remote_addr,
            response_code,
            error,
        }
    }

    pub fn ok(op: &str, path: &str, token: Option<String>) -> Self {
        Self::new(op, path, token, None, 200, None)
    }

    pub fn err(op: &str, path: &str, token: Option<String>, msg: &str, code: u16) -> Self {
        Self::new(op, path, token, None, code, Some(msg.to_string()))
    }
}

pub struct AuditLog {
    entries: Vec<AuditEntry>,
    max_entries: usize,
}

impl AuditLog {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries,
        }
    }

    pub fn append(&mut self, entry: AuditEntry) {
        self.entries.push(entry);
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
    }

    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    pub fn query_by_path(&self, path_prefix: &str) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.path.starts_with(path_prefix))
            .collect()
    }

    pub fn recent(&self, n: usize) -> Vec<&AuditEntry> {
        let start = self.entries.len().saturating_sub(n);
        self.entries[start..].iter().collect()
    }
}
