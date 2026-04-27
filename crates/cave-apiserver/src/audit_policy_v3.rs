//! Audit backend dispatch — line-by-line port of upstream
//! `staging/src/k8s.io/apiserver/pkg/audit/`. Layered atop M4 audit_policy_v2.
//!
//! Provides:
//!   * `Backend` trait + `MultiBackend` fan-out.
//!   * `BatchBackend` with deadline + size flush.
//!   * `LogBackend` with file rotation by size + count.
//!   * `WebhookBackend` shape (sender wired #[ignore]).
//!
//! Tenant invariant: a backend MUST never blend events across tenants in
//! a single batch when the backend is shared by multiple tenants. Each
//! batch carries a single tenant_id; mixed-tenant batches are split.

use crate::audit::{AuditEvent, AuditStage};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
// Backend trait — `audit/types.go::Backend`. ProcessEvents is fire-and-forget;
// real backends fan out async. Returning the count is a deviation we use for
// observability.
// ─────────────────────────────────────────────────────────────────────────────

pub trait Backend: Send + Sync {
    fn process_events(&self, events: &[AuditEvent]) -> usize;
    fn run(&self) {} // long-running fn; default is no-op
    fn shutdown(&self) {}
    fn name(&self) -> &str { "anonymous" }
}

// ─────────────────────────────────────────────────────────────────────────────
// MultiBackend — fan-out helper, mirrors `union/union.go`.
// ─────────────────────────────────────────────────────────────────────────────

pub struct MultiBackend {
    pub backends: Vec<Arc<dyn Backend>>,
}

impl MultiBackend {
    pub fn new() -> Self { Self { backends: vec![] } }
    pub fn add(&mut self, b: Arc<dyn Backend>) { self.backends.push(b); }
    pub fn count(&self) -> usize { self.backends.len() }
}

impl Default for MultiBackend { fn default() -> Self { Self::new() } }

impl Backend for MultiBackend {
    fn name(&self) -> &str { "multi" }
    fn process_events(&self, events: &[AuditEvent]) -> usize {
        let mut total = 0;
        for b in &self.backends {
            total += b.process_events(events);
        }
        total
    }
    fn run(&self) { for b in &self.backends { b.run(); } }
    fn shutdown(&self) { for b in &self.backends { b.shutdown(); } }
}

// ─────────────────────────────────────────────────────────────────────────────
// BatchBackend — `buffered/buffered.go`. Buffers events until either
// `batch_size` is reached or `max_wait` elapses, then flushes downstream.
// Tenant invariant enforced: events are partitioned by tenant_id so a flush
// never mingles tenants.
// ─────────────────────────────────────────────────────────────────────────────

pub struct BatchConfig {
    pub batch_size: usize,
    pub max_wait: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self { Self { batch_size: 400, max_wait: Duration::from_secs(30) } }
}

pub struct BatchBackend {
    pub downstream: Arc<dyn Backend>,
    pub config: BatchConfig,
    inner: Mutex<BatchInner>,
}

struct BatchInner {
    queue: VecDeque<AuditEvent>,
    last_flush: Instant,
}

impl BatchBackend {
    pub fn new(downstream: Arc<dyn Backend>, config: BatchConfig) -> Self {
        Self {
            downstream, config,
            inner: Mutex::new(BatchInner {
                queue: VecDeque::new(),
                last_flush: Instant::now(),
            }),
        }
    }

    /// Force a flush, partitioning by tenant_id so no batch crosses tenants.
    pub fn flush(&self) -> usize {
        let mut g = self.inner.lock().unwrap();
        if g.queue.is_empty() { return 0; }
        let drained: Vec<AuditEvent> = g.queue.drain(..).collect();
        g.last_flush = Instant::now();
        drop(g);
        let mut by_tenant: std::collections::BTreeMap<String, Vec<AuditEvent>>
            = Default::default();
        for ev in drained {
            by_tenant.entry(ev.tenant_id.clone()).or_default().push(ev);
        }
        let mut total = 0;
        for (_t, batch) in by_tenant {
            total += self.downstream.process_events(&batch);
        }
        total
    }

    /// Returns true if a flush is now warranted (size or deadline).
    pub fn should_flush(&self) -> bool {
        let g = self.inner.lock().unwrap();
        g.queue.len() >= self.config.batch_size
            || g.last_flush.elapsed() >= self.config.max_wait
    }

    pub fn pending(&self) -> usize {
        self.inner.lock().unwrap().queue.len()
    }
}

impl Backend for BatchBackend {
    fn name(&self) -> &str { "batch" }
    fn process_events(&self, events: &[AuditEvent]) -> usize {
        let mut g = self.inner.lock().unwrap();
        for e in events { g.queue.push_back(e.clone()); }
        let need_flush = g.queue.len() >= self.config.batch_size
            || g.last_flush.elapsed() >= self.config.max_wait;
        drop(g);
        if need_flush { self.flush() } else { events.len() }
    }
    fn shutdown(&self) { let _ = self.flush(); }
}

// ─────────────────────────────────────────────────────────────────────────────
// LogBackend — file-style backend with size+count rotation. We hold the
// in-memory buffer + pretend-rotation index so tests can assert behavior.
// ─────────────────────────────────────────────────────────────────────────────

pub struct LogBackendConfig {
    pub max_bytes: u64,    // size that triggers rotation
    pub max_backups: usize, // backups kept (oldest pruned)
}

impl Default for LogBackendConfig {
    fn default() -> Self { Self { max_bytes: 100 * 1024 * 1024, max_backups: 10 } }
}

pub struct LogBackend {
    pub config: LogBackendConfig,
    inner: Mutex<LogInner>,
}

struct LogInner {
    /// Currently-active log buffer.
    active: Vec<AuditEvent>,
    active_bytes: u64,
    /// Each rotated file is a Vec<AuditEvent>; backups[0] is the most recent.
    backups: VecDeque<Vec<AuditEvent>>,
}

impl LogBackend {
    pub fn new(config: LogBackendConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(LogInner {
                active: vec![], active_bytes: 0, backups: VecDeque::new(),
            }),
        }
    }
    pub fn rotate(&self) {
        let mut g = self.inner.lock().unwrap();
        let drained = std::mem::take(&mut g.active);
        g.active_bytes = 0;
        if !drained.is_empty() {
            g.backups.push_front(drained);
            while g.backups.len() > self.config.max_backups {
                g.backups.pop_back();
            }
        }
    }
    pub fn active_count(&self) -> usize { self.inner.lock().unwrap().active.len() }
    pub fn backup_count(&self) -> usize { self.inner.lock().unwrap().backups.len() }
    pub fn total_count(&self) -> usize {
        let g = self.inner.lock().unwrap();
        g.active.len() + g.backups.iter().map(|b| b.len()).sum::<usize>()
    }
    pub fn active_bytes(&self) -> u64 { self.inner.lock().unwrap().active_bytes }
}

fn estimate_event_bytes(e: &AuditEvent) -> u64 {
    serde_json::to_vec(e).map(|v| v.len() as u64).unwrap_or(256)
}

impl Backend for LogBackend {
    fn name(&self) -> &str { "log" }
    fn process_events(&self, events: &[AuditEvent]) -> usize {
        let mut written = 0;
        for e in events {
            let mut g = self.inner.lock().unwrap();
            let size = estimate_event_bytes(e);
            if g.active_bytes + size > self.config.max_bytes
                && self.config.max_bytes > 0
            {
                drop(g);
                self.rotate();
                let mut g = self.inner.lock().unwrap();
                g.active.push(e.clone());
                g.active_bytes = size;
            } else {
                g.active.push(e.clone());
                g.active_bytes += size;
            }
            written += 1;
        }
        written
    }
    fn shutdown(&self) {} // file would be flushed; in-memory is no-op
}

// ─────────────────────────────────────────────────────────────────────────────
// WebhookBackend — fan-out to a remote audit sink. Real HTTPS is #[ignore]'d;
// we model the success/error categorization.
// ─────────────────────────────────────────────────────────────────────────────

pub trait AuditSinkClient: Send + Sync {
    fn send(&self, events: &[AuditEvent]) -> Result<(), String>;
}

pub struct WebhookBackend {
    pub client: Arc<dyn AuditSinkClient>,
    pub failure_policy: WebhookFailurePolicy,
    inner: Mutex<WebhookInner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookFailurePolicy {
    /// Drop events on send failure (default for audit).
    Drop,
    /// Re-queue for next call (bounded by `max_pending`).
    Retry { max_pending: usize },
}

#[derive(Default)]
struct WebhookInner {
    pending: VecDeque<AuditEvent>,
    successes: u64,
    drops: u64,
}

impl WebhookBackend {
    pub fn new(client: Arc<dyn AuditSinkClient>, failure_policy: WebhookFailurePolicy) -> Self {
        Self { client, failure_policy, inner: Mutex::new(WebhookInner::default()) }
    }
    pub fn drops(&self) -> u64 { self.inner.lock().unwrap().drops }
    pub fn successes(&self) -> u64 { self.inner.lock().unwrap().successes }
    pub fn pending(&self) -> usize { self.inner.lock().unwrap().pending.len() }
}

impl Backend for WebhookBackend {
    fn name(&self) -> &str { "webhook" }
    fn process_events(&self, events: &[AuditEvent]) -> usize {
        let mut to_send: Vec<AuditEvent> = events.iter().cloned().collect();
        // include any pending from previous failures
        let mut g = self.inner.lock().unwrap();
        for e in g.pending.drain(..) { to_send.push(e); }
        drop(g);
        match self.client.send(&to_send) {
            Ok(()) => {
                let mut g = self.inner.lock().unwrap();
                g.successes += to_send.len() as u64;
                to_send.len()
            }
            Err(_) => {
                let mut g = self.inner.lock().unwrap();
                match self.failure_policy {
                    WebhookFailurePolicy::Drop => {
                        g.drops += to_send.len() as u64;
                    }
                    WebhookFailurePolicy::Retry { max_pending } => {
                        for e in to_send {
                            if g.pending.len() < max_pending {
                                g.pending.push_back(e);
                            } else {
                                g.drops += 1;
                            }
                        }
                    }
                }
                0
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Event redaction — `policy.go::sanitize`. Strips request/response bodies,
// scrubs known-secret kinds (Secret), enforces tenant_id label injection.
// ─────────────────────────────────────────────────────────────────────────────

pub fn inject_tenant_label(event: &mut AuditEvent) {
    event.annotations.insert(
        "audit.k8s.io/tenant-id".into(),
        event.tenant_id.clone(),
    );
}

pub fn scrub_secret_payloads(event: &mut AuditEvent) {
    if event.resource == "secrets" || event.resource == "Secret" {
        event.request_object = None;
        event.response_object = None;
    }
}

pub fn redact_for_stage(event: &mut AuditEvent) {
    if matches!(event.stage, AuditStage::RequestReceived) {
        // RequestReceived has no response body to begin with; nothing to do.
        event.response_object = None;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Counting backend — TEST helper for tenant-isolation assertions.
// ─────────────────────────────────────────────────────────────────────────────

pub struct CountingBackend {
    inner: Mutex<CountingInner>,
}

#[derive(Default)]
struct CountingInner {
    /// Records the (tenant_id, count) of each call to process_events.
    calls: Vec<(String, usize)>,
    total: usize,
}

impl CountingBackend {
    pub fn new() -> Self {
        Self { inner: Mutex::new(CountingInner::default()) }
    }
    pub fn calls(&self) -> Vec<(String, usize)> {
        self.inner.lock().unwrap().calls.clone()
    }
    pub fn total(&self) -> usize { self.inner.lock().unwrap().total }
}

impl Default for CountingBackend { fn default() -> Self { Self::new() } }

impl Backend for CountingBackend {
    fn name(&self) -> &str { "counting" }
    fn process_events(&self, events: &[AuditEvent]) -> usize {
        let mut g = self.inner.lock().unwrap();
        if let Some(first) = events.first() {
            g.calls.push((first.tenant_id.clone(), events.len()));
        }
        g.total += events.len();
        events.len()
    }
}

#[cfg(test)]
mod tests;
