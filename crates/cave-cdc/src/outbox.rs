// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transactional outbox event router.
//!
//! Cite: debezium-core `OutboxEventRouter` (SMT) +
//! `debezium-connector-postgres OutboxEventRouterIT`. Pattern: the
//! application writes a row to an `outbox` table inside the same DB
//! transaction as its business mutation, and the CDC pipeline turns
//! each row into a Kafka topic message keyed by `aggregate_id`.

use crate::error::{CdcError, CdcResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEntry {
    /// Cite: debezium `OutboxEventRouter` `id` field — UUID by default.
    pub id: String,
    pub tenant_id: String,
    /// Cite: `aggregatetype` field maps onto the destination topic.
    pub aggregate_type: String,
    /// Cite: `aggregateid` field — used as the Kafka message key.
    pub aggregate_id: String,
    /// Cite: `type` field — application event type (e.g. `OrderCreated`).
    pub event_type: String,
    /// Cite: `payload` field — opaque event body.
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl OutboxEntry {
    /// Cite: debezium `OutboxEventRouter::apply` — every entry must
    /// have a non-empty id, aggregate_type, aggregate_id, and a
    /// payload (events with an empty payload are deletes; cave keeps
    /// the explicit invariant for the scaffold).
    pub fn validate(&self) -> CdcResult<()> {
        if self.id.trim().is_empty() {
            return Err(CdcError::InvalidConfig("outbox id must be non-empty".into()));
        }
        if self.aggregate_type.trim().is_empty() {
            return Err(CdcError::InvalidConfig("aggregate_type must be non-empty".into()));
        }
        if self.aggregate_id.trim().is_empty() {
            return Err(CdcError::InvalidConfig("aggregate_id must be non-empty".into()));
        }
        if self.tenant_id.trim().is_empty() {
            return Err(CdcError::InvalidConfig("tenant_id must be non-empty".into()));
        }
        Ok(())
    }
}

/// Cite: debezium `OutboxEventRouter` — the router maps each entry to
/// a topic + key + headers. It MUST de-duplicate by `id` so retries
/// are idempotent at the destination.
#[derive(Debug, Default)]
pub struct OutboxEventRouter {
    pub tenant_id: String,
    pub topic_prefix: String,
    seen_ids: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxRouted {
    pub topic: String,
    pub key: String,
    pub headers: std::collections::HashMap<String, String>,
    pub value: serde_json::Value,
}

impl OutboxEventRouter {
    pub fn new(tenant_id: impl Into<String>, topic_prefix: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            topic_prefix: topic_prefix.into(),
            seen_ids: HashSet::new(),
        }
    }

    /// Cite: debezium `OutboxEventRouter::apply` — when the same `id`
    /// is replayed, the router rejects with a duplicate-id error so
    /// upstream offsets can be advanced without double-publishing.
    pub fn route(&mut self, entry: &OutboxEntry) -> CdcResult<OutboxRouted> {
        entry.validate()?;
        if entry.tenant_id != self.tenant_id {
            return Err(CdcError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: entry.tenant_id.clone(),
            });
        }
        if !self.seen_ids.insert(entry.id.clone()) {
            return Err(CdcError::DuplicateOutboxEventId(entry.id.clone()));
        }
        let topic = format!("{}.{}.{}",
            self.topic_prefix, self.tenant_id, entry.aggregate_type);

        let mut headers = std::collections::HashMap::new();
        headers.insert("id".into(), entry.id.clone());
        headers.insert("eventType".into(), entry.event_type.clone());
        headers.insert("tenant_id".into(), self.tenant_id.clone());

        Ok(OutboxRouted {
            topic,
            key: entry.aggregate_id.clone(),
            headers,
            value: entry.payload.clone(),
        })
    }

    pub fn seen(&self, id: &str) -> bool { self.seen_ids.contains(id) }
    pub fn dedupe_size(&self) -> usize { self.seen_ids.len() }

    /// Operator-driven cleanup. Cite: debezium recommends pruning the
    /// outbox table after the CDC pipeline confirms delivery; the
    /// router's dedupe set follows the same retention.
    pub fn forget(&mut self, id: &str) -> bool {
        self.seen_ids.remove(id)
    }
}
