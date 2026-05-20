// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pluggable audit backend registry.
//!
//! Upstream: `staging/src/k8s.io/apiserver/plugin/pkg/audit/`
//!
//! The upstream `plugin/pkg/audit/` tree carries the four canonical sinks
//! (`log`, `webhook`, `buffered`, `truncate`) plus the
//! `audit.Backend` interface and the `audit.Backend.Register()` plumbing
//! that the central audit handler dispatches into. Until this module
//! landed cave-apiserver had two hard-coded sinks (`audit::AuditSink` and
//! `audit_worm::WormStore`) but no way to register a new backend without
//! editing the central handler.
//!
//! This module ports the missing piece: a `Backend` trait + the four
//! built-in implementations (`LogBackend`, `WebhookBackend`,
//! `BufferedBackend`, `TruncatingBackend`) + a name-keyed `BackendRegistry`
//! + an `AuditDispatcher` that fans each `AuditEvent` to every registered
//! backend. The existing `audit.rs` + `audit_worm.rs` sinks continue to
//! work unchanged — this adds a *parallel* plugin chain that future
//! deployment code can opt into via `AuditDispatcher::dispatch()`.
//!
//! Tenant invariant: backends MUST NOT strip `tenant_id` from events.
//! Each built-in honours this; custom backends are encouraged to follow.

use crate::audit::AuditEvent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Result of a backend operation.
pub type BackendResult = Result<(), BackendError>;

/// Errors emitted by an audit backend.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("backend buffer full (dropped events: {dropped})")]
    BufferFull { dropped: u64 },
    #[error("webhook delivery failed: {0}")]
    WebhookFailed(String),
    #[error("backend not registered: {0}")]
    NotRegistered(String),
    #[error("io: {0}")]
    Io(String),
}

/// Pluggable audit backend.
///
/// Mirrors `audit.Backend` from upstream. `process_events` is invoked
/// from the central handler with the batch of events that survived
/// policy + filter chains. Implementations should be `Send + Sync` and
/// should never block on I/O longer than ~100ms; long-running sinks
/// belong behind `BufferedBackend`.
#[async_trait]
pub trait AuditBackend: Send + Sync {
    /// Stable name used for registration and `BackendRegistry` lookup.
    fn name(&self) -> &str;

    /// Process a batch of events. Backends MUST NOT modify
    /// `event.tenant_id`; doing so is a Charter v2 invariant violation.
    async fn process_events(&self, events: &[AuditEvent]) -> BackendResult;

    /// Shutdown hook — flush any buffered events. Default no-op.
    async fn shutdown(&self) -> BackendResult {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// LogBackend — writes events to an in-memory log buffer (mirrors upstream
// log.Backend, which writes to a file). Production deployments swap the
// underlying sink for a tracing-subscriber writer; the in-memory store
// is sufficient for parity tests and unit tests.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct LogBackend {
    name: String,
    events: RwLock<Vec<AuditEvent>>,
}

impl LogBackend {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            events: RwLock::new(Vec::new()),
        }
    }

    pub async fn drain(&self) -> Vec<AuditEvent> {
        let mut g = self.events.write().await;
        std::mem::take(&mut *g)
    }

    pub async fn len(&self) -> usize {
        self.events.read().await.len()
    }
}

#[async_trait]
impl AuditBackend for LogBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn process_events(&self, events: &[AuditEvent]) -> BackendResult {
        let mut g = self.events.write().await;
        g.extend_from_slice(events);
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WebhookBackend — POSTs events to a configured URL. We don't ship the
// actual HTTP client here (it would pull reqwest into the lib build);
// instead we expose a `Delivery` trait so callers wire in their preferred
// transport. A `RecordingDelivery` is provided for tests.
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait Delivery: Send + Sync {
    async fn deliver(&self, url: &str, payload: &[u8]) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct RecordingDelivery {
    pub posted: RwLock<Vec<(String, Vec<u8>)>>,
}

#[async_trait]
impl Delivery for RecordingDelivery {
    async fn deliver(&self, url: &str, payload: &[u8]) -> Result<(), String> {
        self.posted
            .write()
            .await
            .push((url.to_string(), payload.to_vec()));
        Ok(())
    }
}

pub struct WebhookBackend {
    name: String,
    url: String,
    delivery: Arc<dyn Delivery>,
}

impl WebhookBackend {
    pub fn new(
        name: impl Into<String>,
        url: impl Into<String>,
        delivery: Arc<dyn Delivery>,
    ) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            delivery,
        }
    }
}

#[async_trait]
impl AuditBackend for WebhookBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn process_events(&self, events: &[AuditEvent]) -> BackendResult {
        let payload = serde_json::to_vec(events)
            .map_err(|e| BackendError::WebhookFailed(format!("serialise: {}", e)))?;
        self.delivery
            .deliver(&self.url, &payload)
            .await
            .map_err(BackendError::WebhookFailed)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BufferedBackend — wraps an inner backend with a bounded ring-buffer +
// background drain. Equivalent to upstream `buffered.Backend`.
// ─────────────────────────────────────────────────────────────────────────────

pub struct BufferedBackend {
    name: String,
    inner: Arc<dyn AuditBackend>,
    capacity: usize,
    buffer: RwLock<Vec<AuditEvent>>,
    dropped: RwLock<u64>,
}

impl BufferedBackend {
    pub fn new(name: impl Into<String>, inner: Arc<dyn AuditBackend>, capacity: usize) -> Self {
        Self {
            name: name.into(),
            inner,
            capacity,
            buffer: RwLock::new(Vec::new()),
            dropped: RwLock::new(0),
        }
    }

    /// Number of events currently held in the buffer.
    pub async fn buffered_len(&self) -> usize {
        self.buffer.read().await.len()
    }

    /// Lifetime-cumulative count of events that were dropped because the
    /// buffer was at capacity.
    pub async fn dropped_count(&self) -> u64 {
        *self.dropped.read().await
    }

    /// Flush the buffer through the inner backend.
    pub async fn flush(&self) -> BackendResult {
        let batch = {
            let mut g = self.buffer.write().await;
            std::mem::take(&mut *g)
        };
        if batch.is_empty() {
            return Ok(());
        }
        self.inner.process_events(&batch).await
    }
}

#[async_trait]
impl AuditBackend for BufferedBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn process_events(&self, events: &[AuditEvent]) -> BackendResult {
        let mut g = self.buffer.write().await;
        let room = self.capacity.saturating_sub(g.len());
        let take = events.len().min(room);
        g.extend_from_slice(&events[..take]);
        let dropped = (events.len() - take) as u64;
        if dropped > 0 {
            *self.dropped.write().await += dropped;
        }
        Ok(())
    }

    async fn shutdown(&self) -> BackendResult {
        self.flush().await?;
        self.inner.shutdown().await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TruncatingBackend — clamps oversized RequestResponse payloads so they
// don't blow up the downstream sink. Mirrors upstream
// `truncate.Backend`.
// ─────────────────────────────────────────────────────────────────────────────

pub struct TruncatingBackend {
    name: String,
    inner: Arc<dyn AuditBackend>,
    max_payload_bytes: usize,
}

impl TruncatingBackend {
    pub fn new(
        name: impl Into<String>,
        inner: Arc<dyn AuditBackend>,
        max_payload_bytes: usize,
    ) -> Self {
        Self {
            name: name.into(),
            inner,
            max_payload_bytes,
        }
    }

    fn truncated_payload(value: &serde_json::Value, max: usize) -> serde_json::Value {
        let s = value.to_string();
        if s.len() <= max {
            return value.clone();
        }
        // Replace with a structured marker — never strip silently.
        serde_json::json!({
            "_truncated": true,
            "original_size": s.len(),
            "max_size": max,
        })
    }
}

#[async_trait]
impl AuditBackend for TruncatingBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn process_events(&self, events: &[AuditEvent]) -> BackendResult {
        let mut truncated: Vec<AuditEvent> = events.to_vec();
        for ev in truncated.iter_mut() {
            if let Some(req) = ev.request_object.as_ref() {
                ev.request_object = Some(Self::truncated_payload(req, self.max_payload_bytes));
            }
            if let Some(rsp) = ev.response_object.as_ref() {
                ev.response_object = Some(Self::truncated_payload(rsp, self.max_payload_bytes));
            }
        }
        self.inner.process_events(&truncated).await
    }

    async fn shutdown(&self) -> BackendResult {
        self.inner.shutdown().await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BackendRegistry — name → Backend lookup. Equivalent to upstream
// `audit.Registry`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct BackendRegistry {
    by_name: RwLock<HashMap<String, Arc<dyn AuditBackend>>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, backend: Arc<dyn AuditBackend>) {
        let n = backend.name().to_string();
        self.by_name.write().await.insert(n, backend);
    }

    pub async fn unregister(&self, name: &str) -> Option<Arc<dyn AuditBackend>> {
        self.by_name.write().await.remove(name)
    }

    pub async fn get(&self, name: &str) -> Option<Arc<dyn AuditBackend>> {
        self.by_name.read().await.get(name).cloned()
    }

    pub async fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.by_name.read().await.keys().cloned().collect();
        v.sort();
        v
    }

    pub async fn len(&self) -> usize {
        self.by_name.read().await.len()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AuditDispatcher — fans an event batch out to every registered backend.
// Errors are collected but never short-circuit; one broken backend must
// not silence the others.
// ─────────────────────────────────────────────────────────────────────────────

pub struct AuditDispatcher {
    registry: Arc<BackendRegistry>,
}

impl AuditDispatcher {
    pub fn new(registry: Arc<BackendRegistry>) -> Self {
        Self { registry }
    }

    pub async fn dispatch(&self, events: &[AuditEvent]) -> Vec<(String, BackendError)> {
        let mut failures = Vec::new();
        let names = self.registry.names().await;
        for n in names {
            if let Some(b) = self.registry.get(&n).await {
                if let Err(e) = b.process_events(events).await {
                    failures.push((n.clone(), e));
                }
            }
        }
        failures
    }

    pub async fn shutdown_all(&self) -> Vec<(String, BackendError)> {
        let mut failures = Vec::new();
        let names = self.registry.names().await;
        for n in names {
            if let Some(b) = self.registry.get(&n).await {
                if let Err(e) = b.shutdown().await {
                    failures.push((n.clone(), e));
                }
            }
        }
        failures
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditLevel, AuditStage};

    fn ev(audit_id: &str) -> AuditEvent {
        AuditEvent::new(
            audit_id,
            AuditLevel::Metadata,
            AuditStage::ResponseComplete,
            "alice",
            "tenant-a",
            "ns",
            "create",
            "pods",
            "nginx",
            "/api/v1/namespaces/ns/pods",
            201,
        )
    }

    #[tokio::test]
    async fn log_backend_records_events_in_order() {
        let b = LogBackend::new("log");
        b.process_events(&[ev("1"), ev("2"), ev("3")]).await.unwrap();
        assert_eq!(b.len().await, 3);
        let drained = b.drain().await;
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].audit_id, "1");
        assert_eq!(drained[2].audit_id, "3");
        assert_eq!(b.len().await, 0);
    }

    #[tokio::test]
    async fn webhook_backend_posts_serialised_batch() {
        let rec = Arc::new(RecordingDelivery::default());
        let b = WebhookBackend::new("hook", "https://example.test/audit", rec.clone());
        b.process_events(&[ev("a"), ev("b")]).await.unwrap();
        let posted = rec.posted.read().await;
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].0, "https://example.test/audit");
        let parsed: Vec<AuditEvent> = serde_json::from_slice(&posted[0].1).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].audit_id, "a");
    }

    #[tokio::test]
    async fn buffered_backend_drops_when_over_capacity() {
        let inner = Arc::new(LogBackend::new("log"));
        let inner_dyn: Arc<dyn AuditBackend> = inner.clone();
        let buf = BufferedBackend::new("buf", inner_dyn, 2);
        buf.process_events(&[ev("1"), ev("2"), ev("3"), ev("4")])
            .await
            .unwrap();
        assert_eq!(buf.buffered_len().await, 2);
        assert_eq!(buf.dropped_count().await, 2);
        // Flush propagates buffered events to the inner backend.
        buf.flush().await.unwrap();
        assert_eq!(inner.len().await, 2);
    }

    #[tokio::test]
    async fn truncating_backend_clamps_oversized_request_object() {
        let inner = Arc::new(LogBackend::new("log"));
        let inner_dyn: Arc<dyn AuditBackend> = inner.clone();
        let t = TruncatingBackend::new("trunc", inner_dyn, 32);
        let mut big = ev("big");
        // ~100-char payload — well over the 32-byte clamp.
        big.request_object = Some(serde_json::json!({
            "long": "x".repeat(200),
        }));
        t.process_events(&[big]).await.unwrap();
        let events = inner.drain().await;
        assert_eq!(events.len(), 1);
        let req = events[0].request_object.as_ref().unwrap();
        assert_eq!(req["_truncated"], serde_json::json!(true));
        assert!(req["original_size"].as_u64().unwrap() > 32);
    }

    #[tokio::test]
    async fn registry_register_lookup_unregister_round_trip() {
        let reg = BackendRegistry::new();
        let a: Arc<dyn AuditBackend> = Arc::new(LogBackend::new("a"));
        let b: Arc<dyn AuditBackend> = Arc::new(LogBackend::new("b"));
        reg.register(a.clone()).await;
        reg.register(b.clone()).await;
        assert_eq!(reg.len().await, 2);
        assert_eq!(reg.names().await, vec!["a".to_string(), "b".to_string()]);
        assert!(reg.get("a").await.is_some());
        assert!(reg.get("missing").await.is_none());
        reg.unregister("a").await.expect("unregister returns backend");
        assert_eq!(reg.len().await, 1);
        assert!(reg.get("a").await.is_none());
    }

    #[tokio::test]
    async fn dispatcher_fans_out_to_all_registered_backends() {
        let reg = Arc::new(BackendRegistry::new());
        let log1 = Arc::new(LogBackend::new("log1"));
        let log2 = Arc::new(LogBackend::new("log2"));
        let log1_dyn: Arc<dyn AuditBackend> = log1.clone();
        let log2_dyn: Arc<dyn AuditBackend> = log2.clone();
        reg.register(log1_dyn).await;
        reg.register(log2_dyn).await;
        let d = AuditDispatcher::new(reg);
        let failures = d.dispatch(&[ev("x"), ev("y")]).await;
        assert!(failures.is_empty(), "no backend should fail");

        assert_eq!(log1.len().await, 2);
        assert_eq!(log2.len().await, 2);
    }

    /// Backend that always errors — used to prove dispatch does not
    /// short-circuit on a single broken sink.
    struct AlwaysFail;
    #[async_trait]
    impl AuditBackend for AlwaysFail {
        fn name(&self) -> &str {
            "fail"
        }
        async fn process_events(&self, _events: &[AuditEvent]) -> BackendResult {
            Err(BackendError::Io("boom".into()))
        }
    }

    #[tokio::test]
    async fn dispatcher_records_failure_but_continues_other_backends() {
        let reg = Arc::new(BackendRegistry::new());
        let good = Arc::new(LogBackend::new("good"));
        let good_dyn: Arc<dyn AuditBackend> = good.clone();
        reg.register(Arc::new(AlwaysFail)).await;
        reg.register(good_dyn).await;
        let d = AuditDispatcher::new(reg);
        let failures = d.dispatch(&[ev("z")]).await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "fail");
        // The healthy backend still received the batch.
        assert_eq!(good.len().await, 1);
    }

    #[tokio::test]
    async fn buffered_backend_preserves_tenant_id() {
        let inner = Arc::new(LogBackend::new("log"));
        let inner_dyn: Arc<dyn AuditBackend> = inner.clone();
        let buf = BufferedBackend::new("buf", inner_dyn, 10);
        let mut e = ev("t");
        e.tenant_id = "tenant-z".into();
        buf.process_events(&[e]).await.unwrap();
        buf.flush().await.unwrap();
        let got = inner.drain().await;
        assert_eq!(got[0].tenant_id, "tenant-z");
    }
}
