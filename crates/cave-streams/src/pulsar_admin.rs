// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Apache Pulsar admin / control-plane parity — tenants, namespaces,
//! topics, subscriptions, producers, consumers and the pulsar-admin REST
//! API surface.
//!
//! This module is the **admin half** of the Pulsar split.  The **wire half**
//! (binary protocol on port 6650 — CONNECT/PRODUCER/SEND/SUBSCRIBE/FLOW/
//! MESSAGE/ACK) lives in [`crate::pulsar_wire`] and dispatches via
//! [`crate::pulsar_dispatch`]; canonical addressing (`persistent://t/ns/l`)
//! is parsed by [`crate::pulsar_topic::TopicName`] and the wire-side tenant
//! registry lives in [`crate::tenant`].  The two halves intentionally use
//! independent type families because they serve different surfaces — the
//! wire layer needs a minimal lookup registry, the admin layer needs a
//! rich CRUD model with retention, TTL, quotas, cursors, etc.
//!
//! Mirrors the read- and ops-side of the Pulsar admin REST API
//! (https://pulsar.apache.org/admin-rest-api/) and the canonical pub/sub model:
//!
//! - **Tenant** — top-level multi-tenancy boundary
//! - **Namespace** — `tenant/namespace`, owns retention/policy defaults
//! - **Topic**     — `persistent://tenant/namespace/topic` (or `non-persistent://...`)
//! - **Subscription** — exclusive / shared / failover / key-shared
//! - **Producer**  — appends `PulsarMessage` records to a topic
//! - **Consumer**  — reads via a `Subscription`, acknowledges by `MessageId`
//!
//! Storage is in-memory and thread-safe (RwLock-guarded BTreeMaps).  This
//! gives a Pulsar-compatible control plane without pulling in the full
//! BookKeeper storage layer; persistence can be layered on top later.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use uuid::Uuid;

// ───────────────────────────── Errors ────────────────────────────────────────

#[derive(Debug, Error)]
pub enum PulsarError {
    #[error("tenant `{0}` not found")] TenantNotFound(String),
    #[error("namespace `{0}` not found")] NamespaceNotFound(String),
    #[error("topic `{0}` not found")] TopicNotFound(String),
    #[error("subscription `{0}` not found")] SubscriptionNotFound(String),
    #[error("topic `{0}` already exists")] TopicAlreadyExists(String),
    #[error("subscription `{0}` already exists")] SubscriptionAlreadyExists(String),
    #[error("invalid topic name: {0}")] InvalidTopic(String),
    #[error("invalid subscription type")] InvalidSubscriptionType,
    #[error("subscription is at end-of-stream")] EndOfStream,
}

pub type PulsarResult<T> = Result<T, PulsarError>;

// ───────────────────────────── Models ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tenant {
    pub name: String,
    pub admin_roles: Vec<String>,
    pub allowed_clusters: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Namespace {
    pub tenant: String,
    pub name: String, // bare namespace name (no tenant prefix)
    pub retention_minutes: u64,
    pub retention_size_mb: u64,
    pub message_ttl_seconds: Option<u64>,
    pub deduplication_enabled: bool,
    pub backlog_quota_bytes: Option<u64>,
    pub max_producers_per_topic: u32,
    pub max_consumers_per_topic: u32,
    pub max_subscriptions_per_topic: u32,
    pub created_at: DateTime<Utc>,
}

impl Namespace {
    pub fn fqn(&self) -> String {
        format!("{}/{}", self.tenant, self.name)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TopicDomain {
    Persistent,
    NonPersistent,
}

impl TopicDomain {
    pub fn scheme(self) -> &'static str {
        match self {
            TopicDomain::Persistent => "persistent",
            TopicDomain::NonPersistent => "non-persistent",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Topic {
    pub domain: TopicDomain,
    pub tenant: String,
    pub namespace: String,
    pub name: String,
    pub partitions: u32, // 0 = non-partitioned
    pub created_at: DateTime<Utc>,
}

impl Topic {
    pub fn fqn(&self) -> String {
        format!(
            "{}://{}/{}/{}",
            self.domain.scheme(),
            self.tenant,
            self.namespace,
            self.name
        )
    }

    pub fn parse(fqn: &str) -> PulsarResult<(TopicDomain, String, String, String)> {
        // persistent://tenant/namespace/topic[-partition-N]
        let (scheme, rest) = fqn.split_once("://")
            .ok_or_else(|| PulsarError::InvalidTopic(fqn.to_string()))?;
        let domain = match scheme {
            "persistent" => TopicDomain::Persistent,
            "non-persistent" => TopicDomain::NonPersistent,
            _ => return Err(PulsarError::InvalidTopic(fqn.to_string())),
        };
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 3 {
            return Err(PulsarError::InvalidTopic(fqn.to_string()));
        }
        Ok((domain, parts[0].to_string(), parts[1].to_string(), parts[2].to_string()))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum SubscriptionType {
    Exclusive,
    Shared,
    Failover,
    KeyShared,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InitialPosition {
    Earliest,
    Latest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub topic: String, // FQN
    pub name: String,
    pub sub_type: SubscriptionType,
    pub initial_position: InitialPosition,
    pub cursor_pos: i64, // 0-based offset of next message to deliver
    pub backlog: i64,    // unacked count
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MessageId {
    pub ledger_id: u64,
    pub entry_id: u64,
    pub partition: i32,
    pub batch_index: i32,
}

impl MessageId {
    pub const EARLIEST: MessageId = MessageId { ledger_id: 0, entry_id: 0, partition: -1, batch_index: -1 };
    pub const LATEST: MessageId = MessageId { ledger_id: u64::MAX, entry_id: u64::MAX, partition: -1, batch_index: -1 };
    pub fn from_offset(offset: u64) -> Self {
        Self { ledger_id: 0, entry_id: offset, partition: -1, batch_index: -1 }
    }
    pub fn offset(&self) -> u64 {
        self.entry_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulsarMessage {
    pub id: MessageId,
    pub key: Option<String>,
    pub value: Vec<u8>,
    pub properties: HashMap<String, String>,
    pub publish_time: DateTime<Utc>,
    pub event_time: Option<DateTime<Utc>>,
    pub producer_name: Option<String>,
    pub sequence_id: u64,
}

impl PulsarMessage {
    pub fn new(value: Vec<u8>) -> Self {
        Self {
            id: MessageId::EARLIEST,
            key: None,
            value,
            properties: HashMap::new(),
            publish_time: Utc::now(),
            event_time: None,
            producer_name: None,
            sequence_id: 0,
        }
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_property(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.properties.insert(k.into(), v.into());
        self
    }
}

// ───────────────────────────── Topic store ──────────────────────────────────

#[derive(Debug, Default)]
struct TopicStore {
    log: RwLock<Vec<PulsarMessage>>, // append-only, indexed by entry_id
    next_entry_id: AtomicU64,
    /// subscription name → cursor position (next message to deliver)
    cursors: RwLock<BTreeMap<String, Subscription>>,
    /// per-subscription unacked set: msg id → ack pending
    unacked: RwLock<BTreeMap<String, BTreeMap<u64, ()>>>,
    /// per-subscription dead-letter messages
    dlq: RwLock<BTreeMap<String, Vec<PulsarMessage>>>,
    /// counts
    producer_count: AtomicU64,
}

impl TopicStore {
    fn append(&self, mut msg: PulsarMessage) -> MessageId {
        let entry = self.next_entry_id.fetch_add(1, Ordering::SeqCst);
        msg.id = MessageId::from_offset(entry);
        let mut log = self.log.write().unwrap();
        log.push(msg.clone());
        msg.id
    }

    fn len(&self) -> u64 {
        self.next_entry_id.load(Ordering::SeqCst)
    }
}

// ───────────────────────────── Cluster ──────────────────────────────────────

/// Top-level Pulsar control-plane state.
#[derive(Debug, Default)]
pub struct PulsarAdminCluster {
    tenants: RwLock<BTreeMap<String, Tenant>>,
    namespaces: RwLock<BTreeMap<String, Namespace>>, // key = "tenant/namespace"
    topics: RwLock<BTreeMap<String, Arc<TopicMeta>>>, // key = topic FQN
    /// Stable name → topic store, keyed by FQN for quick lookup
    stores: RwLock<BTreeMap<String, Arc<TopicStore>>>,
}

#[derive(Debug)]
struct TopicMeta {
    pub topic: Topic,
}

impl PulsarAdminCluster {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    // ── Tenants ──────────────────────────────────────────────────────────

    pub fn create_tenant(&self, name: &str) -> Tenant {
        let t = Tenant {
            name: name.to_string(),
            admin_roles: vec![],
            allowed_clusters: vec!["standalone".into()],
            created_at: Utc::now(),
        };
        self.tenants.write().unwrap().insert(name.to_string(), t.clone());
        t
    }

    pub fn list_tenants(&self) -> Vec<Tenant> {
        self.tenants.read().unwrap().values().cloned().collect()
    }

    pub fn get_tenant(&self, name: &str) -> PulsarResult<Tenant> {
        self.tenants.read().unwrap().get(name).cloned()
            .ok_or_else(|| PulsarError::TenantNotFound(name.to_string()))
    }

    pub fn delete_tenant(&self, name: &str) -> PulsarResult<()> {
        // Cascade: remove tenant's namespaces (and their topics).
        let names: Vec<String> = self.namespaces.read().unwrap().values()
            .filter(|ns| ns.tenant == name).map(|ns| ns.fqn()).collect();
        for ns_fqn in names {
            let _ = self.delete_namespace(&ns_fqn);
        }
        if self.tenants.write().unwrap().remove(name).is_none() {
            return Err(PulsarError::TenantNotFound(name.to_string()));
        }
        Ok(())
    }

    // ── Namespaces ───────────────────────────────────────────────────────

    pub fn create_namespace(&self, tenant: &str, name: &str) -> PulsarResult<Namespace> {
        self.get_tenant(tenant)?;
        let ns = Namespace {
            tenant: tenant.to_string(),
            name: name.to_string(),
            retention_minutes: 0,
            retention_size_mb: 0,
            message_ttl_seconds: None,
            deduplication_enabled: false,
            backlog_quota_bytes: None,
            max_producers_per_topic: 0,
            max_consumers_per_topic: 0,
            max_subscriptions_per_topic: 0,
            created_at: Utc::now(),
        };
        self.namespaces.write().unwrap().insert(ns.fqn(), ns.clone());
        Ok(ns)
    }

    pub fn list_namespaces(&self, tenant: &str) -> Vec<String> {
        self.namespaces.read().unwrap().values()
            .filter(|ns| ns.tenant == tenant)
            .map(|ns| ns.fqn())
            .collect()
    }

    pub fn get_namespace(&self, fqn: &str) -> PulsarResult<Namespace> {
        self.namespaces.read().unwrap().get(fqn).cloned()
            .ok_or_else(|| PulsarError::NamespaceNotFound(fqn.to_string()))
    }

    pub fn set_namespace_retention(&self, fqn: &str, minutes: u64, size_mb: u64) -> PulsarResult<()> {
        let mut g = self.namespaces.write().unwrap();
        let ns = g.get_mut(fqn).ok_or_else(|| PulsarError::NamespaceNotFound(fqn.to_string()))?;
        ns.retention_minutes = minutes;
        ns.retention_size_mb = size_mb;
        Ok(())
    }

    pub fn set_namespace_ttl(&self, fqn: &str, ttl_seconds: u64) -> PulsarResult<()> {
        let mut g = self.namespaces.write().unwrap();
        let ns = g.get_mut(fqn).ok_or_else(|| PulsarError::NamespaceNotFound(fqn.to_string()))?;
        ns.message_ttl_seconds = Some(ttl_seconds);
        Ok(())
    }

    pub fn delete_namespace(&self, fqn: &str) -> PulsarResult<()> {
        // Cascade: delete topics in namespace.
        let topics: Vec<String> = {
            let parts: Vec<&str> = fqn.split('/').collect();
            self.topics.read().unwrap().values()
                .filter(|t| parts.len() == 2 && t.topic.tenant == parts[0] && t.topic.namespace == parts[1])
                .map(|t| t.topic.fqn())
                .collect()
        };
        for t in topics {
            let _ = self.delete_topic(&t);
        }
        if self.namespaces.write().unwrap().remove(fqn).is_none() {
            return Err(PulsarError::NamespaceNotFound(fqn.to_string()));
        }
        Ok(())
    }

    // ── Topics ───────────────────────────────────────────────────────────

    pub fn create_topic(&self, fqn: &str, partitions: u32) -> PulsarResult<Topic> {
        let (domain, tenant, namespace, name) = Topic::parse(fqn)?;
        let ns_key = format!("{tenant}/{namespace}");
        self.get_namespace(&ns_key)?;
        let mut topics = self.topics.write().unwrap();
        if topics.contains_key(fqn) {
            return Err(PulsarError::TopicAlreadyExists(fqn.to_string()));
        }
        let t = Topic {
            domain,
            tenant,
            namespace,
            name,
            partitions,
            created_at: Utc::now(),
        };
        topics.insert(fqn.to_string(), Arc::new(TopicMeta { topic: t.clone() }));
        self.stores.write().unwrap().insert(fqn.to_string(), Arc::new(TopicStore::default()));
        Ok(t)
    }

    pub fn delete_topic(&self, fqn: &str) -> PulsarResult<()> {
        if self.topics.write().unwrap().remove(fqn).is_none() {
            return Err(PulsarError::TopicNotFound(fqn.to_string()));
        }
        self.stores.write().unwrap().remove(fqn);
        Ok(())
    }

    pub fn list_topics(&self, namespace_fqn: &str) -> Vec<String> {
        let parts: Vec<&str> = namespace_fqn.split('/').collect();
        self.topics.read().unwrap().values()
            .filter(|t| parts.len() == 2 && t.topic.tenant == parts[0] && t.topic.namespace == parts[1])
            .map(|t| t.topic.fqn())
            .collect()
    }

    pub fn topic_exists(&self, fqn: &str) -> bool {
        self.topics.read().unwrap().contains_key(fqn)
    }

    pub fn topic_stats(&self, fqn: &str) -> PulsarResult<TopicStats> {
        let store = self.stores.read().unwrap().get(fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(fqn.to_string()))?;
        let cursors = store.cursors.read().unwrap();
        let subs: Vec<SubscriptionStats> = cursors.values().map(|s| SubscriptionStats {
            name: s.name.clone(),
            sub_type: s.sub_type,
            backlog: s.backlog,
            cursor: s.cursor_pos,
        }).collect();
        Ok(TopicStats {
            topic: fqn.to_string(),
            messages_in: store.len() as i64,
            producers: store.producer_count.load(Ordering::SeqCst) as i64,
            subscriptions: subs,
        })
    }

    // ── Subscriptions ────────────────────────────────────────────────────

    pub fn create_subscription(
        &self,
        topic_fqn: &str,
        sub_name: &str,
        sub_type: SubscriptionType,
        initial_position: InitialPosition,
    ) -> PulsarResult<Subscription> {
        let store = self.stores.read().unwrap().get(topic_fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(topic_fqn.to_string()))?;
        let cursor_pos = match initial_position {
            InitialPosition::Earliest => 0,
            InitialPosition::Latest => store.len() as i64,
        };
        let mut cursors = store.cursors.write().unwrap();
        if cursors.contains_key(sub_name) {
            return Err(PulsarError::SubscriptionAlreadyExists(sub_name.to_string()));
        }
        let sub = Subscription {
            topic: topic_fqn.to_string(),
            name: sub_name.to_string(),
            sub_type,
            initial_position,
            cursor_pos,
            backlog: store.len() as i64 - cursor_pos,
            created_at: Utc::now(),
        };
        cursors.insert(sub_name.to_string(), sub.clone());
        store.unacked.write().unwrap().insert(sub_name.to_string(), BTreeMap::new());
        Ok(sub)
    }

    pub fn list_subscriptions(&self, topic_fqn: &str) -> PulsarResult<Vec<Subscription>> {
        let store = self.stores.read().unwrap().get(topic_fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(topic_fqn.to_string()))?;
        Ok(store.cursors.read().unwrap().values().cloned().collect())
    }

    pub fn delete_subscription(&self, topic_fqn: &str, sub_name: &str) -> PulsarResult<()> {
        let store = self.stores.read().unwrap().get(topic_fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(topic_fqn.to_string()))?;
        if store.cursors.write().unwrap().remove(sub_name).is_none() {
            return Err(PulsarError::SubscriptionNotFound(sub_name.to_string()));
        }
        store.unacked.write().unwrap().remove(sub_name);
        Ok(())
    }

    pub fn skip_all(&self, topic_fqn: &str, sub_name: &str) -> PulsarResult<()> {
        let store = self.stores.read().unwrap().get(topic_fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(topic_fqn.to_string()))?;
        let len = store.len() as i64;
        let mut cursors = store.cursors.write().unwrap();
        let sub = cursors.get_mut(sub_name)
            .ok_or_else(|| PulsarError::SubscriptionNotFound(sub_name.to_string()))?;
        sub.cursor_pos = len;
        sub.backlog = 0;
        Ok(())
    }

    pub fn reset_cursor(&self, topic_fqn: &str, sub_name: &str, position: MessageId) -> PulsarResult<()> {
        let store = self.stores.read().unwrap().get(topic_fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(topic_fqn.to_string()))?;
        let len = store.len() as i64;
        let mut cursors = store.cursors.write().unwrap();
        let sub = cursors.get_mut(sub_name)
            .ok_or_else(|| PulsarError::SubscriptionNotFound(sub_name.to_string()))?;
        let new_pos = if position == MessageId::EARLIEST {
            0
        } else if position == MessageId::LATEST {
            len
        } else {
            (position.entry_id as i64).clamp(0, len)
        };
        sub.cursor_pos = new_pos;
        sub.backlog = len - new_pos;
        Ok(())
    }

    // ── Producer / publish ───────────────────────────────────────────────

    pub fn open_producer(&self, topic_fqn: &str) -> PulsarResult<Producer> {
        let store = self.stores.read().unwrap().get(topic_fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(topic_fqn.to_string()))?;
        store.producer_count.fetch_add(1, Ordering::SeqCst);
        Ok(Producer {
            topic: topic_fqn.to_string(),
            store,
            name: format!("producer-{}", Uuid::new_v4()),
            sequence: AtomicU64::new(0),
        })
    }

    pub fn open_consumer(&self, topic_fqn: &str, sub_name: &str) -> PulsarResult<Consumer> {
        let store = self.stores.read().unwrap().get(topic_fqn).cloned()
            .ok_or_else(|| PulsarError::TopicNotFound(topic_fqn.to_string()))?;
        if !store.cursors.read().unwrap().contains_key(sub_name) {
            return Err(PulsarError::SubscriptionNotFound(sub_name.to_string()));
        }
        Ok(Consumer {
            topic: topic_fqn.to_string(),
            sub_name: sub_name.to_string(),
            store,
        })
    }
}

// ───────────────────────────── Producer ─────────────────────────────────────

#[derive(Debug)]
pub struct Producer {
    pub topic: String,
    pub name: String,
    store: Arc<TopicStore>,
    sequence: AtomicU64,
}

impl Producer {
    pub fn send(&self, mut msg: PulsarMessage) -> MessageId {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        msg.sequence_id = seq;
        if msg.producer_name.is_none() {
            msg.producer_name = Some(self.name.clone());
        }
        let id = self.store.append(msg);
        // Bump backlog for every subscription.
        let mut cursors = self.store.cursors.write().unwrap();
        for sub in cursors.values_mut() {
            sub.backlog += 1;
        }
        id
    }

    pub fn close(self) {
        self.store.producer_count.fetch_sub(1, Ordering::SeqCst);
    }
}

// ───────────────────────────── Consumer ─────────────────────────────────────

#[derive(Debug)]
pub struct Consumer {
    pub topic: String,
    pub sub_name: String,
    store: Arc<TopicStore>,
}

impl Consumer {
    pub fn receive(&self) -> PulsarResult<Option<PulsarMessage>> {
        let mut cursors = self.store.cursors.write().unwrap();
        let sub = cursors.get_mut(&self.sub_name)
            .ok_or_else(|| PulsarError::SubscriptionNotFound(self.sub_name.clone()))?;
        let log = self.store.log.read().unwrap();
        if (sub.cursor_pos as usize) >= log.len() {
            return Ok(None);
        }
        let msg = log[sub.cursor_pos as usize].clone();
        let mid = msg.id;
        sub.cursor_pos += 1;
        sub.backlog = (log.len() as i64 - sub.cursor_pos).max(0);
        // Track unacked
        drop(cursors);
        let mut unacked = self.store.unacked.write().unwrap();
        if let Some(m) = unacked.get_mut(&self.sub_name) {
            m.insert(mid.entry_id, ());
        }
        Ok(Some(msg))
    }

    pub fn ack(&self, id: MessageId) -> PulsarResult<()> {
        let mut unacked = self.store.unacked.write().unwrap();
        let m = unacked.get_mut(&self.sub_name)
            .ok_or_else(|| PulsarError::SubscriptionNotFound(self.sub_name.clone()))?;
        m.remove(&id.entry_id);
        Ok(())
    }

    pub fn nack(&self, id: MessageId) -> PulsarResult<()> {
        // Negative ack — for simplicity we move the cursor back to redeliver.
        let mut cursors = self.store.cursors.write().unwrap();
        let sub = cursors.get_mut(&self.sub_name)
            .ok_or_else(|| PulsarError::SubscriptionNotFound(self.sub_name.clone()))?;
        if (id.entry_id as i64) < sub.cursor_pos {
            sub.cursor_pos = id.entry_id as i64;
            sub.backlog = self.store.len() as i64 - sub.cursor_pos;
        }
        Ok(())
    }

    pub fn unacked_count(&self) -> usize {
        self.store.unacked.read().unwrap()
            .get(&self.sub_name).map(|m| m.len()).unwrap_or(0)
    }

    pub fn dlq(&self) -> Vec<PulsarMessage> {
        self.store.dlq.read().unwrap().get(&self.sub_name).cloned().unwrap_or_default()
    }

    pub fn send_to_dlq(&self, msg: PulsarMessage) {
        self.store.dlq.write().unwrap()
            .entry(self.sub_name.clone()).or_default().push(msg);
    }
}

// ───────────────────────────── Stats ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicStats {
    pub topic: String,
    pub messages_in: i64,
    pub producers: i64,
    pub subscriptions: Vec<SubscriptionStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionStats {
    pub name: String,
    pub sub_type: SubscriptionType,
    pub backlog: i64,
    pub cursor: i64,
}

// ───────────────────────────── Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster_with_ns() -> (Arc<PulsarAdminCluster>, &'static str, &'static str) {
        let c = PulsarAdminCluster::new();
        c.create_tenant("acme");
        c.create_namespace("acme", "default").unwrap();
        (c, "acme", "default")
    }

    fn make_topic(c: &PulsarAdminCluster, name: &str) -> String {
        let fqn = format!("persistent://acme/default/{name}");
        c.create_topic(&fqn, 0).unwrap();
        fqn
    }

    #[test]
    fn create_and_list_tenants() {
        let c = PulsarAdminCluster::new();
        c.create_tenant("a");
        c.create_tenant("b");
        let names: Vec<_> = c.list_tenants().iter().map(|t| t.name.clone()).collect();
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"b".to_string()));
    }

    #[test]
    fn delete_unknown_tenant_errors() {
        let c = PulsarAdminCluster::new();
        let err = c.delete_tenant("ghost").unwrap_err();
        assert!(matches!(err, PulsarError::TenantNotFound(_)));
    }

    #[test]
    fn delete_tenant_cascades_namespaces_and_topics() {
        let (c, _t, _ns) = cluster_with_ns();
        let _ = make_topic(&c, "logs");
        c.delete_tenant("acme").unwrap();
        assert!(c.list_tenants().is_empty());
        assert!(c.list_namespaces("acme").is_empty());
        assert!(c.list_topics("acme/default").is_empty());
    }

    #[test]
    fn create_namespace_requires_tenant() {
        let c = PulsarAdminCluster::new();
        let err = c.create_namespace("acme", "default").unwrap_err();
        assert!(matches!(err, PulsarError::TenantNotFound(_)));
    }

    #[test]
    fn namespace_retention_and_ttl_set() {
        let (c, _, _) = cluster_with_ns();
        c.set_namespace_retention("acme/default", 60, 1024).unwrap();
        c.set_namespace_ttl("acme/default", 300).unwrap();
        let ns = c.get_namespace("acme/default").unwrap();
        assert_eq!(ns.retention_minutes, 60);
        assert_eq!(ns.retention_size_mb, 1024);
        assert_eq!(ns.message_ttl_seconds, Some(300));
    }

    #[test]
    fn topic_fqn_round_trip() {
        let (domain, t, ns, n) = Topic::parse("persistent://acme/default/logs").unwrap();
        assert_eq!(domain, TopicDomain::Persistent);
        assert_eq!(t, "acme");
        assert_eq!(ns, "default");
        assert_eq!(n, "logs");
        let topic = Topic {
            domain, tenant: t, namespace: ns, name: n,
            partitions: 0, created_at: Utc::now(),
        };
        assert_eq!(topic.fqn(), "persistent://acme/default/logs");
    }

    #[test]
    fn topic_parse_rejects_garbage() {
        assert!(matches!(Topic::parse("not-a-topic"), Err(PulsarError::InvalidTopic(_))));
        assert!(matches!(Topic::parse("foo://bar"), Err(PulsarError::InvalidTopic(_))));
        assert!(matches!(Topic::parse("persistent://only-one-segment"), Err(PulsarError::InvalidTopic(_))));
    }

    #[test]
    fn non_persistent_topic_supported() {
        let (c, _, _) = cluster_with_ns();
        let fqn = "non-persistent://acme/default/transient";
        c.create_topic(fqn, 0).unwrap();
        assert!(c.topic_exists(fqn));
    }

    #[test]
    fn create_topic_requires_namespace() {
        let c = PulsarAdminCluster::new();
        c.create_tenant("acme");
        let err = c.create_topic("persistent://acme/no-such/topic", 0).unwrap_err();
        assert!(matches!(err, PulsarError::NamespaceNotFound(_)));
    }

    #[test]
    fn create_duplicate_topic_errors() {
        let (c, _, _) = cluster_with_ns();
        let _ = make_topic(&c, "a");
        let err = c.create_topic("persistent://acme/default/a", 0).unwrap_err();
        assert!(matches!(err, PulsarError::TopicAlreadyExists(_)));
    }

    #[test]
    fn delete_topic_removes_from_listing() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "drop");
        c.delete_topic(&fqn).unwrap();
        assert!(!c.topic_exists(&fqn));
        assert!(c.list_topics("acme/default").is_empty());
    }

    #[test]
    fn list_topics_scoped_to_namespace() {
        let (c, _, _) = cluster_with_ns();
        c.create_namespace("acme", "other").unwrap();
        let _ = make_topic(&c, "in-default");
        c.create_topic("persistent://acme/other/in-other", 0).unwrap();
        let default_topics = c.list_topics("acme/default");
        let other_topics = c.list_topics("acme/other");
        assert_eq!(default_topics.len(), 1);
        assert_eq!(other_topics.len(), 1);
    }

    #[test]
    fn produce_and_consume_one_message() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s1", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        let id = p.send(PulsarMessage::new(b"hello".to_vec()));
        let cons = c.open_consumer(&fqn, "s1").unwrap();
        let m = cons.receive().unwrap().unwrap();
        assert_eq!(m.value, b"hello");
        assert_eq!(m.id, id);
    }

    #[test]
    fn fifo_order_preserved() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s1", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        for i in 0..5 {
            p.send(PulsarMessage::new(vec![i]));
        }
        let cons = c.open_consumer(&fqn, "s1").unwrap();
        let mut got = vec![];
        while let Some(m) = cons.receive().unwrap() {
            got.push(m.value[0]);
        }
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn initial_position_latest_skips_existing() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        let p = c.open_producer(&fqn).unwrap();
        p.send(PulsarMessage::new(b"old1".to_vec()));
        p.send(PulsarMessage::new(b"old2".to_vec()));
        c.create_subscription(&fqn, "s1", SubscriptionType::Exclusive, InitialPosition::Latest).unwrap();
        let cons = c.open_consumer(&fqn, "s1").unwrap();
        assert!(cons.receive().unwrap().is_none(), "latest should skip past existing entries");
        // New message after the subscription is delivered.
        p.send(PulsarMessage::new(b"new".to_vec()));
        let m = cons.receive().unwrap().unwrap();
        assert_eq!(m.value, b"new");
    }

    #[test]
    fn ack_clears_unacked_count() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Shared, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        p.send(PulsarMessage::new(b"a".to_vec()));
        p.send(PulsarMessage::new(b"b".to_vec()));
        let cons = c.open_consumer(&fqn, "s").unwrap();
        let m1 = cons.receive().unwrap().unwrap();
        let m2 = cons.receive().unwrap().unwrap();
        assert_eq!(cons.unacked_count(), 2);
        cons.ack(m1.id).unwrap();
        cons.ack(m2.id).unwrap();
        assert_eq!(cons.unacked_count(), 0);
    }

    #[test]
    fn nack_redelivers_message() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        p.send(PulsarMessage::new(b"a".to_vec()));
        p.send(PulsarMessage::new(b"b".to_vec()));
        let cons = c.open_consumer(&fqn, "s").unwrap();
        let m1 = cons.receive().unwrap().unwrap();
        cons.nack(m1.id).unwrap();
        // After nack, next receive sees same message again.
        let again = cons.receive().unwrap().unwrap();
        assert_eq!(again.value, b"a");
    }

    #[test]
    fn skip_all_clears_backlog() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        for _ in 0..10 { p.send(PulsarMessage::new(b"x".to_vec())); }
        c.skip_all(&fqn, "s").unwrap();
        let cons = c.open_consumer(&fqn, "s").unwrap();
        assert!(cons.receive().unwrap().is_none());
    }

    #[test]
    fn reset_cursor_to_earliest_replays_everything() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        let p = c.open_producer(&fqn).unwrap();
        for _ in 0..3 { p.send(PulsarMessage::new(b"x".to_vec())); }
        // Subscription created at Latest after messages exist → cursor is past them.
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Latest).unwrap();
        let cons = c.open_consumer(&fqn, "s").unwrap();
        assert!(cons.receive().unwrap().is_none(), "latest skips existing");
        c.reset_cursor(&fqn, "s", MessageId::EARLIEST).unwrap();
        let mut count = 0;
        while cons.receive().unwrap().is_some() { count += 1; }
        assert_eq!(count, 3);
    }

    #[test]
    fn reset_cursor_to_specific_offset() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        for i in 0..5 { p.send(PulsarMessage::new(vec![i])); }
        c.reset_cursor(&fqn, "s", MessageId::from_offset(3)).unwrap();
        let cons = c.open_consumer(&fqn, "s").unwrap();
        let m = cons.receive().unwrap().unwrap();
        assert_eq!(m.value, vec![3]);
    }

    #[test]
    fn duplicate_subscription_errors() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let err = c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap_err();
        assert!(matches!(err, PulsarError::SubscriptionAlreadyExists(_)));
    }

    #[test]
    fn delete_subscription_removes_it() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        c.delete_subscription(&fqn, "s").unwrap();
        assert!(c.list_subscriptions(&fqn).unwrap().is_empty());
    }

    #[test]
    fn consumer_on_unknown_subscription_errors() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        let err = c.open_consumer(&fqn, "ghost").unwrap_err();
        assert!(matches!(err, PulsarError::SubscriptionNotFound(_)));
    }

    #[test]
    fn topic_stats_counts_messages_and_subs() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s1", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        c.create_subscription(&fqn, "s2", SubscriptionType::Shared, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        p.send(PulsarMessage::new(b"1".to_vec()));
        p.send(PulsarMessage::new(b"2".to_vec()));
        let stats = c.topic_stats(&fqn).unwrap();
        assert_eq!(stats.messages_in, 2);
        assert_eq!(stats.producers, 1);
        assert_eq!(stats.subscriptions.len(), 2);
        let backlogs: Vec<_> = stats.subscriptions.iter().map(|s| s.backlog).collect();
        assert!(backlogs.iter().all(|b| *b == 2));
    }

    #[test]
    fn properties_and_keys_round_trip() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        p.send(PulsarMessage::new(b"v".to_vec())
            .with_key("k1")
            .with_property("trace_id", "abc"));
        let cons = c.open_consumer(&fqn, "s").unwrap();
        let m = cons.receive().unwrap().unwrap();
        assert_eq!(m.key.as_deref(), Some("k1"));
        assert_eq!(m.properties.get("trace_id").map(|s| s.as_str()), Some("abc"));
        assert_eq!(m.producer_name.as_deref(), Some(p.name.as_str()));
    }

    #[test]
    fn dlq_send_and_read() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Exclusive, InitialPosition::Earliest).unwrap();
        let cons = c.open_consumer(&fqn, "s").unwrap();
        cons.send_to_dlq(PulsarMessage::new(b"poison".to_vec()));
        let dlq = cons.dlq();
        assert_eq!(dlq.len(), 1);
        assert_eq!(dlq[0].value, b"poison");
    }

    #[test]
    fn producer_close_decrements_count() {
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        let p1 = c.open_producer(&fqn).unwrap();
        let p2 = c.open_producer(&fqn).unwrap();
        let stats_before = c.topic_stats(&fqn).unwrap();
        assert_eq!(stats_before.producers, 2);
        p1.close();
        let stats_after = c.topic_stats(&fqn).unwrap();
        assert_eq!(stats_after.producers, 1);
        p2.close();
        let stats_zero = c.topic_stats(&fqn).unwrap();
        assert_eq!(stats_zero.producers, 0);
    }

    #[test]
    fn message_id_earliest_latest_constants() {
        assert_eq!(MessageId::EARLIEST.entry_id, 0);
        assert_eq!(MessageId::LATEST.entry_id, u64::MAX);
        assert_ne!(MessageId::EARLIEST, MessageId::LATEST);
    }

    #[test]
    fn delete_namespace_cascades_topics() {
        let (c, _, _) = cluster_with_ns();
        make_topic(&c, "a");
        make_topic(&c, "b");
        c.delete_namespace("acme/default").unwrap();
        assert!(c.list_topics("acme/default").is_empty());
        assert!(c.get_namespace("acme/default").is_err());
    }

    #[test]
    fn shared_subscription_advances_global_cursor() {
        // Two consumers on a Shared subscription should each receive distinct messages.
        let (c, _, _) = cluster_with_ns();
        let fqn = make_topic(&c, "q");
        c.create_subscription(&fqn, "s", SubscriptionType::Shared, InitialPosition::Earliest).unwrap();
        let p = c.open_producer(&fqn).unwrap();
        p.send(PulsarMessage::new(b"1".to_vec()));
        p.send(PulsarMessage::new(b"2".to_vec()));
        let c1 = c.open_consumer(&fqn, "s").unwrap();
        let c2 = c.open_consumer(&fqn, "s").unwrap();
        let m1 = c1.receive().unwrap().unwrap();
        let m2 = c2.receive().unwrap().unwrap();
        assert_eq!(m1.value, b"1");
        assert_eq!(m2.value, b"2");
        assert!(c1.receive().unwrap().is_none());
    }

    #[test]
    fn list_namespaces_scoped_to_tenant() {
        let c = PulsarAdminCluster::new();
        c.create_tenant("a");
        c.create_tenant("b");
        c.create_namespace("a", "ns1").unwrap();
        c.create_namespace("a", "ns2").unwrap();
        c.create_namespace("b", "ns1").unwrap();
        let a_ns = c.list_namespaces("a");
        let b_ns = c.list_namespaces("b");
        assert_eq!(a_ns.len(), 2);
        assert_eq!(b_ns.len(), 1);
        assert_eq!(b_ns[0], "b/ns1");
    }
}
