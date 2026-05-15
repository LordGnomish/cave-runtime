// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-invocation telemetry for `cavectl`.
//!
//! Captures verb, surface (native/compat), tenant scope, exit status,
//! and duration. The sink is pluggable: production code writes to
//! cave's metrics endpoint; tests use `InMemorySink` to assert on the
//! event stream without I/O.

use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub verb: String,
    pub surface: Surface,
    pub tenant: Option<String>,
    pub status: Status,
    pub duration_ms: u128,
    /// Free-form attributes (e.g. `resource=pods`, `namespace=prod`).
    pub attrs: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Native,
    Compat,
}

impl Surface {
    pub fn as_str(&self) -> &'static str {
        match self {
            Surface::Native => "native",
            Surface::Compat => "compat",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Error,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Error => "error",
        }
    }
}

pub trait Sink: Send + Sync {
    fn record(&self, event: Event);
}

/// In-memory sink for tests.
#[derive(Default)]
pub struct InMemorySink {
    events: Mutex<Vec<Event>>,
}

impl InMemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn last(&self) -> Option<Event> {
        self.events.lock().unwrap().last().cloned()
    }
}

impl Sink for InMemorySink {
    fn record(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }
}

/// Sink that drops every event. Useful when telemetry is opted out.
pub struct NullSink;

impl Sink for NullSink {
    fn record(&self, _: Event) {}
}

/// Builder for an `Event`. The verb is mandatory; everything else has
/// reasonable defaults.
pub struct EventBuilder {
    verb: String,
    surface: Surface,
    tenant: Option<String>,
    status: Status,
    duration: Duration,
    attrs: Vec<(String, String)>,
}

impl EventBuilder {
    pub fn new(verb: impl Into<String>) -> Self {
        Self {
            verb: verb.into(),
            surface: Surface::Native,
            tenant: None,
            status: Status::Ok,
            duration: Duration::ZERO,
            attrs: Vec::new(),
        }
    }

    pub fn surface(mut self, s: Surface) -> Self {
        self.surface = s;
        self
    }

    pub fn tenant(mut self, t: impl Into<String>) -> Self {
        self.tenant = Some(t.into());
        self
    }

    pub fn status(mut self, s: Status) -> Self {
        self.status = s;
        self
    }

    pub fn duration(mut self, d: Duration) -> Self {
        self.duration = d;
        self
    }

    pub fn attr(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.attrs.push((k.into(), v.into()));
        self
    }

    pub fn build(self) -> Event {
        Event {
            verb: self.verb,
            surface: self.surface,
            tenant: self.tenant,
            status: self.status,
            duration_ms: self.duration.as_millis(),
            attrs: self.attrs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_minimal() {
        let e = EventBuilder::new("get").build();
        assert_eq!(e.verb, "get");
        assert_eq!(e.surface, Surface::Native);
        assert_eq!(e.status, Status::Ok);
        assert_eq!(e.duration_ms, 0);
        assert!(e.tenant.is_none());
        assert!(e.attrs.is_empty());
    }

    #[test]
    fn builder_full() {
        let e = EventBuilder::new("kubectl")
            .surface(Surface::Compat)
            .tenant("acme")
            .status(Status::Error)
            .duration(Duration::from_millis(123))
            .attr("resource", "pods")
            .attr("namespace", "prod")
            .build();
        assert_eq!(e.verb, "kubectl");
        assert_eq!(e.surface, Surface::Compat);
        assert_eq!(e.tenant.as_deref(), Some("acme"));
        assert_eq!(e.status, Status::Error);
        assert_eq!(e.duration_ms, 123);
        assert_eq!(e.attrs.len(), 2);
    }

    #[test]
    fn surface_as_str() {
        assert_eq!(Surface::Native.as_str(), "native");
        assert_eq!(Surface::Compat.as_str(), "compat");
    }

    #[test]
    fn status_as_str() {
        assert_eq!(Status::Ok.as_str(), "ok");
        assert_eq!(Status::Error.as_str(), "error");
    }

    #[test]
    fn in_memory_sink_records() {
        let sink = InMemorySink::new();
        assert!(sink.is_empty());
        sink.record(EventBuilder::new("get").build());
        assert_eq!(sink.len(), 1);
        assert!(!sink.is_empty());
    }

    #[test]
    fn in_memory_sink_records_in_order() {
        let sink = InMemorySink::new();
        sink.record(EventBuilder::new("a").build());
        sink.record(EventBuilder::new("b").build());
        sink.record(EventBuilder::new("c").build());
        let evs = sink.events();
        assert_eq!(evs[0].verb, "a");
        assert_eq!(evs[1].verb, "b");
        assert_eq!(evs[2].verb, "c");
    }

    #[test]
    fn in_memory_sink_last() {
        let sink = InMemorySink::new();
        sink.record(EventBuilder::new("first").build());
        sink.record(EventBuilder::new("last").build());
        assert_eq!(sink.last().unwrap().verb, "last");
    }

    #[test]
    fn in_memory_sink_last_empty() {
        assert!(InMemorySink::new().last().is_none());
    }

    #[test]
    fn null_sink_drops() {
        let n = NullSink;
        n.record(EventBuilder::new("x").build());
        // Just confirm it doesn't panic.
    }

    #[test]
    fn duration_zero_default() {
        let e = EventBuilder::new("x").build();
        assert_eq!(e.duration_ms, 0);
    }

    #[test]
    fn duration_milliseconds() {
        let e = EventBuilder::new("x")
            .duration(Duration::from_secs(2))
            .build();
        assert_eq!(e.duration_ms, 2000);
    }

    #[test]
    fn attr_order_preserved() {
        let e = EventBuilder::new("x")
            .attr("a", "1")
            .attr("b", "2")
            .attr("c", "3")
            .build();
        assert_eq!(e.attrs[0].0, "a");
        assert_eq!(e.attrs[2].0, "c");
    }

    #[test]
    fn surface_compat_propagates() {
        let e = EventBuilder::new("kubectl")
            .surface(Surface::Compat)
            .build();
        assert_eq!(e.surface, Surface::Compat);
    }

    #[test]
    fn status_error_propagates() {
        let e = EventBuilder::new("x").status(Status::Error).build();
        assert_eq!(e.status, Status::Error);
    }

    #[test]
    fn sink_through_dyn_trait() {
        let sink: Box<dyn Sink> = Box::new(InMemorySink::new());
        sink.record(EventBuilder::new("x").build());
        // Trait obj doesn't expose len; just confirm no compile/panic.
    }

    #[test]
    fn multiple_events_with_same_verb() {
        let sink = InMemorySink::new();
        for _ in 0..10 {
            sink.record(EventBuilder::new("get").build());
        }
        assert_eq!(sink.len(), 10);
    }

    #[test]
    fn event_clone_eq() {
        let e1 = EventBuilder::new("x").attr("k", "v").build();
        let e2 = e1.clone();
        assert_eq!(e1, e2);
    }

    #[test]
    fn surface_eq_distinct() {
        assert_ne!(Surface::Native, Surface::Compat);
    }

    #[test]
    fn status_eq_distinct() {
        assert_ne!(Status::Ok, Status::Error);
    }

    #[test]
    fn null_sink_does_not_grow_in_memory() {
        // Behavioural: there's no list to grow. Just confirm many calls.
        let n: Box<dyn Sink> = Box::new(NullSink);
        for _ in 0..1000 {
            n.record(EventBuilder::new("x").build());
        }
    }

    #[test]
    fn tenant_attr_when_set() {
        let e = EventBuilder::new("get").tenant("acme").build();
        assert_eq!(e.tenant.as_deref(), Some("acme"));
    }
}
