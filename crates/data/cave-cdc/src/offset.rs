// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Offset backing store — checkpoint persistence for connector resume.
//!
//! Cite: debezium-storage `io.debezium.storage.OffsetBackingStore`
//! interface + `io.debezium.storage.file.history.FileOffsetBackingStore`
//! (file-backed) and `InMemoryOffsetBackingStore` (test/embedded).
//! cave-cdc exposes an in-memory implementation that is serializable
//! to JSON so the operator can snapshot and reload offsets without a
//! Kafka Connect cluster.
//!
//! Key design: offsets are keyed by a composite `OffsetKey` that
//! encodes `(tenant_id, connector_name, schema, table)` so keys from
//! different tenants can NEVER collide even in a shared store.

use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Composite key for a connector offset. Cite: debezium
/// `OffsetBackingStore` — keys are opaque byte arrays in the Java
/// API; cave makes them typed + tenant-scoped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OffsetKey {
    pub tenant_id: String,
    pub connector: String,
    pub schema: String,
    pub table: String,
}

impl OffsetKey {
    pub fn new(
        tenant_id: impl Into<String>,
        connector: impl Into<String>,
        schema: impl Into<String>,
        table: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            connector: connector.into(),
            schema: schema.into(),
            table: table.into(),
        }
    }

    /// Produce a stable, opaque string key suitable for on-disk storage
    /// or topic naming. Cite: debezium `KafkaOffsetBackingStore` key
    /// encoding — use `|` as a separator (safe since tenant names
    /// cannot contain it in cave's naming rules).
    pub fn to_key_string(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.tenant_id, self.connector, self.schema, self.table
        )
    }
}

/// In-memory offset store.  Cite: debezium
/// `InMemoryOffsetBackingStore` — stores offsets in a `HashMap`
/// keyed by the partition map; used for embedded + test scenarios.
/// cave extends this with: tenant isolation, JSON serialisation, and
/// a `set_checked` method that enforces cross-tenant guards.
#[derive(Debug, Default)]
pub struct OffsetStore {
    pub tenant_id: String,
    /// canonical key string → JSON offset value.
    offsets: HashMap<String, serde_json::Value>,
    /// Retain the typed key alongside the string key so callers can
    /// iterate over all offsets with full metadata.
    keys: HashMap<String, OffsetKey>,
}

impl OffsetStore {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            offsets: HashMap::new(),
            keys: HashMap::new(),
        }
    }

    /// Unconditionally set an offset.  Cite: debezium
    /// `OffsetBackingStore::set` — overwrites any existing value for
    /// the key. Call `set_checked` when tenant isolation must be
    /// enforced.
    pub fn set(&mut self, key: OffsetKey, value: serde_json::Value) {
        let k = key.to_key_string();
        self.offsets.insert(k.clone(), value);
        self.keys.insert(k, key);
    }

    /// Tenant-checked set.  Rejects keys whose `tenant_id` differs
    /// from the store's `tenant_id`.
    pub fn set_checked(&mut self, key: OffsetKey, value: serde_json::Value) -> CdcResult<()> {
        if key.tenant_id != self.tenant_id {
            return Err(CdcError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: key.tenant_id.clone(),
            });
        }
        self.set(key, value);
        Ok(())
    }

    /// Retrieve the latest committed offset for a key. Returns `None`
    /// if the connector has never committed to this partition.  Cite:
    /// debezium `OffsetBackingStore::get`.
    pub fn get(&self, key: &OffsetKey) -> Option<&serde_json::Value> {
        self.offsets.get(&key.to_key_string())
    }

    /// Delete the stored offset for a key.  Cite: debezium
    /// `OffsetBackingStore::set(key, null)` — setting to null is the
    /// API for deleting an offset.
    pub fn delete(&mut self, key: &OffsetKey) {
        let k = key.to_key_string();
        self.offsets.remove(&k);
        self.keys.remove(&k);
    }

    /// Return a snapshot of all stored (key, value) pairs.
    pub fn all_offsets(&self) -> Vec<(&OffsetKey, &serde_json::Value)> {
        self.offsets
            .iter()
            .filter_map(|(k, v)| self.keys.get(k).map(|ok| (ok, v)))
            .collect()
    }

    /// Serialize the full store to JSON for persistence. Cite: debezium
    /// `FileOffsetBackingStore` — persists to a flat file using the
    /// Kafka Connect serialisation format; cave uses plain JSON.
    pub fn to_json(&self) -> serde_json::Value {
        let map: serde_json::Map<String, serde_json::Value> = self
            .offsets
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        serde_json::Value::Object(map)
    }
}
