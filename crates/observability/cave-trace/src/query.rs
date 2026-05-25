// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query engine — wraps the store with higher-level search and analysis APIs.
//!
//! This layer sits between routes and storage; it owns no state itself.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::storage::TraceStore;
use crate::types::{
    LatencyHistogram, ServiceDependency, ServiceEdge, Span, SpanStatus, Trace, TraceId,
    TraceSearchQuery,
};
use crate::{Result, TraceError};

pub struct QueryEngine {
    store: Arc<RwLock<TraceStore>>,
}

impl QueryEngine {
    pub fn new(store: Arc<RwLock<TraceStore>>) -> Self {
        QueryEngine { store }
    }

    // ── Trace retrieval ────────────────────────────────────────────────────

    pub async fn get_trace(&self, trace_id: TraceId) -> Result<Trace> {
        let store = self.store.read().await;
        let record = store
            .get_trace(trace_id)
            .ok_or_else(|| TraceError::NotFound(crate::types::format_trace_id(trace_id)))?;
        record
            .to_trace()
            .ok_or_else(|| TraceError::StorageError("empty trace record".into()))
    }

    pub async fn get_trace_spans(&self, trace_id: TraceId) -> Result<Vec<Span>> {
        let store = self.store.read().await;
        let record = store
            .get_trace(trace_id)
            .ok_or_else(|| TraceError::NotFound(crate::types::format_trace_id(trace_id)))?;
        Ok(record.spans.clone())
    }

    // ── Search ─────────────────────────────────────────────────────────────

    pub async fn search(&self, query: &TraceSearchQuery) -> Result<Vec<Trace>> {
        let store = self.store.read().await;
        let records = store.search(query);
        let traces = records.into_iter().filter_map(|r| r.to_trace()).collect();
        Ok(traces)
    }

    // ── Services / operations ──────────────────────────────────────────────

    pub async fn list_services(&self, tenant_id: Option<&str>) -> Vec<String> {
        self.store.read().await.list_services(tenant_id)
    }

    pub async fn list_operations(&self, service: &str, tenant_id: Option<&str>) -> Vec<String> {
        self.store.read().await.list_operations(service, tenant_id)
    }

    pub async fn list_tag_names(&self, tenant_id: Option<&str>) -> Vec<String> {
        self.store.read().await.list_tag_names(tenant_id)
    }

    pub async fn list_tag_values(&self, tag_name: &str, tenant_id: Option<&str>) -> Vec<String> {
        self.store.read().await.list_tag_values(tag_name, tenant_id)
    }

    // ── Latency histograms ─────────────────────────────────────────────────

    pub async fn latency_histogram(
        &self,
        service: &str,
        operation: &str,
        start_ns: Option<u64>,
        end_ns: Option<u64>,
        tenant_id: Option<&str>,
    ) -> LatencyHistogram {
        self.store
            .read()
            .await
            .latency_histogram(service, operation, start_ns, end_ns, tenant_id)
    }

    /// All (service, operation) latency histograms for a tenant.
    pub async fn all_histograms(&self, tenant_id: Option<&str>) -> Vec<LatencyHistogram> {
        let store = self.store.read().await;
        let services = store.list_services(tenant_id);
        let mut out = Vec::new();
        for svc in &services {
            for op in store.list_operations(svc, tenant_id) {
                out.push(store.latency_histogram(svc, &op, None, None, tenant_id));
            }
        }
        out
    }

    // ── Error rate analysis ────────────────────────────────────────────────

    /// Returns (service, operation) → error_rate in [0,1] for the given window.
    pub async fn error_rates(
        &self,
        start_ns: Option<u64>,
        end_ns: Option<u64>,
        tenant_id: Option<&str>,
    ) -> HashMap<(String, String), f64> {
        let store = self.store.read().await;
        let mut counts: HashMap<(String, String), (u64, u64)> = HashMap::new(); // (total, errors)

        let records = if let Some(t) = tenant_id {
            store.all_records_for_tenant(t)
        } else {
            store.all_records_for_tenant("") // will be empty; let's iterate differently
        };

        // Iterate all traces (we can't call all_records easily without a tenant param)
        // Instead use all_records_for_tenant with a wildcard approach
        let records: Vec<_> = {
            let q = TraceSearchQuery {
                tenant_id: tenant_id.map(|t| t.to_owned()),
                start_time_ns: start_ns,
                end_time_ns: end_ns,
                limit: Some(10_000),
                ..Default::default()
            };
            store.search(&q)
        };

        for record in records {
            for span in &record.spans {
                if let (Some(s), Some(e)) = (start_ns, end_ns) {
                    if span.start_time_unix_nano < s || span.start_time_unix_nano > e {
                        continue;
                    }
                }
                let key = (span.service_name.clone(), span.operation_name.clone());
                let entry = counts.entry(key).or_insert((0, 0));
                entry.0 += 1;
                if span.has_error() {
                    entry.1 += 1;
                }
            }
        }

        counts
            .into_iter()
            .map(|(k, (total, errors))| {
                let rate = if total > 0 {
                    errors as f64 / total as f64
                } else {
                    0.0
                };
                (k, rate)
            })
            .collect()
    }

    // ── Dependency graph ───────────────────────────────────────────────────

    /// Build service → service call edges from parent/child span relationships.
    pub async fn service_dependencies(
        &self,
        start_ns: Option<u64>,
        end_ns: Option<u64>,
        tenant_id: Option<&str>,
    ) -> Vec<ServiceDependency> {
        let store = self.store.read().await;
        let q = TraceSearchQuery {
            tenant_id: tenant_id.map(|t| t.to_owned()),
            start_time_ns: start_ns,
            end_time_ns: end_ns,
            limit: Some(10_000),
            ..Default::default()
        };
        let records = store.search(&q);

        let mut edges: HashMap<ServiceEdge, (u64, u64, u64)> = HashMap::new(); // call, error, dur

        for record in records {
            let span_map: HashMap<u64, &Span> =
                record.spans.iter().map(|s| (s.span_id, s)).collect();

            for span in &record.spans {
                if let Some(parent_id) = span.parent_span_id {
                    if let Some(parent) = span_map.get(&parent_id) {
                        if parent.service_name != span.service_name {
                            let edge = ServiceEdge {
                                parent: parent.service_name.clone(),
                                child: span.service_name.clone(),
                            };
                            let e = edges.entry(edge).or_insert((0, 0, 0));
                            e.0 += 1;
                            if span.has_error() {
                                e.1 += 1;
                            }
                            e.2 += span.duration_ns;
                        }
                    }
                }
            }
        }

        edges
            .into_iter()
            .map(|(edge, (calls, errors, dur))| ServiceDependency {
                parent: edge.parent,
                child: edge.child,
                call_count: calls,
                error_count: errors,
                total_duration_ns: dur,
            })
            .collect()
    }

    // ── Trace comparison ───────────────────────────────────────────────────

    pub async fn compare_traces(
        &self,
        trace_a: TraceId,
        trace_b: TraceId,
    ) -> Result<TraceComparisonResult> {
        let (a, b) = tokio::try_join!(self.get_trace(trace_a), self.get_trace(trace_b),)?;
        Ok(compare(&a, &b))
    }

    // ── Critical path ──────────────────────────────────────────────────────

    pub async fn critical_path(&self, trace_id: TraceId) -> Result<CriticalPathResult> {
        let spans = self.get_trace_spans(trace_id).await?;
        let forest = crate::types::SpanNode::build_forest(&spans);
        let root = forest
            .into_iter()
            .next()
            .ok_or_else(|| TraceError::StorageError("no root span found".into()))?;
        let path_ids = root.critical_path();
        let path_spans: Vec<Span> = path_ids
            .iter()
            .filter_map(|id| spans.iter().find(|s| s.span_id == *id).cloned())
            .collect();
        let total_ns = path_spans.iter().map(|s| s.duration_ns).sum();
        Ok(CriticalPathResult {
            path_span_ids: path_ids,
            path_spans,
            total_duration_ns: total_ns,
        })
    }

    // ── Throughput (spans / traces per time bucket) ────────────────────────

    pub async fn throughput(
        &self,
        bucket_size_ns: u64,
        start_ns: u64,
        end_ns: u64,
        tenant_id: Option<&str>,
    ) -> Vec<ThroughputBucket> {
        let store = self.store.read().await;
        let q = TraceSearchQuery {
            tenant_id: tenant_id.map(|t| t.to_owned()),
            start_time_ns: Some(start_ns),
            end_time_ns: Some(end_ns),
            limit: Some(100_000),
            ..Default::default()
        };
        let records = store.search(&q);

        let num_buckets = ((end_ns - start_ns) / bucket_size_ns + 1) as usize;
        let mut buckets = vec![ThroughputBucket::default(); num_buckets.min(1000)];

        for record in records {
            let ts = record.trace_start_ns();
            if ts < start_ns || ts > end_ns {
                continue;
            }
            let idx = ((ts - start_ns) / bucket_size_ns) as usize;
            if let Some(b) = buckets.get_mut(idx) {
                b.trace_count += 1;
                b.span_count += record.columnar.span_count() as u64;
                b.error_count += record.columnar.error_count() as u64;
                b.start_time_ns = start_ns + idx as u64 * bucket_size_ns;
            }
        }
        buckets
    }
}

// ─── Comparison ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraceComparisonResult {
    pub trace_a_id: String,
    pub trace_b_id: String,
    /// Operations present in A but not B.
    pub only_in_a: Vec<String>,
    /// Operations present in B but not A.
    pub only_in_b: Vec<String>,
    /// Operations in both, with duration delta in ns.
    pub common: Vec<OperationDelta>,
    pub duration_delta_ns: i128,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationDelta {
    pub operation: String,
    pub service: String,
    pub duration_a_ns: u64,
    pub duration_b_ns: u64,
    pub delta_ns: i128,
    pub delta_pct: f64,
}

fn compare(a: &Trace, b: &Trace) -> TraceComparisonResult {
    // Key: (service, operation)
    let ops_a: HashMap<(String, String), u64> = a
        .spans
        .iter()
        .map(|s| {
            (
                (s.service_name.clone(), s.operation_name.clone()),
                s.duration_ns,
            )
        })
        .collect();
    let ops_b: HashMap<(String, String), u64> = b
        .spans
        .iter()
        .map(|s| {
            (
                (s.service_name.clone(), s.operation_name.clone()),
                s.duration_ns,
            )
        })
        .collect();

    let keys_a: HashSet<_> = ops_a.keys().collect();
    let keys_b: HashSet<_> = ops_b.keys().collect();

    let only_in_a: Vec<String> = keys_a
        .difference(&keys_b)
        .map(|(s, o)| format!("{}/{}", s, o))
        .collect();
    let only_in_b: Vec<String> = keys_b
        .difference(&keys_a)
        .map(|(s, o)| format!("{}/{}", s, o))
        .collect();

    let common: Vec<OperationDelta> = keys_a
        .intersection(&keys_b)
        .map(|(svc, op)| {
            let da = ops_a[&(svc.clone(), op.clone())];
            let db = ops_b[&(svc.clone(), op.clone())];
            let delta = db as i128 - da as i128;
            let pct = if da > 0 {
                delta as f64 / da as f64 * 100.0
            } else {
                0.0
            };
            OperationDelta {
                operation: op.clone(),
                service: svc.clone(),
                duration_a_ns: da,
                duration_b_ns: db,
                delta_ns: delta,
                delta_pct: pct,
            }
        })
        .collect();

    let dur_delta = b.duration_ns as i128 - a.duration_ns as i128;

    TraceComparisonResult {
        trace_a_id: crate::types::format_trace_id(a.trace_id),
        trace_b_id: crate::types::format_trace_id(b.trace_id),
        only_in_a,
        only_in_b,
        common,
        duration_delta_ns: dur_delta,
    }
}

// ─── Throughput ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ThroughputBucket {
    pub start_time_ns: u64,
    pub trace_count: u64,
    pub span_count: u64,
    pub error_count: u64,
}

// ─── Critical path ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CriticalPathResult {
    pub path_span_ids: Vec<u64>,
    pub path_spans: Vec<Span>,
    pub total_duration_ns: u64,
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::RetentionPolicy;
    use crate::types::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_store() -> Arc<RwLock<TraceStore>> {
        Arc::new(RwLock::new(TraceStore::new(RetentionPolicy::default())))
    }

    fn span(trace_id: TraceId, span_id: SpanId, svc: &str, op: &str) -> Span {
        Span {
            trace_id,
            span_id,
            parent_span_id: None,
            operation_name: op.into(),
            service_name: svc.into(),
            start_time_unix_nano: 1_000_000_000,
            end_time_unix_nano: 1_005_000_000,
            duration_ns: 5_000_000,
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

    #[tokio::test]
    async fn get_trace_not_found() {
        let store = make_store();
        let engine = QueryEngine::new(store);
        assert!(engine.get_trace(999).await.is_err());
    }

    #[tokio::test]
    async fn list_services_after_ingest() {
        let store = make_store();
        {
            let mut w = store.write().await;
            w.ingest_spans(vec![span(1, 1, "cart", "checkout")]);
        }
        let engine = QueryEngine::new(store);
        let svcs = engine.list_services(None).await;
        assert!(svcs.contains(&"cart".to_owned()));
    }

    #[tokio::test]
    async fn throughput_bucketing() {
        let store = make_store();
        {
            let mut w = store.write().await;
            w.ingest_spans(vec![span(1, 1, "svc", "op")]);
        }
        let engine = QueryEngine::new(store);
        let buckets = engine
            .throughput(1_000_000_000, 0, 2_000_000_000, None)
            .await;
        let total_traces: u64 = buckets.iter().map(|b| b.trace_count).sum();
        assert_eq!(total_traces, 1);
    }
}
