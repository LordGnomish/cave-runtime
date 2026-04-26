//! Flow aggregation — groups flows by (src-ns, dst-ns, verdict, protocol).

use crate::models::{AggregatedFlow, Flow};
use chrono::Utc;
use dashmap::DashMap;

pub struct FlowAggregator {
    aggregated: DashMap<String, AggregatedFlow>,
}

impl FlowAggregator {
    pub fn new() -> Self {
        Self { aggregated: DashMap::new() }
    }

    pub fn record(&self, flow: &Flow) {
        let key = format!(
            "{}/{}/{:?}/{:?}",
            flow.source.namespace, flow.destination.namespace,
            flow.verdict, flow.l4.protocol
        );
        let mut entry = self.aggregated.entry(key.clone()).or_insert_with(|| AggregatedFlow {
            key: key.clone(),
            source_namespace: flow.source.namespace.clone(),
            dest_namespace: flow.destination.namespace.clone(),
            verdict: flow.verdict.clone(),
            l4_protocol: flow.l4.protocol.clone(),
            count: 0,
            last_seen: Utc::now(),
            drop_reasons: vec![],
        });
        entry.count += 1;
        entry.last_seen = Utc::now();
        if let Some(reason) = &flow.drop_reason {
            if !entry.drop_reasons.contains(reason) {
                entry.drop_reasons.push(reason.clone());
            }
        }
    }

    pub fn list(&self) -> Vec<AggregatedFlow> {
        self.aggregated.iter().map(|r| r.value().clone()).collect()
    }

    pub fn top_dropped(&self, limit: usize) -> Vec<AggregatedFlow> {
        let mut all = self.list();
        all.sort_by(|a, b| b.count.cmp(&a.count));
        all.into_iter()
            .filter(|f| !f.drop_reasons.is_empty())
            .take(limit)
            .collect()
    }
}

impl Default for FlowAggregator {
    fn default() -> Self { Self::new() }
}
