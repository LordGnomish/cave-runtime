use crate::error::*;
use crate::types::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[allow(dead_code)]
pub struct TraceStore {
    traces: Arc<RwLock<HashMap<String, Trace>>>,
    spans: Arc<RwLock<HashMap<String, Span>>>,
    service_index: Arc<RwLock<HashMap<String, Vec<String>>>>,
    operation_index: Arc<RwLock<HashMap<String, Vec<String>>>>,
    tag_index: Arc<RwLock<HashMap<String, Vec<String>>>>,
    max_traces: usize,
}

impl TraceStore {
    pub fn new(max_traces: usize) -> Self {
        TraceStore {
            traces: Arc::new(RwLock::new(HashMap::new())),
            spans: Arc::new(RwLock::new(HashMap::new())),
            service_index: Arc::new(RwLock::new(HashMap::new())),
            operation_index: Arc::new(RwLock::new(HashMap::new())),
            tag_index: Arc::new(RwLock::new(HashMap::new())),
            max_traces,
        }
    }

    /// Ingest a batch of spans (may belong to multiple trace IDs)
    pub fn ingest_spans(&self, spans: Vec<Span>) -> TraceResult<()> {
        if spans.is_empty() {
            return Ok(());
        }

        // Group by trace_id
        let mut by_trace: HashMap<String, Vec<Span>> = HashMap::new();
        for span in spans {
            by_trace
                .entry(span.trace_id.clone())
                .or_default()
                .push(span);
        }

        for (trace_id, new_spans) in by_trace {
            let mut traces = self.traces.write().unwrap();
            let mut spans_store = self.spans.write().unwrap();
            let mut service_idx = self.service_index.write().unwrap();
            let mut op_idx = self.operation_index.write().unwrap();
            let mut tag_idx = self.tag_index.write().unwrap();

            // Merge or create
            let all_spans = if let Some(existing) = traces.get(&trace_id) {
                let mut merged = existing.spans.clone();
                for span in &new_spans {
                    if !merged.iter().any(|s| s.span_id == span.span_id) {
                        merged.push(span.clone());
                    }
                }
                merged
            } else {
                new_spans.clone()
            };

            // Index spans
            for span in &new_spans {
                spans_store.insert(span.span_id.clone(), span.clone());

                // Service index
                let svc_entry = service_idx
                    .entry(span.service_name.clone())
                    .or_default();
                if !svc_entry.contains(&trace_id) {
                    svc_entry.push(trace_id.clone());
                }

                // Operation index
                let op_entry = op_idx
                    .entry(span.operation_name.clone())
                    .or_default();
                if !op_entry.contains(&trace_id) {
                    op_entry.push(trace_id.clone());
                }

                // Tag index
                for (k, v) in &span.tags {
                    let str_val = match v {
                        AttributeValue::String(s) => s.clone(),
                        AttributeValue::Bool(b) => b.to_string(),
                        AttributeValue::Int(i) => i.to_string(),
                        AttributeValue::Double(d) => d.to_string(),
                        AttributeValue::StringArray(a) => a.join(","),
                    };
                    let key = format!("{}:{}", k, str_val);
                    let tag_entry = tag_idx.entry(key).or_default();
                    if !tag_entry.contains(&trace_id) {
                        tag_entry.push(trace_id.clone());
                    }
                }
            }

            if let Some(trace) = Trace::from_spans(all_spans) {
                // Evict oldest if at capacity (simple: remove first inserted)
                if traces.len() >= self.max_traces && !traces.contains_key(&trace_id) {
                    if let Some(oldest) = traces.keys().next().cloned() {
                        traces.remove(&oldest);
                    }
                }
                traces.insert(trace_id.clone(), trace);
            }
        }

        Ok(())
    }

    pub fn get_trace(&self, trace_id: &str) -> TraceResult<Trace> {
        self.traces
            .read()
            .unwrap()
            .get(trace_id)
            .cloned()
            .ok_or_else(|| TraceError::TraceNotFound(trace_id.to_string()))
    }

    pub fn get_span(&self, span_id: &str) -> TraceResult<Span> {
        self.spans
            .read()
            .unwrap()
            .get(span_id)
            .cloned()
            .ok_or_else(|| TraceError::SpanNotFound(span_id.to_string()))
    }

    pub fn list_services(&self) -> Vec<String> {
        let mut svcs: Vec<String> = self.service_index.read().unwrap().keys().cloned().collect();
        svcs.sort();
        svcs
    }

    pub fn list_operations(&self, service_name: &str) -> Vec<String> {
        let traces = self.traces.read().unwrap();
        let service_trace_ids = self
            .service_index
            .read()
            .unwrap()
            .get(service_name)
            .cloned()
            .unwrap_or_default();

        let mut ops: std::collections::HashSet<String> = std::collections::HashSet::new();
        for tid in &service_trace_ids {
            if let Some(trace) = traces.get(tid) {
                for span in &trace.spans {
                    if span.service_name == service_name {
                        ops.insert(span.operation_name.clone());
                    }
                }
            }
        }
        let mut result: Vec<String> = ops.into_iter().collect();
        result.sort();
        result
    }

    pub fn trace_count(&self) -> usize {
        self.traces.read().unwrap().len()
    }

    pub fn delete_trace(&self, trace_id: &str) -> TraceResult<()> {
        let mut traces = self.traces.write().unwrap();
        let trace = traces
            .remove(trace_id)
            .ok_or_else(|| TraceError::TraceNotFound(trace_id.to_string()))?;

        // Remove spans
        let mut spans = self.spans.write().unwrap();
        for span in &trace.spans {
            spans.remove(&span.span_id);
        }

        // Clean up indexes
        let mut svc_idx = self.service_index.write().unwrap();
        for entry in svc_idx.values_mut() {
            entry.retain(|tid| tid != trace_id);
        }

        let mut op_idx = self.operation_index.write().unwrap();
        for entry in op_idx.values_mut() {
            entry.retain(|tid| tid != trace_id);
        }

        let mut tag_idx = self.tag_index.write().unwrap();
        for entry in tag_idx.values_mut() {
            entry.retain(|tid| tid != trace_id);
        }

        Ok(())
    }

    pub fn all_traces(&self) -> Vec<Trace> {
        self.traces.read().unwrap().values().cloned().collect()
    }

    pub fn find_trace_ids_by_service(&self, service: &str) -> Vec<String> {
        self.service_index
            .read()
            .unwrap()
            .get(service)
            .cloned()
            .unwrap_or_default()
    }

    pub fn find_trace_ids_by_operation(&self, operation: &str) -> Vec<String> {
        self.operation_index
            .read()
            .unwrap()
            .get(operation)
            .cloned()
            .unwrap_or_default()
    }

    pub fn find_trace_ids_by_tag(&self, key: &str, value: &str) -> Vec<String> {
        let index_key = format!("{}:{}", key, value);
        self.tag_index
            .read()
            .unwrap()
            .get(&index_key)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_span(
        trace_id: &str,
        span_id: &str,
        parent: Option<&str>,
        service: &str,
        op: &str,
        duration_us: i64,
        status: SpanStatus,
    ) -> Span {
        let now = Utc::now();
        let mut tags = HashMap::new();
        tags.insert(
            "env".to_string(),
            AttributeValue::String("prod".to_string()),
        );
        Span {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: parent.map(|s| s.to_string()),
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
    fn store_ingest_and_get() {
        let store = TraceStore::new(100);
        let spans = vec![
            make_span("t1", "s1", None, "svc-a", "http.get", 5000, SpanStatus::Ok),
            make_span(
                "t1",
                "s2",
                Some("s1"),
                "svc-b",
                "db.query",
                2000,
                SpanStatus::Ok,
            ),
        ];
        store.ingest_spans(spans).unwrap();
        let trace = store.get_trace("t1").unwrap();
        assert_eq!(trace.trace_id, "t1");
        assert_eq!(trace.span_count, 2);
    }

    #[test]
    fn store_list_services() {
        let store = TraceStore::new(100);
        store
            .ingest_spans(vec![make_span(
                "t1",
                "s1",
                None,
                "frontend",
                "GET /",
                1000,
                SpanStatus::Ok,
            )])
            .unwrap();
        store
            .ingest_spans(vec![make_span(
                "t2",
                "s2",
                None,
                "backend",
                "POST /api",
                2000,
                SpanStatus::Ok,
            )])
            .unwrap();
        let services = store.list_services();
        assert!(services.contains(&"frontend".to_string()));
        assert!(services.contains(&"backend".to_string()));
    }

    #[test]
    fn store_list_operations() {
        let store = TraceStore::new(100);
        store
            .ingest_spans(vec![
                make_span("t1", "s1", None, "api", "GET /users", 1000, SpanStatus::Ok),
                make_span(
                    "t1",
                    "s2",
                    Some("s1"),
                    "api",
                    "POST /users",
                    500,
                    SpanStatus::Ok,
                ),
            ])
            .unwrap();
        let ops = store.list_operations("api");
        assert!(ops.contains(&"GET /users".to_string()));
        assert!(ops.contains(&"POST /users".to_string()));
    }

    #[test]
    fn store_delete_trace() {
        let store = TraceStore::new(100);
        store
            .ingest_spans(vec![make_span(
                "t1",
                "s1",
                None,
                "svc",
                "op",
                1000,
                SpanStatus::Ok,
            )])
            .unwrap();
        assert!(store.get_trace("t1").is_ok());
        store.delete_trace("t1").unwrap();
        assert!(store.get_trace("t1").is_err());
    }
}
