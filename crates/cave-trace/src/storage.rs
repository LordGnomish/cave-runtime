//! Columnar trace storage with Bloom filter, tag index, and retention policies.
//!
//! Architecture
//! ────────────
//! • `TraceStore` holds all state under a single `RwLock`.
//! • Primary store: `HashMap<TraceId, TraceRecord>` — full spans + columnar projection.
//! • Secondary indexes:
//!     service_index   : service_name → BTreeSet<TraceId>
//!     operation_index : (service, operation) → BTreeSet<TraceId>
//!     tag_index       : (key, value) → BTreeSet<TraceId>
//!     time_index      : BTreeMap<start_ns, TraceId>  — ordered, supports range scans
//! • Bloom filter on TraceId for fast "not found" rejection.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{
    build_histogram, LatencyHistogram, Span, SpanId, SpanStatus, TagValue, Trace, TraceId,
    TraceSearchQuery,
};
use crate::{Result, TraceError};

// ─── Bloom filter ──────────────────────────────────────────────────────────

const BLOOM_BITS: usize = 1 << 22; // 4 M bits ≈ 512 KB, ~1 M items @ 0.1% FPR
const BLOOM_K: usize = 7; // number of hash functions

pub struct BloomFilter {
    bits: Vec<u64>,
    item_count: usize,
}

impl BloomFilter {
    pub fn new() -> Self {
        BloomFilter {
            bits: vec![0u64; BLOOM_BITS / 64],
            item_count: 0,
        }
    }

    fn hashes(trace_id: TraceId) -> [usize; BLOOM_K] {
        // Double-hashing: h_i(x) = (h1 + i * h2) mod m
        let bytes = trace_id.to_le_bytes();
        let h1 = fnv1a_64(&bytes);
        let h2 = fnv1a_64(&trace_id.to_be_bytes());
        let m = BLOOM_BITS as u64;
        let mut out = [0usize; BLOOM_K];
        for i in 0..BLOOM_K {
            out[i] = (h1.wrapping_add((i as u64).wrapping_mul(h2)) % m) as usize;
        }
        out
    }

    pub fn insert(&mut self, trace_id: TraceId) {
        for bit in Self::hashes(trace_id) {
            self.bits[bit / 64] |= 1u64 << (bit % 64);
        }
        self.item_count += 1;
    }

    pub fn may_contain(&self, trace_id: TraceId) -> bool {
        Self::hashes(trace_id)
            .iter()
            .all(|&bit| self.bits[bit / 64] & (1u64 << (bit % 64)) != 0)
    }

    pub fn clear(&mut self) {
        self.bits.iter_mut().for_each(|w| *w = 0);
        self.item_count = 0;
    }

    pub fn estimated_items(&self) -> usize {
        self.item_count
    }
}

fn fnv1a_64(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x00000100000001b3);
    }
    h
}

// ─── Columnar span projection (fast analytics) ────────────────────────────

/// Per-trace columnar arrays — kept in sync with `spans` for O(1) analytics.
#[derive(Debug, Default)]
pub struct ColumnarTrace {
    pub span_ids: Vec<SpanId>,
    pub start_times_ns: Vec<u64>,
    pub durations_ns: Vec<u64>,
    pub service_names: Vec<String>,
    pub operation_names: Vec<String>,
    pub statuses: Vec<SpanStatus>,
    pub kinds: Vec<crate::types::SpanKind>,
    pub is_root: Vec<bool>,
}

impl ColumnarTrace {
    pub fn push_span(&mut self, span: &Span) {
        self.span_ids.push(span.span_id);
        self.start_times_ns.push(span.start_time_unix_nano);
        self.durations_ns.push(span.duration_ns);
        self.service_names.push(span.service_name.clone());
        self.operation_names.push(span.operation_name.clone());
        self.statuses.push(span.status);
        self.kinds.push(span.kind);
        self.is_root.push(span.is_root());
    }

    pub fn error_count(&self) -> usize {
        self.statuses.iter().filter(|s| s.is_error()).count()
    }

    pub fn span_count(&self) -> usize {
        self.span_ids.len()
    }
}

// ─── Trace record ──────────────────────────────────────────────────────────

pub struct TraceRecord {
    pub trace_id: TraceId,
    pub spans: Vec<Span>,
    pub columnar: ColumnarTrace,
    pub ingested_at_ns: u64,
    pub tenant_id: String,
}

impl TraceRecord {
    pub fn new(trace_id: TraceId, tenant_id: String) -> Self {
        TraceRecord {
            trace_id,
            spans: Vec::new(),
            columnar: ColumnarTrace::default(),
            ingested_at_ns: now_ns(),
            tenant_id,
        }
    }

    pub fn add_span(&mut self, span: Span) {
        self.columnar.push_span(&span);
        self.spans.push(span);
    }

    pub fn to_trace(&self) -> Option<Trace> {
        Trace::from_spans(self.spans.clone())
    }

    pub fn trace_start_ns(&self) -> u64 {
        self.columnar.start_times_ns.iter().copied().min().unwrap_or(0)
    }

    pub fn trace_end_ns(&self) -> u64 {
        self.columnar
            .start_times_ns
            .iter()
            .zip(self.columnar.durations_ns.iter())
            .map(|(&s, &d)| s + d)
            .max()
            .unwrap_or(0)
    }
}

// ─── Retention policy ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    /// Maximum age of traces in nanoseconds.
    pub max_age_ns: u64,
    /// Maximum number of traces per tenant.
    pub max_traces_per_tenant: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy {
            max_age_ns: 72 * 3_600 * 1_000_000_000, // 72 h
            max_traces_per_tenant: 100_000,
        }
    }
}

impl RetentionPolicy {
    pub fn from_hours(hours: u64, max_per_tenant: usize) -> Self {
        RetentionPolicy {
            max_age_ns: hours * 3_600 * 1_000_000_000,
            max_traces_per_tenant: max_per_tenant,
        }
    }
}

// ─── Tag index entry ───────────────────────────────────────────────────────

type ServiceKey = String;
type OpKey = (String, String); // (service, operation)
type TagKey = (String, String); // (key, display_value)

// ─── Main store ────────────────────────────────────────────────────────────

pub struct TraceStore {
    traces: HashMap<TraceId, TraceRecord>,
    span_index: HashMap<SpanId, TraceId>,

    // Secondary indexes (tenant-scoped values use "tenant/key" prefix)
    service_index: HashMap<String, BTreeSet<TraceId>>,
    operation_index: HashMap<OpKey, BTreeSet<TraceId>>,
    tag_index: HashMap<TagKey, BTreeSet<TraceId>>,
    time_index: BTreeMap<u64, Vec<TraceId>>, // start_ns → trace IDs

    bloom: BloomFilter,
    pub retention: RetentionPolicy,
}

impl TraceStore {
    pub fn new(retention: RetentionPolicy) -> Self {
        TraceStore {
            traces: HashMap::new(),
            span_index: HashMap::new(),
            service_index: HashMap::new(),
            operation_index: HashMap::new(),
            tag_index: HashMap::new(),
            time_index: BTreeMap::new(),
            bloom: BloomFilter::new(),
            retention,
        }
    }

    // ── Ingestion ──────────────────────────────────────────────────────────

    /// Ingest a batch of spans. Spans sharing a trace_id are merged into one record.
    pub fn ingest_spans(&mut self, spans: Vec<Span>) {
        for span in spans {
            let trace_id = span.trace_id;
            let tenant_id = span.tenant_id.clone();

            let record = self
                .traces
                .entry(trace_id)
                .or_insert_with(|| TraceRecord::new(trace_id, tenant_id));

            // Update indexes
            let svc = span.service_name.clone();
            let op = span.operation_name.clone();
            let start_ns = span.start_time_unix_nano;

            // Index tags
            for (k, v) in &span.tags {
                self.tag_index
                    .entry((k.clone(), v.display()))
                    .or_default()
                    .insert(trace_id);
            }

            self.span_index.insert(span.span_id, trace_id);
            record.add_span(span);

            // Service / operation indexes
            self.service_index
                .entry(svc.clone())
                .or_default()
                .insert(trace_id);
            self.operation_index
                .entry((svc, op))
                .or_default()
                .insert(trace_id);

            // Time index
            self.time_index.entry(start_ns).or_default().push(trace_id);

            // Bloom
            self.bloom.insert(trace_id);
        }
    }

    // ── Lookups ────────────────────────────────────────────────────────────

    pub fn get_trace(&self, trace_id: TraceId) -> Option<&TraceRecord> {
        if !self.bloom.may_contain(trace_id) {
            return None;
        }
        self.traces.get(&trace_id)
    }

    pub fn get_span(&self, span_id: SpanId) -> Option<&Span> {
        let trace_id = *self.span_index.get(&span_id)?;
        let record = self.traces.get(&trace_id)?;
        record.spans.iter().find(|s| s.span_id == span_id)
    }

    pub fn trace_exists(&self, trace_id: TraceId) -> bool {
        self.bloom.may_contain(trace_id) && self.traces.contains_key(&trace_id)
    }

    // ── Services / operations ──────────────────────────────────────────────

    pub fn list_services(&self, tenant_id: Option<&str>) -> Vec<String> {
        let mut services: Vec<String> = if let Some(tenant) = tenant_id {
            // Filter by traces belonging to this tenant
            self.service_index
                .iter()
                .filter(|(_, ids)| {
                    ids.iter().any(|id| {
                        self.traces
                            .get(id)
                            .map(|r| r.tenant_id == tenant)
                            .unwrap_or(false)
                    })
                })
                .map(|(k, _)| k.clone())
                .collect()
        } else {
            self.service_index.keys().cloned().collect()
        };
        services.sort();
        services.dedup();
        services
    }

    pub fn list_operations(&self, service: &str, tenant_id: Option<&str>) -> Vec<String> {
        let mut ops: Vec<String> = self
            .operation_index
            .iter()
            .filter(|((svc, _), ids)| {
                svc == service
                    && tenant_id.map_or(true, |t| {
                        ids.iter()
                            .any(|id| self.traces.get(id).map(|r| r.tenant_id == t).unwrap_or(false))
                    })
            })
            .map(|((_, op), _)| op.clone())
            .collect();
        ops.sort();
        ops.dedup();
        ops
    }

    pub fn list_tag_names(&self, tenant_id: Option<&str>) -> Vec<String> {
        let mut names: Vec<String> = self
            .tag_index
            .keys()
            .filter(|(key, _)| {
                tenant_id.map_or(true, |t| {
                    self.tag_index
                        .get(&(key.clone(), "".to_string()))
                        .map_or(true, |ids| {
                            ids.iter()
                                .any(|id| self.traces.get(id).map(|r| r.tenant_id == t).unwrap_or(false))
                        })
                })
            })
            .map(|(k, _)| k.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn list_tag_values(&self, tag_name: &str, tenant_id: Option<&str>) -> Vec<String> {
        let _ = tenant_id; // filter by tenant if needed
        let mut vals: Vec<String> = self
            .tag_index
            .keys()
            .filter(|(k, _)| k == tag_name)
            .map(|(_, v)| v.clone())
            .collect();
        vals.sort();
        vals.dedup();
        vals
    }

    // ── Search ─────────────────────────────────────────────────────────────

    pub fn search(&self, query: &TraceSearchQuery) -> Vec<&TraceRecord> {
        // Start with a candidate set
        let candidates: Box<dyn Iterator<Item = TraceId>> = match (&query.service, &query.operation) {
            (Some(svc), Some(op)) => {
                let key = (svc.clone(), op.clone());
                if let Some(ids) = self.operation_index.get(&key) {
                    Box::new(ids.iter().copied().collect::<Vec<_>>().into_iter())
                } else {
                    return vec![];
                }
            }
            (Some(svc), None) => {
                if let Some(ids) = self.service_index.get(svc.as_str()) {
                    Box::new(ids.iter().copied().collect::<Vec<_>>().into_iter())
                } else {
                    return vec![];
                }
            }
            _ => {
                // Time range scan
                let start = query.start_time_ns.unwrap_or(0);
                let end = query.end_time_ns.unwrap_or(u64::MAX);
                let ids: Vec<TraceId> = self
                    .time_index
                    .range(start..=end)
                    .flat_map(|(_, ids)| ids.iter().copied())
                    .collect();
                Box::new(ids.into_iter())
            }
        };

        // Apply tag filters
        let tag_required: Option<Vec<(String, String)>> = query.tags.as_ref().map(|t| {
            t.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        });

        let mut seen = std::collections::HashSet::new();
        let mut results: Vec<&TraceRecord> = candidates
            .filter_map(|id| {
                if !seen.insert(id) {
                    return None;
                }
                let record = self.traces.get(&id)?;

                // Tenant filter
                if let Some(t) = &query.tenant_id {
                    if &record.tenant_id != t {
                        return None;
                    }
                }

                // Time range
                let trace_start = record.trace_start_ns();
                if let Some(start) = query.start_time_ns {
                    if trace_start < start {
                        return None;
                    }
                }
                if let Some(end) = query.end_time_ns {
                    if trace_start > end {
                        return None;
                    }
                }

                // Duration range
                let dur = record.trace_end_ns().saturating_sub(record.trace_start_ns());
                if let Some(min_dur) = query.min_duration_ns {
                    if dur < min_dur {
                        return None;
                    }
                }
                if let Some(max_dur) = query.max_duration_ns {
                    if dur > max_dur {
                        return None;
                    }
                }

                // Tag filter
                if let Some(ref required) = tag_required {
                    for (k, v) in required {
                        let found = self
                            .tag_index
                            .get(&(k.clone(), v.clone()))
                            .map(|ids| ids.contains(&id))
                            .unwrap_or(false);
                        if !found {
                            return None;
                        }
                    }
                }

                Some(record)
            })
            .collect();

        // Sort by trace start time descending (most recent first)
        results.sort_by(|a, b| b.trace_start_ns().cmp(&a.trace_start_ns()));

        let offset = query.offset.unwrap_or(0);
        let limit = query.limit_or_default();
        results.into_iter().skip(offset).take(limit).collect()
    }

    // ── Analytics helpers ──────────────────────────────────────────────────

    /// Collect all spans across all traces matching a service filter.
    pub fn spans_for_service<'a>(
        &'a self,
        service: &str,
        tenant_id: Option<&str>,
    ) -> Vec<&'a Span> {
        self.service_index
            .get(service)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.traces.get(id))
            .filter(|r| tenant_id.map_or(true, |t| r.tenant_id == t))
            .flat_map(|r| r.spans.iter())
            .filter(|s| s.service_name == service)
            .collect()
    }

    /// Compute latency histogram for (service, operation) over a time window.
    pub fn latency_histogram(
        &self,
        service: &str,
        operation: &str,
        start_ns: Option<u64>,
        end_ns: Option<u64>,
        tenant_id: Option<&str>,
    ) -> LatencyHistogram {
        let durations: Vec<u64> = self
            .operation_index
            .get(&(service.to_owned(), operation.to_owned()))
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.traces.get(id))
            .filter(|r| tenant_id.map_or(true, |t| r.tenant_id == t))
            .flat_map(|r| r.spans.iter())
            .filter(|s| s.service_name == service && s.operation_name == operation)
            .filter(|s| {
                start_ns.map_or(true, |t| s.start_time_unix_nano >= t)
                    && end_ns.map_or(true, |t| s.start_time_unix_nano <= t)
            })
            .map(|s| s.duration_ns)
            .collect();

        build_histogram(service.into(), operation.into(), durations)
    }

    /// Collect all trace records for a given tenant.
    pub fn all_records_for_tenant<'a>(&'a self, tenant_id: &str) -> Vec<&'a TraceRecord> {
        self.traces
            .values()
            .filter(|r| r.tenant_id == tenant_id)
            .collect()
    }

    pub fn total_trace_count(&self) -> usize {
        self.traces.len()
    }

    pub fn bloom_item_count(&self) -> usize {
        self.bloom.estimated_items()
    }

    // ── Retention / eviction ───────────────────────────────────────────────

    /// Remove expired traces (age-based) and enforce per-tenant trace limits.
    /// Returns the number of traces removed.
    pub fn apply_retention(&mut self) -> usize {
        let now = now_ns();
        let cutoff = now.saturating_sub(self.retention.max_age_ns);
        let max_per_tenant = self.retention.max_traces_per_tenant;

        let expired: Vec<TraceId> = self
            .traces
            .iter()
            .filter(|(_, r)| r.ingested_at_ns < cutoff)
            .map(|(id, _)| *id)
            .collect();

        let mut removed = expired.len();
        for id in &expired {
            self.remove_trace(*id);
        }

        // Per-tenant overflow: evict oldest when over limit
        let mut tenant_counts: HashMap<String, Vec<(u64, TraceId)>> = HashMap::new();
        for (id, rec) in &self.traces {
            tenant_counts
                .entry(rec.tenant_id.clone())
                .or_default()
                .push((rec.ingested_at_ns, *id));
        }
        for (_tenant, mut entries) in tenant_counts {
            if entries.len() > max_per_tenant {
                entries.sort_unstable(); // oldest first
                let evict = entries.len() - max_per_tenant;
                for (_, id) in entries.into_iter().take(evict) {
                    self.remove_trace(id);
                    removed += 1;
                }
            }
        }

        // Rebuild bloom if significant eviction occurred
        if removed > 0 {
            self.rebuild_bloom();
        }

        removed
    }

    fn remove_trace(&mut self, trace_id: TraceId) {
        if let Some(rec) = self.traces.remove(&trace_id) {
            // Clean span_index
            for span in &rec.spans {
                self.span_index.remove(&span.span_id);
                // Clean secondary indexes
                let svc = &span.service_name;
                let op = &span.operation_name;
                if let Some(s) = self.service_index.get_mut(svc) {
                    s.remove(&trace_id);
                }
                if let Some(s) = self.operation_index.get_mut(&(svc.clone(), op.clone())) {
                    s.remove(&trace_id);
                }
                for (k, v) in &span.tags {
                    if let Some(s) = self.tag_index.get_mut(&(k.clone(), v.display())) {
                        s.remove(&trace_id);
                    }
                }
            }
            // Time index cleanup
            let start_ns = rec.trace_start_ns();
            if let Some(ids) = self.time_index.get_mut(&start_ns) {
                ids.retain(|&id| id != trace_id);
                if ids.is_empty() {
                    self.time_index.remove(&start_ns);
                }
            }
        }
    }

    fn rebuild_bloom(&mut self) {
        self.bloom.clear();
        for &id in self.traces.keys() {
            self.bloom.insert(id);
        }
    }
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashMap;

    fn make_span(trace_id: TraceId, span_id: SpanId, svc: &str, op: &str) -> Span {
        Span {
            trace_id,
            span_id,
            parent_span_id: None,
            operation_name: op.into(),
            service_name: svc.into(),
            start_time_unix_nano: 1_000_000_000,
            end_time_unix_nano: 1_002_000_000,
            duration_ns: 2_000_000,
            status: SpanStatus::Ok,
            kind: SpanKind::Server,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
            tenant_id: "default".into(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        }
    }

    #[test]
    fn ingest_and_retrieve() {
        let mut store = TraceStore::new(RetentionPolicy::default());
        let span = make_span(1, 1, "svc", "get");
        store.ingest_spans(vec![span]);
        assert!(store.get_trace(1).is_some());
        assert!(store.get_trace(9999).is_none());
    }

    #[test]
    fn bloom_filter_may_contain() {
        let mut bloom = BloomFilter::new();
        bloom.insert(42u128);
        assert!(bloom.may_contain(42u128));
        // Note: may have false positives but not false negatives
        assert!(!bloom.may_contain(u128::MAX));
    }

    #[test]
    fn service_index() {
        let mut store = TraceStore::new(RetentionPolicy::default());
        store.ingest_spans(vec![
            make_span(1, 1, "frontend", "GET /"),
            make_span(2, 2, "backend", "query"),
        ]);
        let svcs = store.list_services(None);
        assert!(svcs.contains(&"frontend".into()));
        assert!(svcs.contains(&"backend".into()));
    }

    #[test]
    fn tag_index() {
        let mut store = TraceStore::new(RetentionPolicy::default());
        let mut span = make_span(1, 1, "svc", "op");
        span.tags.insert("http.method".into(), TagValue::String("GET".into()));
        store.ingest_spans(vec![span]);
        let vals = store.list_tag_values("http.method", None);
        assert!(vals.contains(&"GET".into()));
    }

    #[test]
    fn search_by_service() {
        let mut store = TraceStore::new(RetentionPolicy::default());
        store.ingest_spans(vec![
            make_span(1, 1, "alpha", "op"),
            make_span(2, 2, "beta", "op"),
        ]);
        let q = TraceSearchQuery {
            service: Some("alpha".into()),
            ..Default::default()
        };
        let results = store.search(&q);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].trace_id, 1);
    }

    #[test]
    fn retention_evicts_old_traces() {
        let mut store = TraceStore::new(RetentionPolicy {
            max_age_ns: 0, // everything is old immediately
            max_traces_per_tenant: 100_000,
        });
        store.ingest_spans(vec![make_span(1, 1, "svc", "op")]);
        assert_eq!(store.total_trace_count(), 1);
        let removed = store.apply_retention();
        assert_eq!(removed, 1);
        assert_eq!(store.total_trace_count(), 0);
    }

    #[test]
    fn retention_evicts_by_count() {
        let mut store = TraceStore::new(RetentionPolicy {
            max_age_ns: u64::MAX,
            max_traces_per_tenant: 2,
        });
        for i in 1u128..=5 {
            store.ingest_spans(vec![make_span(i, i as u64, "svc", "op")]);
        }
        store.apply_retention();
        assert!(store.total_trace_count() <= 2);
    }
}
