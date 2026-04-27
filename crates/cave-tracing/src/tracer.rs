//! Tracer + TracerProvider + Span SDK.
//!
//! Public surface:
//!   provider.tracer("scope") → Tracer
//!   tracer.span_builder("name").with_kind(...).start_with_parent(...) → Span
//!   span.set_attribute(...) / .add_event(...) / .set_status(...) / .end()
//!
//! On end, the span is consumed and the SDK invokes the configured
//! BatchSpanProcessor (or another `SpanProcessor` impl).

use crate::batch::BatchSpanProcessor;
use crate::id::{new_span_id, new_trace_id};
use crate::sampling::{AlwaysOn, Sampler, SamplingDecision};
use crate::types::*;
use chrono::Utc;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

/// Anything that can receive completed spans. The SDK has one built-in
/// implementation (`BatchSpanProcessor`) but tests use `InMemoryProcessor`.
pub trait SpanProcessor: Send + Sync {
    fn on_end(&self, span: SpanData);
}

impl SpanProcessor for BatchSpanProcessor {
    fn on_end(&self, span: SpanData) {
        BatchSpanProcessor::on_end(self, span)
    }
}

/// Test-only processor that buffers spans synchronously.
#[derive(Debug, Default, Clone)]
pub struct InMemoryProcessor {
    inner: Arc<Mutex<Vec<SpanData>>>,
}

impl InMemoryProcessor {
    pub fn new() -> Self { Default::default() }
    pub fn collected(&self) -> Vec<SpanData> { self.inner.lock().clone() }
    pub fn count(&self) -> usize { self.inner.lock().len() }
}

impl SpanProcessor for InMemoryProcessor {
    fn on_end(&self, span: SpanData) {
        self.inner.lock().push(span);
    }
}

// ─── TracerProvider ────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TracerProvider {
    inner: Arc<TracerProviderInner>,
}

struct TracerProviderInner {
    sampler: Arc<dyn Sampler>,
    processors: Vec<Arc<dyn SpanProcessor>>,
    resource: HashMap<String, String>,
    tenant: parking_lot::RwLock<String>,
}

impl TracerProvider {
    pub fn builder() -> TracerProviderBuilder { TracerProviderBuilder::default() }

    pub fn tracer(&self, scope: impl Into<String>) -> Tracer {
        Tracer { provider: self.clone(), scope: scope.into() }
    }

    pub fn set_tenant(&self, tenant: impl Into<String>) {
        *self.inner.tenant.write() = tenant.into();
    }

    pub fn current_tenant(&self) -> String {
        self.inner.tenant.read().clone()
    }
}

#[derive(Default)]
pub struct TracerProviderBuilder {
    sampler: Option<Arc<dyn Sampler>>,
    processors: Vec<Arc<dyn SpanProcessor>>,
    resource: HashMap<String, String>,
    tenant: Option<String>,
}

impl TracerProviderBuilder {
    pub fn with_sampler(mut self, s: Arc<dyn Sampler>) -> Self {
        self.sampler = Some(s); self
    }
    pub fn with_processor(mut self, p: Arc<dyn SpanProcessor>) -> Self {
        self.processors.push(p); self
    }
    pub fn with_resource(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.resource.insert(k.into(), v.into()); self
    }
    pub fn with_tenant(mut self, t: impl Into<String>) -> Self {
        self.tenant = Some(t.into()); self
    }
    pub fn build(self) -> TracerProvider {
        TracerProvider {
            inner: Arc::new(TracerProviderInner {
                sampler: self.sampler.unwrap_or_else(|| Arc::new(AlwaysOn)),
                processors: self.processors,
                resource: self.resource,
                tenant: parking_lot::RwLock::new(
                    self.tenant.unwrap_or_else(|| DEFAULT_TENANT.to_string()),
                ),
            }),
        }
    }
}

// ─── Tracer ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Tracer {
    provider: TracerProvider,
    scope: String,
}

impl Tracer {
    pub fn span_builder(&self, name: impl Into<String>) -> SpanBuilder {
        SpanBuilder {
            tracer: self.clone(),
            name: name.into(),
            kind: SpanKind::Internal,
            attributes: Attributes::new(),
            links: Vec::new(),
        }
    }
}

// ─── SpanBuilder ───────────────────────────────────────────────────────────

pub struct SpanBuilder {
    tracer: Tracer,
    name: String,
    kind: SpanKind,
    attributes: Attributes,
    links: Vec<Link>,
}

impl SpanBuilder {
    pub fn with_kind(mut self, k: SpanKind) -> Self { self.kind = k; self }
    pub fn with_attribute<V: Into<AttrValue>>(mut self, k: impl Into<String>, v: V) -> Self {
        self.attributes.insert(k.into(), v.into()); self
    }
    pub fn with_link(mut self, l: Link) -> Self { self.links.push(l); self }

    /// Start a span with no parent (root).
    pub fn start(self) -> Span {
        self.start_with_parent(None)
    }

    /// Start a span beneath a (possibly remote) parent context.
    pub fn start_with_parent(self, parent: Option<&SpanContext>) -> Span {
        let trace_id = parent.map(|p| p.trace_id).unwrap_or_else(new_trace_id);
        let span_id = new_span_id();

        let sampler = &self.tracer.provider.inner.sampler;
        let result = sampler.should_sample(parent, trace_id, &self.name, self.kind, &self.attributes);

        let context = SpanContext {
            trace_id,
            span_id,
            trace_flags: result.trace_flags,
            is_remote: false,
        };

        // Merge sampler-provided attributes with builder-provided
        let mut attrs = self.attributes;
        for (k, v) in result.attributes {
            attrs.entry(k).or_insert(v);
        }

        let recording = result.decision.is_recording();
        Span {
            tracer: self.tracer,
            data: Some(SpanData {
                name: self.name,
                context,
                parent_span_id: parent.and_then(|p| if p.is_valid() { Some(p.span_id) } else { None }),
                kind: self.kind,
                start_time: Utc::now(),
                end_time: Utc::now(), // overwritten in end()
                attributes: attrs,
                events: vec![],
                links: self.links,
                status: Status::Unset,
                instrumentation_scope: String::new(), // filled below
                tenant_id: String::new(),
                resource: HashMap::new(),
            }),
            recording,
            sampled: result.decision.is_sampled(),
        }
    }
}

// ─── Span ──────────────────────────────────────────────────────────────────

pub struct Span {
    tracer: Tracer,
    data: Option<SpanData>,
    recording: bool,
    sampled: bool,
}

impl Span {
    pub fn context(&self) -> SpanContext {
        self.data.as_ref().map(|d| d.context).unwrap_or_else(SpanContext::invalid)
    }
    pub fn is_recording(&self) -> bool { self.recording }
    pub fn is_sampled(&self) -> bool { self.sampled }

    pub fn set_attribute<V: Into<AttrValue>>(&mut self, k: impl Into<String>, v: V) {
        if !self.recording { return; }
        if let Some(d) = &mut self.data {
            d.attributes.insert(k.into(), v.into());
        }
    }

    pub fn add_event(&mut self, name: impl Into<String>, attributes: Attributes) {
        if !self.recording { return; }
        if let Some(d) = &mut self.data {
            d.events.push(Event {
                name: name.into(),
                time: Utc::now(),
                attributes,
            });
        }
    }

    pub fn set_status(&mut self, status: Status) {
        if !self.recording { return; }
        if let Some(d) = &mut self.data {
            // OTel: never demote Ok back to Unset; never overwrite Error from non-Error
            match (&d.status, &status) {
                (Status::Error(_), Status::Ok) | (Status::Error(_), Status::Unset) => {}
                _ => d.status = status,
            }
        }
    }

    pub fn record_error(&mut self, message: impl Into<String>) {
        self.set_status(Status::Error(message.into()));
    }

    /// End the span — finalizes timing, attaches resource + tenant + scope,
    /// and dispatches to all registered SpanProcessors.
    pub fn end(mut self) {
        if let Some(mut d) = self.data.take() {
            d.end_time = Utc::now();
            d.instrumentation_scope = self.tracer.scope.clone();
            d.tenant_id = self.tracer.provider.current_tenant();
            d.resource = self.tracer.provider.inner.resource.clone();
            d.resource.insert(TENANT_LABEL.into(), d.tenant_id.clone());

            if !self.recording {
                return; // drop entirely
            }
            for p in self.tracer.provider.inner.processors.iter() {
                p.on_end(d.clone());
            }
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        // If the user forgot to call end(), still flush so spans aren't lost.
        if let Some(mut d) = self.data.take() {
            if !self.recording { return; }
            d.end_time = Utc::now();
            d.instrumentation_scope = self.tracer.scope.clone();
            d.tenant_id = self.tracer.provider.current_tenant();
            d.resource = self.tracer.provider.inner.resource.clone();
            d.resource.insert(TENANT_LABEL.into(), d.tenant_id.clone());
            for p in self.tracer.provider.inner.processors.iter() {
                p.on_end(d.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::{AlwaysOff, AlwaysOn, ParentBased, TraceIdRatioBased};

    fn provider(p: Arc<InMemoryProcessor>) -> TracerProvider {
        TracerProvider::builder()
            .with_processor(p as Arc<dyn SpanProcessor>)
            .build()
    }

    #[test]
    fn test_root_span_emits_to_processor() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("scope");
        tr.span_builder("op").start().end();
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn test_span_carries_scope_and_kind() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("scope-x");
        tr.span_builder("rpc").with_kind(SpanKind::Server).start().end();
        let s = &p.collected()[0];
        assert_eq!(s.instrumentation_scope, "scope-x");
        assert_eq!(s.kind, SpanKind::Server);
    }

    #[test]
    fn test_attribute_set_and_seen_in_data() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("s");
        let mut span = tr.span_builder("op").start();
        span.set_attribute("http.status", 200i64);
        span.set_attribute("user", "alice");
        span.end();
        let d = &p.collected()[0];
        assert_eq!(d.attributes.get("user"), Some(&AttrValue::String("alice".into())));
        assert_eq!(d.attributes.get("http.status"), Some(&AttrValue::Int(200)));
    }

    #[test]
    fn test_event_added() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("s");
        let mut span = tr.span_builder("op").start();
        let mut ev_attrs = Attributes::new();
        ev_attrs.insert("retry".into(), AttrValue::Int(2));
        span.add_event("retry", ev_attrs);
        span.end();
        assert_eq!(p.collected()[0].events.len(), 1);
    }

    #[test]
    fn test_status_set_to_error() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("s");
        let mut span = tr.span_builder("op").start();
        span.set_status(Status::Error("boom".into()));
        span.end();
        assert_eq!(p.collected()[0].status, Status::Error("boom".into()));
    }

    #[test]
    fn test_status_error_not_demoted_by_ok() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("s");
        let mut span = tr.span_builder("op").start();
        span.set_status(Status::Error("first".into()));
        span.set_status(Status::Ok);
        span.end();
        // Error sticks
        assert!(matches!(p.collected()[0].status, Status::Error(_)));
    }

    #[test]
    fn test_span_drop_still_flushes() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("s");
        let _ = tr.span_builder("op").start(); // dropped without end()
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn test_alwaysoff_sampler_drops_span() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = TracerProvider::builder()
            .with_sampler(Arc::new(AlwaysOff))
            .with_processor(p.clone() as Arc<dyn SpanProcessor>)
            .build();
        let tr = tp.tracer("s");
        let span = tr.span_builder("op").start();
        assert!(!span.is_recording());
        span.end();
        assert_eq!(p.count(), 0);
    }

    #[test]
    fn test_root_span_inherits_no_parent() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let span = tp.tracer("s").span_builder("op").start();
        assert!(span.context().is_valid());
        // parent_span_id is None on root
        let ctx = span.context();
        span.end();
        let d = &p.collected()[0];
        assert_eq!(d.parent_span_id, None);
        assert_eq!(d.context.trace_id, ctx.trace_id);
    }

    #[test]
    fn test_child_span_inherits_trace_id_from_parent() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("s");
        let parent_ctx = SpanContext::new(0xdeadbeef_cafe_babe_dead_beef_cafe_babe, 0xfeedface, true);
        let child = tr.span_builder("child").start_with_parent(Some(&parent_ctx));
        let cctx = child.context();
        child.end();
        assert_eq!(cctx.trace_id, parent_ctx.trace_id);
        assert_ne!(cctx.span_id, parent_ctx.span_id);
        let d = &p.collected()[0];
        assert_eq!(d.parent_span_id, Some(parent_ctx.span_id));
    }

    #[test]
    fn test_remote_unsampled_parent_under_parent_based_sampler_drops() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = TracerProvider::builder()
            .with_sampler(Arc::new(ParentBased::new(Arc::new(AlwaysOn))))
            .with_processor(p.clone() as Arc<dyn SpanProcessor>)
            .build();
        let mut parent = SpanContext::new(0xaa, 0xbb, false);
        parent.is_remote = true;
        let span = tp.tracer("s").span_builder("c").start_with_parent(Some(&parent));
        assert!(!span.is_sampled());
        span.end();
        assert_eq!(p.count(), 0);
    }

    #[test]
    fn test_tenant_resource_attached() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = TracerProvider::builder()
            .with_tenant("acme")
            .with_processor(p.clone() as Arc<dyn SpanProcessor>)
            .build();
        tp.tracer("s").span_builder("op").start().end();
        let d = &p.collected()[0];
        assert_eq!(d.tenant_id, "acme");
        assert_eq!(d.resource.get(TENANT_LABEL), Some(&"acme".to_string()));
    }

    #[test]
    fn test_resource_keys_attached() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = TracerProvider::builder()
            .with_resource("service.name", "api")
            .with_resource("service.version", "1.2.3")
            .with_processor(p.clone() as Arc<dyn SpanProcessor>)
            .build();
        tp.tracer("s").span_builder("op").start().end();
        let d = &p.collected()[0];
        assert_eq!(d.resource.get("service.name"), Some(&"api".to_string()));
        assert_eq!(d.resource.get("service.version"), Some(&"1.2.3".to_string()));
    }

    #[test]
    fn test_links_attached() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let tr = tp.tracer("s");
        let linked = SpanContext::new(0x1122, 0x3344, true);
        tr.span_builder("op")
            .with_link(Link { context: linked, attributes: Attributes::new() })
            .start()
            .end();
        let d = &p.collected()[0];
        assert_eq!(d.links.len(), 1);
        assert_eq!(d.links[0].context, linked);
    }

    #[test]
    fn test_set_tenant_after_build_takes_effect() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        tp.set_tenant("globex");
        tp.tracer("s").span_builder("op").start().end();
        assert_eq!(p.collected()[0].tenant_id, "globex");
    }

    #[test]
    fn test_record_error_sets_error_status() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        let mut s = tp.tracer("s").span_builder("op").start();
        s.record_error("network");
        s.end();
        assert_eq!(p.collected()[0].status, Status::Error("network".into()));
    }

    #[test]
    fn test_ratio_zero_drops_through_provider() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = TracerProvider::builder()
            .with_sampler(Arc::new(TraceIdRatioBased::new(0.0)))
            .with_processor(p.clone() as Arc<dyn SpanProcessor>)
            .build();
        for _ in 0..20 {
            tp.tracer("s").span_builder("op").start().end();
        }
        assert_eq!(p.count(), 0);
    }

    #[test]
    fn test_multiple_processors_all_invoked() {
        let p1 = Arc::new(InMemoryProcessor::new());
        let p2 = Arc::new(InMemoryProcessor::new());
        let tp = TracerProvider::builder()
            .with_processor(p1.clone() as Arc<dyn SpanProcessor>)
            .with_processor(p2.clone() as Arc<dyn SpanProcessor>)
            .build();
        tp.tracer("s").span_builder("op").start().end();
        assert_eq!(p1.count(), 1);
        assert_eq!(p2.count(), 1);
    }

    #[test]
    fn test_attribute_skipped_when_not_recording() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = TracerProvider::builder()
            .with_sampler(Arc::new(AlwaysOff))
            .with_processor(p.clone() as Arc<dyn SpanProcessor>)
            .build();
        let mut s = tp.tracer("s").span_builder("op").start();
        s.set_attribute("k", "v"); // ignored
        s.end();
        assert_eq!(p.count(), 0);
    }

    #[test]
    fn test_default_tenant_when_unset() {
        let p = Arc::new(InMemoryProcessor::new());
        let tp = provider(p.clone());
        tp.tracer("s").span_builder("op").start().end();
        assert_eq!(p.collected()[0].tenant_id, DEFAULT_TENANT);
    }
}
