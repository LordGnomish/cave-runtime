// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Offline-first local metadata store — KubeEdge `edge/pkg/metamanager`.
//!
//! MetaManager is the edge node's source of truth while the cloud link is
//! down. It mirrors the upstream metaDB cache and the query path:
//!
//!   * `Insert`/`Update`/`Delete` mutate the local record set;
//!   * a `Query` that hits the local cache answers immediately;
//!   * a `Query` miss while online forwards to the cloud (the caller drives
//!     EdgeHub), then `cache_cloud_response` stores the reply so later reads
//!     are local — this is the "subsequent queries retrieved from the
//!     database, reducing latency" behavior;
//!   * a `Query` miss while offline returns `NotFound` (no round-trip).
//!
//! Resource versions are tracked per key and updates are monotonic — a
//! late-arriving older write is rejected so the freshest spec survives a
//! reconnect storm.

use std::collections::BTreeMap;

/// Result of a metadata query — drives the caller's next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryOutcome {
    /// Local cache hit; the stored value is returned.
    Hit(String),
    /// Local miss while the cloud link is up — forward the request upstream.
    ForwardToCloud,
    /// Local miss while offline — nothing to serve.
    NotFound,
}

#[derive(Debug, Clone)]
struct Record {
    value: String,
    resource_version: u64,
}

/// The edge metadata store.
#[derive(Debug, Clone, Default)]
pub struct MetaManager {
    store: BTreeMap<String, Record>,
    online: bool,
}

impl MetaManager {
    pub fn new() -> Self {
        // Edge boots assuming the cloud is reachable until told otherwise.
        Self {
            store: BTreeMap::new(),
            online: true,
        }
    }

    /// Update the cloud-connection state (driven by EdgeHub / autonomy).
    pub fn set_online(&mut self, online: bool) {
        self.online = online;
    }

    pub fn is_online(&self) -> bool {
        self.online
    }

    /// `InsertOperation` — unconditional local write.
    pub fn insert(&mut self, key: &str, value: &str, resource_version: u64) {
        self.store.insert(
            key.to_string(),
            Record {
                value: value.to_string(),
                resource_version,
            },
        );
    }

    /// `UpdateOperation` — monotonic by resource version. Returns true when
    /// applied, false when the incoming version is not strictly newer (stale).
    pub fn update(&mut self, key: &str, value: &str, resource_version: u64) -> bool {
        match self.store.get(key) {
            Some(existing) if resource_version <= existing.resource_version => false,
            _ => {
                self.store.insert(
                    key.to_string(),
                    Record {
                        value: value.to_string(),
                        resource_version,
                    },
                );
                true
            }
        }
    }

    /// `DeleteOperation` — drop a local record.
    pub fn delete(&mut self, key: &str) {
        self.store.remove(key);
    }

    /// `QueryOperation` — offline-first read.
    pub fn query(&self, key: &str) -> QueryOutcome {
        match self.store.get(key) {
            Some(r) => QueryOutcome::Hit(r.value.clone()),
            None if self.online => QueryOutcome::ForwardToCloud,
            None => QueryOutcome::NotFound,
        }
    }

    /// Store a cloud response after a forwarded miss (cache-through). Uses
    /// the same monotonic guard as `update` so it never regresses a key.
    pub fn cache_cloud_response(&mut self, key: &str, value: &str, resource_version: u64) {
        match self.store.get(key) {
            Some(existing) if resource_version < existing.resource_version => {}
            _ => {
                self.store.insert(
                    key.to_string(),
                    Record {
                        value: value.to_string(),
                        resource_version,
                    },
                );
            }
        }
    }

    /// Current resource version stored for a key.
    pub fn resource_version(&self, key: &str) -> Option<u64> {
        self.store.get(key).map(|r| r.resource_version)
    }

    /// List-by-type: every key sharing `prefix`, sorted (BTreeMap order).
    pub fn list_by_prefix(&self, prefix: &str) -> Vec<(String, String)> {
        self.store
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, r)| (k.clone(), r.value.clone()))
            .collect()
    }

    /// Number of locally cached records.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}
