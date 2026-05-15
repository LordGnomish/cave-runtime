// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MongoDB oplog connector (change streams).
//!
//! Cite: debezium-connector-mongodb v3.5.0.Final
//! `MongoDbConnector.java`, `MongoDbStreamingChangeEventSource.java`.
//! cave models the oplog operation byte (`o = i | u | d | c | n`)
//! and the ResumeToken-like checkpoint primitive.

use crate::connector::{ConnectorState, SourceConnector};
use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};

/// Cite: MongoDB `oplog.rs` `op` field — `i` insert, `u` update,
/// `d` delete, `c` command (DDL-ish), `n` noop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OplogOp { Insert, Update, Delete, Command, Noop }

impl OplogOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Insert  => "i", Self::Update => "u", Self::Delete  => "d",
            Self::Command => "c", Self::Noop   => "n",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "i" | "insert"  => Some(Self::Insert),
            "u" | "update"  => Some(Self::Update),
            "d" | "delete"  => Some(Self::Delete),
            "c" | "command" => Some(Self::Command),
            "n" | "noop"    => Some(Self::Noop),
            _ => None,
        }
    }
}

/// Resume token for change-stream tailing. Cite: MongoDB change
/// streams docs — opaque BSON document; cave models it as an opaque
/// hex blob with a monotonic ordering hint via `cluster_time`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResumeToken {
    pub data_hex: String,
    pub cluster_time_secs: u32,
    pub increment: u32,
}

impl ResumeToken {
    pub fn new(data_hex: impl Into<String>, cluster_time_secs: u32, increment: u32) -> Self {
        Self { data_hex: data_hex.into(), cluster_time_secs, increment }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OplogEvent {
    pub op: OplogOp,
    pub namespace: String,         // "<db>.<collection>"
    pub document_id: serde_json::Value,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub cluster_time_secs: u32,
    pub resume_token: ResumeToken,
}

impl OplogEvent {
    /// Cite: debezium-connector-mongodb namespace parsing — `<db>.<col>`
    /// where the first `.` is the separator (collections may contain
    /// further dots, e.g. `system.indexes`).
    pub fn db_and_collection(&self) -> CdcResult<(&str, &str)> {
        let (db, col) = self.namespace.split_once('.')
            .ok_or_else(|| CdcError::InvalidConfig(
                format!("invalid namespace '{}': expected '<db>.<collection>'", self.namespace),
            ))?;
        if db.is_empty() || col.is_empty() {
            return Err(CdcError::InvalidConfig(
                format!("invalid namespace '{}': empty db or collection", self.namespace),
            ));
        }
        Ok((db, col))
    }
}

#[derive(Debug, Clone)]
pub struct MongoDbConnector {
    pub name: String,
    pub tenant_id: String,
    pub replica_set: String,
    /// `db.collection` patterns. Empty list = all.
    pub include_namespaces: Vec<String>,
    state: ConnectorState,
    last_resume_token: Option<ResumeToken>,
}

impl MongoDbConnector {
    pub fn new(
        name: impl Into<String>,
        tenant_id: impl Into<String>,
        replica_set: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            tenant_id: tenant_id.into(),
            replica_set: replica_set.into(),
            include_namespaces: Vec::new(),
            state: ConnectorState::Initial,
            last_resume_token: None,
        }
    }

    pub fn last_resume_token(&self) -> Option<&ResumeToken> { self.last_resume_token.as_ref() }

    /// Cite: debezium-connector-mongodb `MongoDbStreamingChangeEventSource`
    /// — when the include list is non-empty, only namespaces whose
    /// `<db>.<col>` exactly matches one of the entries pass through.
    pub fn should_emit(&self, namespace: &str) -> bool {
        self.include_namespaces.is_empty()
            || self.include_namespaces.iter().any(|p| p == namespace)
    }

    /// Resume tokens MUST advance monotonically; an older one indicates
    /// a desynchronised checkpoint and is rejected.
    pub fn record_resume_token(&mut self, token: ResumeToken) -> CdcResult<()> {
        if let Some(prev) = &self.last_resume_token {
            if token < *prev {
                return Err(CdcError::InvalidConfig(format!(
                    "resume token regression: {:?} -> {:?}", prev, token,
                )));
            }
        }
        self.last_resume_token = Some(token);
        Ok(())
    }
}

impl SourceConnector for MongoDbConnector {
    fn name(&self) -> &str { &self.name }
    fn tenant_id(&self) -> &str { &self.tenant_id }
    fn state(&self) -> ConnectorState { self.state }

    fn validate(&self) -> CdcResult<()> {
        if self.tenant_id.trim().is_empty() {
            return Err(CdcError::InvalidConfig("tenant_id must be non-empty".into()));
        }
        if self.replica_set.trim().is_empty() {
            return Err(CdcError::InvalidConfig("replica_set must be non-empty".into()));
        }
        Ok(())
    }

    fn start(&mut self) -> CdcResult<()> {
        if self.state == ConnectorState::Streaming {
            return Err(CdcError::AlreadyRunning);
        }
        self.validate()?;
        self.state = ConnectorState::Streaming;
        Ok(())
    }

    fn stop(&mut self) -> CdcResult<()> {
        self.state = ConnectorState::Stopped;
        Ok(())
    }
}
