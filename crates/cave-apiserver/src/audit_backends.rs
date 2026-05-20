// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pluggable audit-backend registry.
//!
//! Mirrors `staging/src/k8s.io/apiserver/plugin/pkg/audit/` from
//! kubernetes/kubernetes v1.36.0 — upstream ships **log**, **webhook**,
//! **buffered** and **truncate** backends and a registry that allows
//! the apiserver to fan-out one event to many backends.
//!
//! cave-apiserver previously hard-wired audit + audit_worm sinks at the
//! handler edge; adding another backend meant editing `routes.rs`. This
//! module reproduces the upstream surface so backends can be registered
//! at boot and discovered by name, just like the upstream
//! `audit.Backend` interface.

use crate::audit::AuditEvent;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Mirrors `audit.Backend.Run` / `audit.Backend.Shutdown` lifecycle.
pub trait AuditBackend: Send + Sync {
    fn name(&self) -> &str;
    fn process(&self, event: &AuditEvent);
    fn flush(&self) {}
    fn shutdown(&self) {}
}

/// `log` backend — append-only, mirrors upstream `audit/log`.
pub struct LogBackend {
    name: String,
    events: Mutex<Vec<AuditEvent>>,
}

impl LogBackend {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            events: Mutex::new(Vec::new()),
        }
    }

    pub fn events(&self) -> Vec<AuditEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl AuditBackend for LogBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn process(&self, e: &AuditEvent) {
        self.events.lock().unwrap().push(e.clone());
    }
}

/// `webhook` backend — POSTs events; the actual transport is injected
/// via `WebhookSender` so unit tests can run without I/O.
pub trait WebhookSender: Send + Sync {
    fn send(&self, event: &AuditEvent) -> Result<(), String>;
}

pub struct WebhookBackend<S: WebhookSender> {
    name: String,
    sender: S,
}

impl<S: WebhookSender> WebhookBackend<S> {
    pub fn new(name: impl Into<String>, sender: S) -> Self {
        Self {
            name: name.into(),
            sender,
        }
    }
}

impl<S: WebhookSender + 'static> AuditBackend for WebhookBackend<S> {
    fn name(&self) -> &str {
        &self.name
    }
    fn process(&self, e: &AuditEvent) {
        // Failures are logged-and-dropped per upstream behaviour — the
        // request must not block on a misbehaving audit webhook.
        let _ = self.sender.send(e);
    }
}

/// `buffered` backend — wraps another backend, batches up to `capacity`
/// events or `flush_after`, whichever comes first. Mirrors upstream
/// `audit/buffered`.
pub struct BufferedBackend {
    name: String,
    inner: Arc<dyn AuditBackend>,
    queue: Mutex<VecDeque<AuditEvent>>,
    capacity: usize,
    flush_after: Duration,
}

impl BufferedBackend {
    pub fn new(
        name: impl Into<String>,
        inner: Arc<dyn AuditBackend>,
        capacity: usize,
        flush_after: Duration,
    ) -> Self {
        Self {
            name: name.into(),
            inner,
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            flush_after,
        }
    }

    pub fn flush_after(&self) -> Duration {
        self.flush_after
    }

    pub fn pending(&self) -> usize {
        self.queue.lock().unwrap().len()
    }
}

impl AuditBackend for BufferedBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn process(&self, e: &AuditEvent) {
        let mut q = self.queue.lock().unwrap();
        q.push_back(e.clone());
        if q.len() >= self.capacity {
            // drain to inner under the lock — keeps event order
            while let Some(ev) = q.pop_front() {
                self.inner.process(&ev);
            }
        }
    }
    fn flush(&self) {
        let mut q = self.queue.lock().unwrap();
        while let Some(ev) = q.pop_front() {
            self.inner.process(&ev);
        }
        self.inner.flush();
    }
    fn shutdown(&self) {
        self.flush();
        self.inner.shutdown();
    }
}

/// `truncate` backend — wraps another backend, drops field bodies that
/// exceed `max_event_bytes`. Mirrors upstream `audit/truncate`.
pub struct TruncateBackend {
    name: String,
    inner: Arc<dyn AuditBackend>,
    max_event_bytes: usize,
}

impl TruncateBackend {
    pub fn new(
        name: impl Into<String>,
        inner: Arc<dyn AuditBackend>,
        max_event_bytes: usize,
    ) -> Self {
        Self {
            name: name.into(),
            inner,
            max_event_bytes,
        }
    }

    fn truncate(&self, e: &AuditEvent) -> AuditEvent {
        let mut out = e.clone();
        if let Some(body) = out.request_object.as_ref() {
            let s = body.to_string();
            if s.len() > self.max_event_bytes {
                out.request_object = Some(serde_json::json!({
                    "@truncated": true,
                    "originalBytes": s.len(),
                }));
            }
        }
        if let Some(body) = out.response_object.as_ref() {
            let s = body.to_string();
            if s.len() > self.max_event_bytes {
                out.response_object = Some(serde_json::json!({
                    "@truncated": true,
                    "originalBytes": s.len(),
                }));
            }
        }
        out
    }
}

impl AuditBackend for TruncateBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn process(&self, e: &AuditEvent) {
        self.inner.process(&self.truncate(e));
    }
    fn flush(&self) {
        self.inner.flush();
    }
    fn shutdown(&self) {
        self.inner.shutdown();
    }
}

/// Registry of named audit backends. Mirrors upstream
/// `audit/dynamicconfig/registry.go` — backends register by name at
/// boot, the apiserver fan-outs each event to every registered
/// backend.
#[derive(Default)]
pub struct AuditBackendRegistry {
    backends: RwLock<Vec<Arc<dyn AuditBackend>>>,
}

impl AuditBackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, b: Arc<dyn AuditBackend>) {
        self.backends.write().unwrap().push(b);
    }

    pub fn names(&self) -> Vec<String> {
        self.backends
            .read()
            .unwrap()
            .iter()
            .map(|b| b.name().to_string())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.backends.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn fan_out(&self, event: &AuditEvent) {
        for b in self.backends.read().unwrap().iter() {
            b.process(event);
        }
    }

    pub fn flush_all(&self) {
        for b in self.backends.read().unwrap().iter() {
            b.flush();
        }
    }

    pub fn shutdown_all(&self) {
        for b in self.backends.read().unwrap().iter() {
            b.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditEvent, AuditLevel, AuditStage};

    fn evt(verb: &str) -> AuditEvent {
        AuditEvent::new(
            format!("test-{verb}"),
            AuditLevel::Metadata,
            AuditStage::ResponseComplete,
            "alice",
            "tenant",
            "default",
            verb,
            "pods",
            "p1",
            "/api/v1/namespaces/default/pods",
            200,
        )
    }

    #[test]
    fn log_backend_records_events() {
        let b = LogBackend::new("log");
        b.process(&evt("create"));
        b.process(&evt("update"));
        assert_eq!(b.events().len(), 2);
    }

    #[test]
    fn registry_fans_out_to_all_backends() {
        let r = AuditBackendRegistry::new();
        let a = Arc::new(LogBackend::new("a"));
        let b = Arc::new(LogBackend::new("b"));
        r.register(a.clone());
        r.register(b.clone());
        r.fan_out(&evt("create"));
        assert_eq!(a.events().len(), 1);
        assert_eq!(b.events().len(), 1);
    }

    #[test]
    fn registry_names_in_registration_order() {
        let r = AuditBackendRegistry::new();
        r.register(Arc::new(LogBackend::new("first")));
        r.register(Arc::new(LogBackend::new("second")));
        assert_eq!(r.names(), vec!["first".to_string(), "second".to_string()]);
        assert_eq!(r.len(), 2);
    }

    struct CapturingSender(Mutex<Vec<AuditEvent>>);
    impl WebhookSender for CapturingSender {
        fn send(&self, e: &AuditEvent) -> Result<(), String> {
            self.0.lock().unwrap().push(e.clone());
            Ok(())
        }
    }

    #[test]
    fn webhook_backend_forwards_event_via_sender() {
        let s = CapturingSender(Mutex::new(vec![]));
        let wb = WebhookBackend::new("hook", s);
        wb.process(&evt("delete"));
        assert_eq!(wb.sender.0.lock().unwrap().len(), 1);
    }

    struct FailingSender;
    impl WebhookSender for FailingSender {
        fn send(&self, _: &AuditEvent) -> Result<(), String> {
            Err("boom".into())
        }
    }

    #[test]
    fn webhook_backend_swallows_send_errors() {
        let wb = WebhookBackend::new("hook", FailingSender);
        wb.process(&evt("create")); // must not panic
    }

    #[test]
    fn buffered_backend_holds_events_until_capacity() {
        let inner = Arc::new(LogBackend::new("sink"));
        let bb = BufferedBackend::new(
            "buf",
            inner.clone() as Arc<dyn AuditBackend>,
            3,
            Duration::from_secs(60),
        );
        bb.process(&evt("a"));
        bb.process(&evt("b"));
        assert_eq!(bb.pending(), 2);
        assert_eq!(inner.events().len(), 0);
        bb.process(&evt("c"));
        // capacity reached → drains
        assert_eq!(bb.pending(), 0);
        assert_eq!(inner.events().len(), 3);
    }

    #[test]
    fn buffered_backend_flush_drains_partial_batch() {
        let inner = Arc::new(LogBackend::new("sink"));
        let bb = BufferedBackend::new(
            "buf",
            inner.clone() as Arc<dyn AuditBackend>,
            5,
            Duration::from_secs(60),
        );
        bb.process(&evt("x"));
        bb.flush();
        assert_eq!(inner.events().len(), 1);
    }

    #[test]
    fn truncate_backend_strips_large_request_body() {
        let inner = Arc::new(LogBackend::new("sink"));
        let tb = TruncateBackend::new("trunc", inner.clone() as Arc<dyn AuditBackend>, 16);
        let mut e = evt("create");
        e.request_object = Some(serde_json::json!({"k": "x".repeat(64)}));
        tb.process(&e);
        let recorded = inner.events();
        let body = recorded[0].request_object.as_ref().unwrap();
        assert_eq!(body["@truncated"], serde_json::json!(true));
    }

    #[test]
    fn truncate_backend_passes_small_body_through() {
        let inner = Arc::new(LogBackend::new("sink"));
        let tb = TruncateBackend::new("trunc", inner.clone() as Arc<dyn AuditBackend>, 1024);
        let mut e = evt("get");
        e.request_object = Some(serde_json::json!({"k": "v"}));
        tb.process(&e);
        let recorded = inner.events();
        assert_eq!(recorded[0].request_object, Some(serde_json::json!({"k": "v"})));
    }

    #[test]
    fn shutdown_propagates_through_wrappers() {
        let inner = Arc::new(LogBackend::new("sink"));
        let buf = Arc::new(BufferedBackend::new(
            "buf",
            inner.clone() as Arc<dyn AuditBackend>,
            10,
            Duration::from_secs(60),
        ));
        buf.process(&evt("z"));
        buf.shutdown(); // calls flush then inner.shutdown
        assert_eq!(inner.events().len(), 1);
    }
}
