//! In-memory log store with optional PostgreSQL persistence.
//!
//! Primary index: `DashMap<fingerprint, StreamData>` keyed by
//! SHA-fingerprint(labels + tenant).  A tokio broadcast channel
//! powers live WebSocket tail.

use crate::models::{FiredAlert, LabelMatcher, Labels, LogEntry};
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::debug;

// ─── Internal stream storage ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StreamData {
    pub labels: Labels,
    pub tenant: Option<String>,
    /// Entries kept sorted ascending by timestamp.
    pub entries: Vec<LogEntry>,
}

// ─── Tail event ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TailEvent {
    pub labels: Labels,
    pub entries: Vec<LogEntry>,
    pub tenant: Option<String>,
}

// ─── LogStore ─────────────────────────────────────────────────────────────────

const TAIL_CAP: usize = 4096;

pub struct LogStore {
    streams: Arc<DashMap<u64, StreamData>>,
    tail_tx: broadcast::Sender<TailEvent>,
    pub retention: Duration,
}

impl LogStore {
    pub fn new(retention: Duration) -> Self {
        let (tail_tx, _) = broadcast::channel(TAIL_CAP);
        Self {
            streams: Arc::new(DashMap::new()),
            tail_tx,
            retention,
        }
    }

    // ── Push ─────────────────────────────────────────────────────────────────

    /// Ingest entries for a stream. Entries are merged and kept sorted.
    pub fn push(&self, labels: Labels, entries: Vec<LogEntry>, tenant: Option<String>) {
        if entries.is_empty() {
            return;
        }
        let fp = labels.fingerprint_with_tenant(tenant.as_deref());

        {
            let mut stream = self.streams.entry(fp).or_insert_with(|| StreamData {
                labels: labels.clone(),
                tenant: tenant.clone(),
                entries: Vec::new(),
            });
            stream.entries.extend(entries.iter().cloned());
            stream.entries.sort_by_key(|e| e.timestamp);
        }

        let _ = self.tail_tx.send(TailEvent {
            labels,
            entries,
            tenant,
        });

        debug!(streams = self.streams.len(), "push complete");
    }

    // ── Query ─────────────────────────────────────────────────────────────────

    /// Return all (labels, entries) pairs whose label set matches `matchers`
    /// and which have at least one entry in [start, end].
    pub fn query_streams(
        &self,
        matchers: &[LabelMatcher],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: usize,
        direction_forward: bool,
        tenant: Option<&str>,
    ) -> Vec<(Labels, Vec<LogEntry>)> {
        let mut results: Vec<(Labels, Vec<LogEntry>)> = self
            .streams
            .iter()
            .filter_map(|r| {
                let s = r.value();
                // Tenant isolation
                if let Some(t) = tenant {
                    if s.tenant.as_deref() != Some(t) {
                        return None;
                    }
                }
                if !s.labels.matches(matchers) {
                    return None;
                }
                let entries: Vec<LogEntry> = s
                    .entries
                    .iter()
                    .filter(|e| e.timestamp >= start && e.timestamp <= end)
                    .cloned()
                    .collect();
                if entries.is_empty() {
                    None
                } else {
                    Some((s.labels.clone(), entries))
                }
            })
            .collect();

        // Sort streams deterministically
        results.sort_by(|(a, _), (b, _)| a.to_selector().cmp(&b.to_selector()));

        // Apply per-stream direction + limit
        let mut total = 0usize;
        results.retain_mut(|(_, entries)| {
            if total >= limit {
                return false;
            }
            if !direction_forward {
                entries.reverse();
            }
            let take = (limit - total).min(entries.len());
            entries.truncate(take);
            total += entries.len();
            true
        });

        results
    }

    // ── Label metadata ────────────────────────────────────────────────────────

    pub fn label_names(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        tenant: Option<&str>,
    ) -> Vec<String> {
        let mut names = std::collections::HashSet::new();
        for r in self.streams.iter() {
            let s = r.value();
            if self.tenant_mismatch(s, tenant) {
                continue;
            }
            if s.entries.iter().any(|e| e.timestamp >= start && e.timestamp <= end) {
                names.extend(s.labels.0.keys().cloned());
            }
        }
        let mut v: Vec<_> = names.into_iter().collect();
        v.sort();
        v
    }

    pub fn label_values(
        &self,
        label: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        tenant: Option<&str>,
    ) -> Vec<String> {
        let mut values = std::collections::HashSet::new();
        for r in self.streams.iter() {
            let s = r.value();
            if self.tenant_mismatch(s, tenant) {
                continue;
            }
            if s.entries.iter().any(|e| e.timestamp >= start && e.timestamp <= end) {
                if let Some(v) = s.labels.0.get(label) {
                    values.insert(v.clone());
                }
            }
        }
        let mut v: Vec<_> = values.into_iter().collect();
        v.sort();
        v
    }

    /// Return all label sets that match `matchers` and have data in [start, end].
    pub fn series(
        &self,
        matchers: &[LabelMatcher],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        tenant: Option<&str>,
    ) -> Vec<Labels> {
        self.streams
            .iter()
            .filter_map(|r| {
                let s = r.value();
                if self.tenant_mismatch(s, tenant) {
                    return None;
                }
                if !s.labels.matches(matchers) {
                    return None;
                }
                if s.entries.iter().any(|e| e.timestamp >= start && e.timestamp <= end) {
                    Some(s.labels.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    // ── Aggregation helpers for metric queries ────────────────────────────────

    /// Count entries per time bucket of `step_secs` seconds.
    pub fn count_over_buckets(
        &self,
        matchers: &[LabelMatcher],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        range_secs: i64,
        step_secs: i64,
        tenant: Option<&str>,
    ) -> Vec<(DateTime<Utc>, f64)> {
        // Collect all matching timestamps (must be eager to avoid holding DashMap Ref)
        let mut all: Vec<DateTime<Utc>> = self
            .streams
            .iter()
            .flat_map(|r| {
                let s = r.value();
                if self.tenant_mismatch(s, tenant) || !s.labels.matches(matchers) {
                    return vec![];
                }
                s.entries
                    .iter()
                    .filter(|e| e.timestamp >= start && e.timestamp <= end)
                    .map(|e| e.timestamp)
                    .collect::<Vec<_>>()
            })
            .collect();
        all.sort();

        bucket_aggregate(all, start, end, range_secs, step_secs, |ts, window_start| {
            ts >= window_start
        })
    }

    /// Rate (count / range_secs) per time bucket.
    pub fn rate_over_buckets(
        &self,
        matchers: &[LabelMatcher],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        range_secs: i64,
        step_secs: i64,
        tenant: Option<&str>,
    ) -> Vec<(DateTime<Utc>, f64)> {
        self.count_over_buckets(matchers, start, end, range_secs, step_secs, tenant)
            .into_iter()
            .map(|(t, c)| (t, c / range_secs as f64))
            .collect()
    }

    /// Bytes (sum of line lengths) per time bucket.
    pub fn bytes_over_buckets(
        &self,
        matchers: &[LabelMatcher],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        range_secs: i64,
        step_secs: i64,
        tenant: Option<&str>,
    ) -> Vec<(DateTime<Utc>, f64)> {
        let mut all: Vec<(DateTime<Utc>, usize)> = self
            .streams
            .iter()
            .flat_map(|r| {
                let s = r.value();
                if self.tenant_mismatch(s, tenant) || !s.labels.matches(matchers) {
                    return vec![];
                }
                s.entries
                    .iter()
                    .filter(|e| e.timestamp >= start && e.timestamp <= end)
                    .map(|e| (e.timestamp, e.line.len()))
                    .collect::<Vec<_>>()
            })
            .collect();
        all.sort_by_key(|(t, _)| *t);

        let step = Duration::seconds(step_secs);
        let range = Duration::seconds(range_secs);
        let mut result = vec![];
        let mut t = start;
        while t <= end {
            let ws = t - range;
            let bytes: f64 = all
                .iter()
                .filter(|(ts, _)| *ts >= ws && *ts <= t)
                .map(|(_, b)| *b as f64)
                .sum();
            result.push((t, bytes));
            t = t + step;
        }
        result
    }

    // ── Tail ─────────────────────────────────────────────────────────────────

    pub fn subscribe(&self) -> broadcast::Receiver<TailEvent> {
        self.tail_tx.subscribe()
    }

    // ── Retention ─────────────────────────────────────────────────────────────

    /// Remove entries older than the configured retention window, then drop
    /// empty streams.
    pub fn prune(&self) {
        let cutoff = Utc::now() - self.retention;
        for mut r in self.streams.iter_mut() {
            r.entries.retain(|e| e.timestamp >= cutoff);
        }
        self.streams.retain(|_, s| !s.entries.is_empty());
    }

    // ── Alert evaluation ──────────────────────────────────────────────────────

    pub fn eval_alert(
        &self,
        rule: &crate::models::AlertRule,
        now: DateTime<Utc>,
    ) -> Option<FiredAlert> {
        use crate::logql::{eval::Evaluator, parser::parse};
        let expr = parse(&rule.expr).ok()?;
        let duration = Duration::seconds(rule.duration_secs as i64);
        let start = now - duration;
        let eval = Evaluator::new(self);
        let value = eval.eval_scalar(&expr, start, now, rule.tenant.as_deref())?;
        if rule.condition.eval(value) {
            Some(FiredAlert {
                rule_id: rule.id,
                rule_name: rule.name.clone(),
                value,
                fired_at: now,
                severity: rule.severity.clone(),
                annotations: rule.annotations.clone(),
            })
        } else {
            None
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn tenant_mismatch(&self, s: &StreamData, tenant: Option<&str>) -> bool {
        match tenant {
            Some(t) => s.tenant.as_deref() != Some(t),
            None => false,
        }
    }

    /// Total number of streams.
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    /// Total number of entries across all streams.
    pub fn entry_count(&self) -> usize {
        self.streams.iter().map(|r| r.entries.len()).sum()
    }
}

// ─── Bucket aggregation helper ────────────────────────────────────────────────

fn bucket_aggregate(
    mut timestamps: Vec<DateTime<Utc>>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    range_secs: i64,
    step_secs: i64,
    filter: impl Fn(DateTime<Utc>, DateTime<Utc>) -> bool,
) -> Vec<(DateTime<Utc>, f64)> {
    timestamps.sort();
    let step = Duration::seconds(step_secs);
    let range = Duration::seconds(range_secs);
    let mut result = vec![];
    let mut t = start;
    while t <= end {
        let window_start = t - range;
        let count = timestamps
            .iter()
            .filter(|&&ts| filter(ts, window_start) && ts <= t)
            .count() as f64;
        result.push((t, count));
        t = t + step;
    }
    result
}
