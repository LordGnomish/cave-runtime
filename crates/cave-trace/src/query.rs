use crate::error::*;
use crate::storage::TraceStore;
use crate::types::*;
use std::collections::HashSet;
use std::sync::Arc;

pub struct QueryEngine {
    store: Arc<TraceStore>,
}

impl QueryEngine {
    pub fn new(store: Arc<TraceStore>) -> Self {
        QueryEngine { store }
    }

    /// Main query method — filter + sort + limit
    pub fn find_traces(&self, q: &TraceQuery) -> Vec<Trace> {
        // Start with candidate trace IDs using indexes
        let candidate_ids: Option<HashSet<String>> = if let Some(svc) = &q.service_name {
            let ids: HashSet<String> = self
                .store
                .find_trace_ids_by_service(svc)
                .into_iter()
                .collect();
            Some(ids)
        } else if let Some(op) = &q.operation_name {
            let ids: HashSet<String> = self
                .store
                .find_trace_ids_by_operation(op)
                .into_iter()
                .collect();
            Some(ids)
        } else {
            None
        };

        // Further filter by tags if provided
        let tag_filtered: Option<HashSet<String>> =
            if let Some(tags) = &q.tags {
                let mut tag_ids: Option<HashSet<String>> = None;
                for (k, v) in tags {
                    let ids: HashSet<String> = self
                        .store
                        .find_trace_ids_by_tag(k, v)
                        .into_iter()
                        .collect();
                    tag_ids = Some(match tag_ids {
                        None => ids,
                        Some(existing) => existing.intersection(&ids).cloned().collect(),
                    });
                }
                tag_ids
            } else {
                None
            };

        // Merge index results
        let final_candidates: Option<HashSet<String>> = match (candidate_ids, tag_filtered) {
            (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        // Fetch traces
        let all_traces = if let Some(ids) = final_candidates {
            ids.into_iter()
                .filter_map(|id| self.store.get_trace(&id).ok())
                .collect::<Vec<_>>()
        } else {
            self.store.all_traces()
        };

        // Apply remaining filters
        let mut filtered: Vec<Trace> = all_traces
            .into_iter()
            .filter(|t| {
                if let Some(min) = q.min_duration_us {
                    if t.duration_us < min {
                        return false;
                    }
                }
                if let Some(max) = q.max_duration_us {
                    if t.duration_us > max {
                        return false;
                    }
                }
                if let Some(start) = q.start_time {
                    if t.start_time < start {
                        return false;
                    }
                }
                if let Some(end) = q.end_time {
                    if t.start_time > end {
                        return false;
                    }
                }
                // Re-apply service/operation filter in case we used all_traces path
                if let Some(svc) = &q.service_name {
                    if &t.service_name != svc {
                        return false;
                    }
                }
                if let Some(op) = &q.operation_name {
                    if &t.operation_name != op {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Sort by start_time descending
        filtered.sort_by(|a, b| b.start_time.cmp(&a.start_time));

        // Apply limit
        if let Some(limit) = q.limit {
            filtered.truncate(limit);
        }

        filtered
    }

    pub fn get_trace(&self, trace_id: &str) -> TraceResult<Trace> {
        self.store.get_trace(trace_id)
    }

    pub fn traces_for_service(
        &self,
        service: &str,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Vec<Trace> {
        let q = TraceQuery {
            service_name: Some(service.to_string()),
            start_time: Some(start),
            end_time: Some(end),
            ..Default::default()
        };
        self.find_traces(&q)
    }

    pub fn slowest_traces(&self, service: Option<&str>, limit: usize) -> Vec<Trace> {
        let mut traces = if let Some(svc) = service {
            let q = TraceQuery {
                service_name: Some(svc.to_string()),
                ..Default::default()
            };
            self.find_traces(&q)
        } else {
            self.store.all_traces()
        };
        traces.sort_by(|a, b| b.duration_us.cmp(&a.duration_us));
        traces.truncate(limit);
        traces
    }

    pub fn error_traces(&self, service: Option<&str>, limit: usize) -> Vec<Trace> {
        let candidates = if let Some(svc) = service {
            let q = TraceQuery {
                service_name: Some(svc.to_string()),
                ..Default::default()
            };
            self.find_traces(&q)
        } else {
            self.store.all_traces()
        };
        let mut errors: Vec<Trace> = candidates
            .into_iter()
            .filter(|t| t.error_count > 0)
            .collect();
        errors.sort_by(|a, b| b.start_time.cmp(&a.start_time));
        errors.truncate(limit);
        errors
    }

    /// Throughput: traces per time bucket (bucket_secs wide)
    pub fn throughput(
        &self,
        service: &str,
        bucket_secs: u64,
    ) -> Vec<(chrono::DateTime<chrono::Utc>, u64)> {
        let traces = self.store.find_trace_ids_by_service(service);
        if traces.is_empty() {
            return vec![];
        }
        let trace_list: Vec<Trace> = traces
            .iter()
            .filter_map(|id| self.store.get_trace(id).ok())
            .collect();

        if trace_list.is_empty() {
            return vec![];
        }

        // Find min/max timestamps
        let min_ts = trace_list
            .iter()
            .map(|t| t.start_time.timestamp())
            .min()
            .unwrap_or(0);
        let max_ts = trace_list
            .iter()
            .map(|t| t.start_time.timestamp())
            .max()
            .unwrap_or(0);

        if bucket_secs == 0 {
            return vec![];
        }

        let num_buckets = ((max_ts - min_ts) / bucket_secs as i64) + 1;
        let mut buckets: Vec<u64> = vec![0; num_buckets as usize];

        for trace in &trace_list {
            let idx = ((trace.start_time.timestamp() - min_ts) / bucket_secs as i64) as usize;
            if idx < buckets.len() {
                buckets[idx] += 1;
            }
        }

        buckets
            .into_iter()
            .enumerate()
            .map(|(i, count)| {
                let ts = chrono::DateTime::from_timestamp(
                    min_ts + (i as i64 * bucket_secs as i64),
                    0,
                )
                .unwrap_or_else(chrono::Utc::now);
                (ts, count)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_span_with_tag(
        trace_id: &str,
        span_id: &str,
        service: &str,
        op: &str,
        duration_us: i64,
        status: SpanStatus,
        tag_key: Option<&str>,
        tag_val: Option<&str>,
    ) -> Span {
        let now = Utc::now();
        let mut tags = HashMap::new();
        if let (Some(k), Some(v)) = (tag_key, tag_val) {
            tags.insert(k.to_string(), AttributeValue::String(v.to_string()));
        }
        Span {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: None,
            operation_name: op.to_string(),
            service_name: service.to_string(),
            start_time: now,
            end_time: now,
            duration_us,
            status,
            kind: SpanKind::Server,
            tags,
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
        }
    }

    #[test]
    fn query_by_service() {
        let store = Arc::new(TraceStore::new(100));
        store
            .ingest_spans(vec![make_span_with_tag(
                "t1",
                "s1",
                "service-a",
                "op",
                1000,
                SpanStatus::Ok,
                None,
                None,
            )])
            .unwrap();
        store
            .ingest_spans(vec![make_span_with_tag(
                "t2",
                "s2",
                "service-b",
                "op",
                2000,
                SpanStatus::Ok,
                None,
                None,
            )])
            .unwrap();

        let engine = QueryEngine::new(store);
        let q = TraceQuery {
            service_name: Some("service-a".to_string()),
            ..Default::default()
        };
        let results = engine.find_traces(&q);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].service_name, "service-a");
    }

    #[test]
    fn query_by_duration() {
        let store = Arc::new(TraceStore::new(100));
        store
            .ingest_spans(vec![make_span_with_tag(
                "t1",
                "s1",
                "svc",
                "fast",
                500,
                SpanStatus::Ok,
                None,
                None,
            )])
            .unwrap();
        store
            .ingest_spans(vec![make_span_with_tag(
                "t2",
                "s2",
                "svc",
                "slow",
                10000,
                SpanStatus::Ok,
                None,
                None,
            )])
            .unwrap();

        let engine = QueryEngine::new(store);
        let q = TraceQuery {
            min_duration_us: Some(1000),
            ..Default::default()
        };
        let results = engine.find_traces(&q);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].operation_name, "slow");
    }

    #[test]
    fn query_by_tags() {
        let store = Arc::new(TraceStore::new(100));
        store
            .ingest_spans(vec![make_span_with_tag(
                "t1",
                "s1",
                "svc",
                "op",
                1000,
                SpanStatus::Ok,
                Some("env"),
                Some("staging"),
            )])
            .unwrap();
        store
            .ingest_spans(vec![make_span_with_tag(
                "t2",
                "s2",
                "svc",
                "op",
                1000,
                SpanStatus::Ok,
                Some("env"),
                Some("prod"),
            )])
            .unwrap();

        let engine = QueryEngine::new(store);
        let mut tags = HashMap::new();
        tags.insert("env".to_string(), "staging".to_string());
        let q = TraceQuery {
            tags: Some(tags),
            ..Default::default()
        };
        let results = engine.find_traces(&q);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].trace_id, "t1");
    }

    #[test]
    fn query_slowest() {
        let store = Arc::new(TraceStore::new(100));
        for (i, dur) in [100, 500, 1000, 5000, 300].iter().enumerate() {
            store
                .ingest_spans(vec![make_span_with_tag(
                    &format!("t{}", i),
                    &format!("s{}", i),
                    "svc",
                    "op",
                    *dur,
                    SpanStatus::Ok,
                    None,
                    None,
                )])
                .unwrap();
        }
        let engine = QueryEngine::new(store);
        let slowest = engine.slowest_traces(None, 3);
        assert_eq!(slowest.len(), 3);
        assert_eq!(slowest[0].duration_us, 5000);
        assert_eq!(slowest[1].duration_us, 1000);
        assert_eq!(slowest[2].duration_us, 500);
    }

    #[test]
    fn query_error_traces() {
        let store = Arc::new(TraceStore::new(100));
        store
            .ingest_spans(vec![make_span_with_tag(
                "t1",
                "s1",
                "svc",
                "op",
                1000,
                SpanStatus::Error,
                None,
                None,
            )])
            .unwrap();
        store
            .ingest_spans(vec![make_span_with_tag(
                "t2",
                "s2",
                "svc",
                "op",
                1000,
                SpanStatus::Ok,
                None,
                None,
            )])
            .unwrap();
        let engine = QueryEngine::new(store);
        let errors = engine.error_traces(None, 10);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].trace_id, "t1");
    }
}
