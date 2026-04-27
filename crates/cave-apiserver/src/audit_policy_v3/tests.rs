//! audit_policy_v3 tests — backend dispatch, batch, log rotation, webhook,
//! redaction.

use super::*;
use crate::audit::{AuditEvent, AuditLevel, AuditStage};
use std::sync::Arc;
use std::thread;

fn ev(tenant: &str, resource: &str, stage: AuditStage) -> AuditEvent {
    AuditEvent::new(
        format!("aid-{tenant}-{resource}"),
        AuditLevel::Metadata, stage,
        "alice", tenant, "ns",
        "create", resource, "obj",
        "/api/v1/...", 200,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MultiBackend
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn multi_fanout_to_all_backends() {
    let a = Arc::new(CountingBackend::new());
    let b = Arc::new(CountingBackend::new());
    let mut m = MultiBackend::new();
    m.add(a.clone()); m.add(b.clone());
    let events = vec![ev("acme", "pods", AuditStage::ResponseComplete)];
    m.process_events(&events);
    assert_eq!(a.total(), 1);
    assert_eq!(b.total(), 1);
}

#[test]
fn multi_count_returns_sum() {
    let a = Arc::new(CountingBackend::new());
    let mut m = MultiBackend::new();
    m.add(a.clone()); m.add(a.clone());
    let n = m.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(n, 2, "two backends each consumed one event");
}

#[test]
fn multi_empty_is_noop() {
    let m = MultiBackend::new();
    assert_eq!(m.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]), 0);
}

#[test]
fn multi_count_initial_zero() {
    assert_eq!(MultiBackend::new().count(), 0);
}

#[test]
fn multi_add_increments() {
    let mut m = MultiBackend::new();
    m.add(Arc::new(CountingBackend::new()));
    m.add(Arc::new(CountingBackend::new()));
    assert_eq!(m.count(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// BatchBackend
// ─────────────────────────────────────────────────────────────────────────────

fn batch(downstream: Arc<dyn Backend>, batch_size: usize, max_wait_ms: u64) -> BatchBackend {
    BatchBackend::new(downstream, BatchConfig {
        batch_size,
        max_wait: std::time::Duration::from_millis(max_wait_ms),
    })
}

#[test]
fn batch_buffers_under_size() {
    let down = Arc::new(CountingBackend::new());
    let b = batch(down.clone(), 5, 60_000);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(down.total(), 0, "downstream not yet flushed");
    assert_eq!(b.pending(), 2);
}

#[test]
fn batch_flushes_at_size() {
    let down = Arc::new(CountingBackend::new());
    let b = batch(down.clone(), 2, 60_000);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(down.total(), 2, "downstream got the flushed batch");
    assert_eq!(b.pending(), 0);
}

#[test]
fn batch_flush_partitions_by_tenant() {
    let down = Arc::new(CountingBackend::new());
    let b = batch(down.clone(), 100, 60_000);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    b.process_events(&[ev("globex", "pods", AuditStage::ResponseComplete)]);
    b.process_events(&[ev("acme", "secrets", AuditStage::ResponseComplete)]);
    let _ = b.flush();
    let calls = down.calls();
    assert_eq!(calls.len(), 2, "two tenants → two batches");
    let tenants: Vec<&String> = calls.iter().map(|(t, _)| t).collect();
    assert!(tenants.contains(&&"acme".to_string()));
    assert!(tenants.contains(&&"globex".to_string()));
}

#[test]
fn batch_explicit_flush_clears_queue() {
    let down = Arc::new(CountingBackend::new());
    let b = batch(down.clone(), 100, 60_000);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    b.flush();
    assert_eq!(b.pending(), 0);
    assert_eq!(down.total(), 1);
}

#[test]
fn batch_flush_empty_is_zero() {
    let down = Arc::new(CountingBackend::new());
    let b = batch(down, 100, 60_000);
    assert_eq!(b.flush(), 0);
}

#[test]
fn batch_should_flush_after_deadline() {
    let down = Arc::new(CountingBackend::new());
    let b = batch(down, 100, 5);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    thread::sleep(std::time::Duration::from_millis(20));
    assert!(b.should_flush());
}

#[test]
fn batch_shutdown_flushes() {
    let down = Arc::new(CountingBackend::new());
    let b = batch(down.clone(), 100, 60_000);
    b.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    b.shutdown();
    assert_eq!(down.total(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// LogBackend
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn log_writes_to_active() {
    let log = LogBackend::new(LogBackendConfig::default());
    log.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(log.active_count(), 1);
    assert_eq!(log.backup_count(), 0);
}

#[test]
fn log_explicit_rotation_moves_to_backups() {
    let log = LogBackend::new(LogBackendConfig::default());
    log.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    log.rotate();
    assert_eq!(log.active_count(), 0);
    assert_eq!(log.backup_count(), 1);
    assert_eq!(log.total_count(), 1);
}

#[test]
fn log_backups_capped_at_max_backups() {
    let log = LogBackend::new(LogBackendConfig {
        max_bytes: u64::MAX, max_backups: 2,
    });
    for i in 0..5 {
        log.process_events(&[ev("acme", &format!("r{i}"), AuditStage::ResponseComplete)]);
        log.rotate();
    }
    assert_eq!(log.backup_count(), 2, "max_backups=2 caps backups");
}

#[test]
fn log_size_triggers_auto_rotation() {
    let log = LogBackend::new(LogBackendConfig {
        max_bytes: 100, max_backups: 5,
    });
    for i in 0..10 {
        log.process_events(&[ev("acme", &format!("kind{i}"), AuditStage::ResponseComplete)]);
    }
    assert!(log.backup_count() > 0,
        "size cap of 100 bytes must trigger rotation across 10 events");
}

#[test]
fn log_rotation_resets_active_bytes() {
    let log = LogBackend::new(LogBackendConfig {
        max_bytes: u64::MAX, max_backups: 5,
    });
    log.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    let before = log.active_bytes();
    assert!(before > 0);
    log.rotate();
    assert_eq!(log.active_bytes(), 0);
}

#[test]
fn log_total_count_sums_active_and_backups() {
    let log = LogBackend::new(LogBackendConfig {
        max_bytes: u64::MAX, max_backups: 5,
    });
    log.process_events(&[ev("acme", "a", AuditStage::ResponseComplete)]);
    log.rotate();
    log.process_events(&[ev("acme", "b", AuditStage::ResponseComplete)]);
    log.process_events(&[ev("acme", "c", AuditStage::ResponseComplete)]);
    assert_eq!(log.total_count(), 3);
}

// ─────────────────────────────────────────────────────────────────────────────
// WebhookBackend
// ─────────────────────────────────────────────────────────────────────────────

struct AlwaysOk;
impl AuditSinkClient for AlwaysOk {
    fn send(&self, _: &[AuditEvent]) -> Result<(), String> { Ok(()) }
}

struct AlwaysFail;
impl AuditSinkClient for AlwaysFail {
    fn send(&self, _: &[AuditEvent]) -> Result<(), String> { Err("nope".into()) }
}

#[test]
fn webhook_success_increments_counter() {
    let w = WebhookBackend::new(Arc::new(AlwaysOk), WebhookFailurePolicy::Drop);
    w.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(w.successes(), 1);
    assert_eq!(w.drops(), 0);
}

#[test]
fn webhook_drop_policy_drops_on_error() {
    let w = WebhookBackend::new(Arc::new(AlwaysFail), WebhookFailurePolicy::Drop);
    w.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(w.drops(), 1);
    assert_eq!(w.pending(), 0);
}

#[test]
fn webhook_retry_policy_queues_failures() {
    let w = WebhookBackend::new(Arc::new(AlwaysFail),
        WebhookFailurePolicy::Retry { max_pending: 5 });
    w.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(w.pending(), 1);
    assert_eq!(w.drops(), 0);
}

#[test]
fn webhook_retry_caps_pending() {
    let w = WebhookBackend::new(Arc::new(AlwaysFail),
        WebhookFailurePolicy::Retry { max_pending: 2 });
    for _ in 0..5 {
        w.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    }
    assert_eq!(w.pending(), 2);
    assert_eq!(w.drops(), 3);
}

#[test]
fn webhook_drains_pending_on_next_success() {
    struct ToggleClient { count: Mutex<u32> }
    impl AuditSinkClient for ToggleClient {
        fn send(&self, _: &[AuditEvent]) -> Result<(), String> {
            let mut c = self.count.lock().unwrap();
            *c += 1;
            if *c == 1 { Err("first call fails".into()) } else { Ok(()) }
        }
    }
    use std::sync::Mutex;
    let client = Arc::new(ToggleClient { count: Mutex::new(0) });
    let w = WebhookBackend::new(client.clone(),
        WebhookFailurePolicy::Retry { max_pending: 5 });
    w.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    assert_eq!(w.pending(), 1);
    w.process_events(&[ev("acme", "more", AuditStage::ResponseComplete)]);
    assert_eq!(w.pending(), 0, "retry drains on next successful send");
    assert_eq!(w.successes(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// Redaction
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn inject_tenant_label_writes_annotation() {
    let mut e = ev("acme", "pods", AuditStage::ResponseComplete);
    inject_tenant_label(&mut e);
    assert_eq!(e.annotations.get("audit.k8s.io/tenant-id"),
               Some(&"acme".to_string()));
}

#[test]
fn scrub_secret_payloads_strips_bodies() {
    let mut e = ev("acme", "secrets", AuditStage::ResponseComplete);
    e.request_object = Some(serde_json::json!({"data":{"k":"v"}}));
    e.response_object = Some(serde_json::json!({"data":{"k":"v"}}));
    scrub_secret_payloads(&mut e);
    assert!(e.request_object.is_none());
    assert!(e.response_object.is_none());
}

#[test]
fn scrub_secret_payloads_preserves_pod_bodies() {
    let mut e = ev("acme", "pods", AuditStage::ResponseComplete);
    e.request_object = Some(serde_json::json!({"x":1}));
    scrub_secret_payloads(&mut e);
    assert!(e.request_object.is_some());
}

#[test]
fn redact_for_stage_drops_response_on_received() {
    let mut e = ev("acme", "pods", AuditStage::RequestReceived);
    e.response_object = Some(serde_json::json!({"x":1}));
    redact_for_stage(&mut e);
    assert!(e.response_object.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// CountingBackend itself
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn counting_records_per_call_tenant() {
    let c = CountingBackend::new();
    c.process_events(&[ev("acme", "pods", AuditStage::ResponseComplete)]);
    c.process_events(&[ev("globex", "secrets", AuditStage::ResponseComplete)]);
    let calls = c.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "acme");
    assert_eq!(calls[1].0, "globex");
}

#[test]
fn counting_total_sums_calls() {
    let c = CountingBackend::new();
    c.process_events(&[
        ev("acme", "pods", AuditStage::ResponseComplete),
        ev("acme", "pods", AuditStage::ResponseComplete),
    ]);
    assert_eq!(c.total(), 2);
}

#[test]
fn counting_empty_call_records_nothing() {
    let c = CountingBackend::new();
    c.process_events(&[]);
    assert!(c.calls().is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real disk / HTTPS
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn log_writes_real_file_with_atomic_rotate() {
    todo!("requires std::fs + atomic rename for real file rotation");
}

#[test] #[ignore]
fn webhook_real_https_backend() {
    todo!("requires HTTPS POST to external sink");
}

#[test] #[ignore]
fn batch_periodic_flush_via_timer() {
    todo!("requires real-time scheduler — flush on max_wait elapsed");
}

#[test] #[ignore]
fn audit_event_redaction_for_managed_fields() {
    todo!("integrates with M4 omitManagedFields");
}
