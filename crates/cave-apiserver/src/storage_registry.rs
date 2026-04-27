//! Generic registry — line-by-line port of upstream
//! `staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go`
//! plus KEP-956 (streaming list / consistent reads) and KEP-1904 (watch
//! progress notification).
//!
//! Layered atop the existing `store.rs` (which is the actual resource
//! store) and `watch_cache.rs`. This module models the generic Strategy
//! type and the consistent-read / watch-progress bookmark protocol that
//! upstream serialises around `etcd.consistentRead`.
//!
//! ## KEP-956 (Consistent reads from cache)
//!
//! Bookmarks at the head of the watch cache let us serve list reads from
//! cache rather than etcd, as long as the cache is at-or-past a recorded
//! resource version.
//!
//! ## KEP-1904 (Watch progress notifications)
//!
//! Idle watches receive periodic Bookmark events containing the current
//! resourceVersion so clients can resume after a disconnect without
//! replaying. Upstream emits a Bookmark every 10 seconds from a goroutine.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// Strategy — `registry/generic/registry/store.go::Strategy`. Each resource
// has its own strategy. We model the per-method hooks as a trait.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct StrategyContext {
    pub user: String,
    pub tenant_id: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyError {
    Invalid(String),
    Forbidden(String),
}

pub trait Strategy: Send + Sync {
    /// Whether the resource is namespaced. Mirrors upstream `NamespaceScoped`.
    fn namespace_scoped(&self) -> bool;
    /// Default new-object fields before validation.
    fn prepare_for_create(&self, ctx: &StrategyContext, obj: &mut serde_json::Value);
    /// Default updated-object fields against the old object.
    fn prepare_for_update(
        &self, ctx: &StrategyContext,
        new_obj: &mut serde_json::Value, old_obj: &serde_json::Value,
    );
    /// Validate a brand-new object.
    fn validate(&self, ctx: &StrategyContext, obj: &serde_json::Value)
        -> Result<(), StrategyError>;
    /// Validate an updated object against its previous state.
    fn validate_update(
        &self, ctx: &StrategyContext,
        new_obj: &serde_json::Value, old_obj: &serde_json::Value,
    ) -> Result<(), StrategyError>;
}

/// Default strategy mix-in: stamps `metadata.creationTimestamp`,
/// rewrites `metadata.resourceVersion`, and refuses cross-tenant writes
/// when the object carries a tenant-id annotation.
pub struct DefaultStrategy {
    pub namespaced: bool,
}

impl Strategy for DefaultStrategy {
    fn namespace_scoped(&self) -> bool { self.namespaced }

    fn prepare_for_create(&self, ctx: &StrategyContext, obj: &mut serde_json::Value) {
        if let Some(meta) = obj.get_mut("metadata").and_then(|m| m.as_object_mut()) {
            meta.entry("creationTimestamp").or_insert_with(||
                serde_json::Value::String(chrono::Utc::now().to_rfc3339()));
            meta.insert("resourceVersion".into(), serde_json::Value::String("0".into()));
            // Stamp tenant annotation when missing.
            let annotations = meta.entry("annotations").or_insert_with(||
                serde_json::Value::Object(serde_json::Map::new()));
            if let Some(ann) = annotations.as_object_mut() {
                ann.entry("cave.runtime/tenant-id".to_string())
                   .or_insert_with(|| serde_json::Value::String(ctx.tenant_id.clone()));
            }
        }
    }

    fn prepare_for_update(
        &self, _: &StrategyContext,
        new_obj: &mut serde_json::Value, old_obj: &serde_json::Value,
    ) {
        // creationTimestamp is immutable.
        if let (Some(new_meta), Some(old_meta)) = (
            new_obj.get_mut("metadata").and_then(|m| m.as_object_mut()),
            old_obj.get("metadata").and_then(|m| m.as_object()),
        ) {
            if let Some(ts) = old_meta.get("creationTimestamp") {
                new_meta.insert("creationTimestamp".into(), ts.clone());
            }
            if let Some(uid) = old_meta.get("uid") {
                new_meta.insert("uid".into(), uid.clone());
            }
        }
    }

    fn validate(&self, ctx: &StrategyContext, obj: &serde_json::Value) -> Result<(), StrategyError> {
        let Some(meta) = obj.get("metadata").and_then(|m| m.as_object()) else {
            return Err(StrategyError::Invalid("metadata missing".into()));
        };
        let Some(name) = meta.get("name").and_then(|n| n.as_str()) else {
            return Err(StrategyError::Invalid("metadata.name required".into()));
        };
        if name.is_empty() {
            return Err(StrategyError::Invalid("metadata.name required".into()));
        }
        if !is_dns1123_subdomain(name) {
            return Err(StrategyError::Invalid(
                format!("metadata.name {name} is not a valid DNS-1123 subdomain")));
        }
        // Tenant annotation invariant.
        if let Some(ann) = meta.get("annotations").and_then(|a| a.as_object()) {
            if let Some(tid) = ann.get("cave.runtime/tenant-id").and_then(|v| v.as_str()) {
                if tid != ctx.tenant_id {
                    return Err(StrategyError::Forbidden(format!(
                        "tenant_id mismatch (object={}, request={})",
                        tid, ctx.tenant_id)));
                }
            }
        }
        Ok(())
    }

    fn validate_update(
        &self, ctx: &StrategyContext,
        new_obj: &serde_json::Value, old_obj: &serde_json::Value,
    ) -> Result<(), StrategyError> {
        self.validate(ctx, new_obj)?;
        // Tenant annotation must not change between update versions.
        let new_t = tenant_of(new_obj);
        let old_t = tenant_of(old_obj);
        if new_t != old_t {
            return Err(StrategyError::Forbidden(format!(
                "tenant_id may not change on update (was={}, now={})",
                old_t, new_t)));
        }
        Ok(())
    }
}

fn tenant_of(o: &serde_json::Value) -> String {
    o.pointer("/metadata/annotations/cave.runtime~1tenant-id")
        .and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// DNS 1123 subdomain: lowercase letters/digits, dots and hyphens, must
/// start and end with alphanumeric. Mirrors `validation.IsDNS1123Subdomain`.
pub fn is_dns1123_subdomain(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 { return false; }
    let bytes = s.as_bytes();
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !alnum(bytes[0]) || !alnum(bytes[bytes.len() - 1]) { return false; }
    let mut prev_dot = false;
    for &c in bytes {
        if c == b'.' {
            if prev_dot { return false; } // no consecutive dots
            prev_dot = true;
            continue;
        }
        prev_dot = false;
        if !(alnum(c) || c == b'-') { return false; }
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-956 — consistent-read protocol from cache. The cache exposes a
// `last_synced_rv` (Lamport-clock lower bound). A read with `resource_version=`
// or `resource_version="0"` is allowed when the cache is at or past the
// requested RV; otherwise we fall through to etcd.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsistentReadOutcome {
    /// The cache is fresh enough — serve from cache at this RV.
    ServeFromCache { at_rv: u64 },
    /// Cache is behind — fall through to backing store.
    FallThrough,
    /// Caller passed an invalid RV (e.g. negative).
    Invalid,
}

pub fn evaluate_consistent_read(
    requested_rv: Option<&str>, cache_synced_rv: u64,
) -> ConsistentReadOutcome {
    let Some(rv) = requested_rv else {
        return ConsistentReadOutcome::ServeFromCache { at_rv: cache_synced_rv };
    };
    if rv == "0" {
        return ConsistentReadOutcome::ServeFromCache { at_rv: cache_synced_rv };
    }
    let Ok(want): Result<u64, _> = rv.parse() else {
        return ConsistentReadOutcome::Invalid;
    };
    if cache_synced_rv >= want {
        ConsistentReadOutcome::ServeFromCache { at_rv: cache_synced_rv }
    } else {
        ConsistentReadOutcome::FallThrough
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-1904 — Watch progress notifications. A watcher with no events for
// `interval` receives a Bookmark event whose RV is the cache's current head.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchEventType {
    Added, Modified, Deleted, Bookmark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchProgressEvent {
    pub kind: WatchEventType,
    pub resource_version: String,
    /// Empty payload for Bookmark events; opaque for the others.
    #[serde(default)]
    pub object: serde_json::Value,
}

pub struct ProgressNotifier {
    pub interval: Duration,
    pub last_emit_rv: Mutex<u64>,
}

impl ProgressNotifier {
    pub fn new(interval: Duration) -> Self {
        Self { interval, last_emit_rv: Mutex::new(0) }
    }
    /// Emit a bookmark IFF `cache_rv > last_emit_rv` and the watcher has been
    /// quiescent. Returns the emitted event when one is produced.
    pub fn maybe_bookmark(&self, cache_rv: u64, idle: bool) -> Option<WatchProgressEvent> {
        if !idle { return None; }
        let mut last = self.last_emit_rv.lock().unwrap();
        if cache_rv <= *last { return None; }
        *last = cache_rv;
        Some(WatchProgressEvent {
            kind: WatchEventType::Bookmark,
            resource_version: cache_rv.to_string(),
            object: serde_json::Value::Null,
        })
    }
    /// Force a bookmark (request from client) — equivalent to the
    /// `?sendInitialEvents=true` plus `?allowWatchBookmarks=true` path.
    pub fn force_bookmark(&self, cache_rv: u64) -> WatchProgressEvent {
        let mut last = self.last_emit_rv.lock().unwrap();
        *last = cache_rv;
        WatchProgressEvent {
            kind: WatchEventType::Bookmark,
            resource_version: cache_rv.to_string(),
            object: serde_json::Value::Null,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming list — `KEP-3157`/`KEP-956`. List requests are now incremental
// streams; we model the chunk protocol.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListChunk {
    pub items: Vec<serde_json::Value>,
    pub continue_token: String,
    pub remaining_item_count: i64,
}

pub struct StreamingListBuilder {
    pub all: Vec<serde_json::Value>,
    pub chunk_size: usize,
}

impl StreamingListBuilder {
    pub fn new(all: Vec<serde_json::Value>, chunk_size: usize) -> Self {
        Self { all, chunk_size }
    }
    pub fn chunk(&self, continue_token: Option<&str>) -> ListChunk {
        let start: usize = continue_token.and_then(|t| t.parse().ok()).unwrap_or(0);
        let end = (start + self.chunk_size).min(self.all.len());
        let items = if start >= self.all.len() {
            vec![]
        } else {
            self.all[start..end].to_vec()
        };
        let remaining = (self.all.len() as i64) - end as i64;
        let token = if end < self.all.len() { end.to_string() } else { String::new() };
        ListChunk { items, continue_token: token, remaining_item_count: remaining.max(0) }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tenant registry index — speeds up list-by-tenant queries. Mirrors
// upstream's `cacher.indexer` but tenant-aware.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct TenantIndex {
    by_tenant: Mutex<HashMap<String, BTreeMap<String, u64>>>, // tenant → name → rv
}

impl TenantIndex {
    pub fn new() -> Self { Self::default() }
    pub fn upsert(&self, tenant: &str, name: &str, rv: u64) {
        self.by_tenant.lock().unwrap()
            .entry(tenant.into()).or_default().insert(name.into(), rv);
    }
    pub fn delete(&self, tenant: &str, name: &str) {
        if let Some(m) = self.by_tenant.lock().unwrap().get_mut(tenant) {
            m.remove(name);
        }
    }
    pub fn list(&self, tenant: &str) -> Vec<(String, u64)> {
        self.by_tenant.lock().unwrap()
            .get(tenant).cloned().unwrap_or_default()
            .into_iter().collect()
    }
    pub fn count(&self, tenant: &str) -> usize {
        self.by_tenant.lock().unwrap()
            .get(tenant).map(|m| m.len()).unwrap_or(0)
    }
}

#[allow(dead_code)]
fn unused_arc() -> Arc<()> { Arc::new(()) }

#[cfg(test)]
mod tests;
