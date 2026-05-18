// SPDX-License-Identifier: AGPL-3.0-or-later
//! BatchSpanProcessor — bounded queue + ticker that flushes spans to the
//! configured `SpanExporter`. Drop-on-overflow is the default backpressure
//! strategy (matching OTel SDK behaviour). Force-flush + shutdown are
//! both supported and idempotent.

use crate::exporter::{ExportError, SpanExporter};
use crate::types::SpanData;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub max_queue_size: usize,
    pub max_export_batch_size: usize,
    pub schedule_delay: Duration,
    pub export_timeout: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self {
        BatchConfig {
            max_queue_size: 2048,
            max_export_batch_size: 512,
            schedule_delay: Duration::from_secs(5),
            export_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct BatchStats {
    pub queued: u64,
    pub dropped: u64,
    pub exported: u64,
    pub export_errors: u64,
}

struct Inner {
    queue: Mutex<Vec<SpanData>>,
    stats: Mutex<BatchStats>,
    notify: Notify,
    closed: parking_lot::RwLock<bool>,
    config: BatchConfig,
}

#[derive(Clone)]
pub struct BatchSpanProcessor {
    inner: Arc<Inner>,
    exporter: Arc<dyn SpanExporter>,
}

impl BatchSpanProcessor {
    pub fn new(exporter: Arc<dyn SpanExporter>, config: BatchConfig) -> Self {
        BatchSpanProcessor {
            inner: Arc::new(Inner {
                queue: Mutex::new(Vec::with_capacity(config.max_queue_size)),
                stats: Mutex::new(BatchStats::default()),
                notify: Notify::new(),
                closed: parking_lot::RwLock::new(false),
                config,
            }),
            exporter,
        }
    }

    /// Try to enqueue a span. If the queue is full, the span is dropped
    /// and the drop counter is incremented.
    pub fn on_end(&self, span: SpanData) {
        if *self.inner.closed.read() {
            return;
        }
        let mut q = self.inner.queue.lock();
        if q.len() >= self.inner.config.max_queue_size {
            self.inner.stats.lock().dropped += 1;
            return;
        }
        q.push(span);
        self.inner.stats.lock().queued += 1;
        if q.len() >= self.inner.config.max_export_batch_size {
            self.inner.notify.notify_one();
        }
    }

    pub fn stats(&self) -> BatchStats {
        self.inner.stats.lock().clone()
    }

    pub fn queue_len(&self) -> usize {
        self.inner.queue.lock().len()
    }

    pub fn config(&self) -> &BatchConfig { &self.inner.config }

    /// Synchronously drain the queue and export everything currently
    /// queued. Used by tests and `shutdown`.
    pub async fn force_flush(&self) -> Result<usize, ExportError> {
        let mut total = 0;
        loop {
            let drained = {
                let mut q = self.inner.queue.lock();
                if q.is_empty() { break; }
                let take = q.len().min(self.inner.config.max_export_batch_size);
                q.drain(..take).collect::<Vec<_>>()
            };
            let n = drained.len();
            match self.exporter.export(drained).await {
                Ok(()) => {
                    self.inner.stats.lock().exported += n as u64;
                    total += n;
                }
                Err(e) => {
                    self.inner.stats.lock().export_errors += 1;
                    return Err(e);
                }
            }
        }
        Ok(total)
    }

    /// Drain and shutdown.
    pub async fn shutdown(&self) -> Result<usize, ExportError> {
        let n = self.force_flush().await?;
        *self.inner.closed.write() = true;
        let _ = self.exporter.shutdown().await;
        Ok(n)
    }

    /// Spawn the background ticker. Returns a handle the caller can `abort()`
    /// to stop the loop early. The ticker periodically flushes the queue
    /// regardless of size, so that low-traffic services still emit.
    pub fn spawn_ticker(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.inner.config.schedule_delay);
            interval.tick().await; // immediate first tick
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let _ = self.force_flush().await;
                    }
                    _ = self.inner.notify.notified() => {
                        let _ = self.force_flush().await;
                    }
                }
                if *self.inner.closed.read() { break; }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exporter::InMemoryExporter;
    use crate::types::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn span(i: u64) -> SpanData {
        let now = Utc::now();
        SpanData {
            name: format!("s{}", i),
            context: SpanContext::new(0xdead, i, true),
            parent_span_id: None,
            kind: SpanKind::Internal,
            start_time: now,
            end_time: now + chrono::Duration::milliseconds(1),
            attributes: HashMap::new(),
            events: vec![],
            links: vec![],
            status: Status::Unset,
            instrumentation_scope: "t".into(),
            tenant_id: "anonymous".into(),
            resource: HashMap::new(),
        }
    }

    fn proc_with(cap: usize, batch: usize) -> (BatchSpanProcessor, InMemoryExporter) {
        let e = InMemoryExporter::new();
        let p = BatchSpanProcessor::new(
            Arc::new(e.clone()),
            BatchConfig {
                max_queue_size: cap,
                max_export_batch_size: batch,
                schedule_delay: Duration::from_millis(50),
                export_timeout: Duration::from_secs(1),
            },
        );
        (p, e)
    }

    #[tokio::test]
    async fn test_force_flush_drains_queue() {
        let (p, e) = proc_with(100, 10);
        for i in 0..5 { p.on_end(span(i)); }
        assert_eq!(p.queue_len(), 5);
        let n = p.force_flush().await.unwrap();
        assert_eq!(n, 5);
        assert_eq!(e.count(), 5);
        assert_eq!(p.queue_len(), 0);
    }

    #[tokio::test]
    async fn test_drop_on_overflow_increments_counter() {
        let (p, _e) = proc_with(3, 10);
        for i in 0..10 { p.on_end(span(i)); }
        let s = p.stats();
        assert_eq!(s.queued, 3);
        assert_eq!(s.dropped, 7);
    }

    #[tokio::test]
    async fn test_force_flush_chunks_by_max_export_batch_size() {
        let (p, e) = proc_with(100, 4);
        for i in 0..10 { p.on_end(span(i)); }
        // 10 spans / batch 4 → three exports of 4, 4, 2 — all collected
        let n = p.force_flush().await.unwrap();
        assert_eq!(n, 10);
        assert_eq!(e.count(), 10);
    }

    #[tokio::test]
    async fn test_force_flush_returns_export_error() {
        let (p, e) = proc_with(100, 10);
        e.fail_next(1);
        for i in 0..5 { p.on_end(span(i)); }
        let res = p.force_flush().await;
        assert!(res.is_err());
        assert_eq!(p.stats().export_errors, 1);
        // Queue is partially drained: the failing batch was already removed.
        assert_eq!(p.queue_len(), 0);
    }

    #[tokio::test]
    async fn test_shutdown_drains_and_blocks_further_writes() {
        let (p, e) = proc_with(100, 10);
        for i in 0..3 { p.on_end(span(i)); }
        p.shutdown().await.unwrap();
        assert_eq!(e.count(), 3);
        // After shutdown, on_end drops silently
        for i in 100..105 { p.on_end(span(i)); }
        assert_eq!(p.queue_len(), 0);
    }

    #[tokio::test]
    async fn test_stats_track_exports_and_dropped() {
        let (p, e) = proc_with(2, 10);
        for i in 0..5 { p.on_end(span(i)); } // 2 queued, 3 dropped
        p.force_flush().await.unwrap();
        let s = p.stats();
        assert_eq!(s.queued, 2);
        assert_eq!(s.dropped, 3);
        assert_eq!(s.exported, 2);
        assert_eq!(e.count(), 2);
    }

    #[tokio::test]
    async fn test_ticker_flushes_periodically() {
        let (p, e) = proc_with(100, 100);
        let handle = p.clone().spawn_ticker();
        for i in 0..3 { p.on_end(span(i)); }
        // Wait a couple of ticker cycles (50ms each)
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();
        let _ = p.shutdown().await;
        assert_eq!(e.count(), 3);
    }

    #[tokio::test]
    async fn test_force_flush_when_empty_is_noop() {
        let (p, e) = proc_with(10, 10);
        let n = p.force_flush().await.unwrap();
        assert_eq!(n, 0);
        assert_eq!(e.count(), 0);
    }

    #[tokio::test]
    async fn test_default_config_values() {
        let c = BatchConfig::default();
        assert_eq!(c.max_queue_size, 2048);
        assert_eq!(c.max_export_batch_size, 512);
        assert_eq!(c.schedule_delay, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_config_accessible_via_processor() {
        let (p, _) = proc_with(50, 10);
        assert_eq!(p.config().max_queue_size, 50);
    }

    #[tokio::test]
    async fn test_repeated_shutdown_does_not_panic() {
        let (p, _) = proc_with(10, 10);
        p.on_end(span(1));
        p.shutdown().await.unwrap();
        // Second shutdown should still succeed (idempotent)
        let res = p.shutdown().await;
        assert!(res.is_ok());
    }
}
