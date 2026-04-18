//! Flow storage with ring-buffer eviction.

use crate::error::{HubbleError, HubbleResult};
use crate::models::*;
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::Mutex;
use uuid::Uuid;

const MAX_FLOWS: usize = 10_000;

pub struct FlowStore {
    flows: Mutex<VecDeque<Flow>>,
    total_ingested: std::sync::atomic::AtomicU64,
}

impl FlowStore {
    pub fn new() -> Self {
        Self {
            flows: Mutex::new(VecDeque::new()),
            total_ingested: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn ingest(&self, req: IngestFlowRequest) -> Flow {
        let flow = Flow {
            id: Uuid::new_v4(),
            time: Utc::now(),
            source: Endpoint {
                id: 0,
                namespace: req.source_ns.clone(),
                pod_name: req.source_pod,
                labels: vec![format!("k8s:io.kubernetes.pod.namespace={}", req.source_ns)],
                ip: req.source_ip,
                port: req.source_port,
            },
            destination: Endpoint {
                id: 0,
                namespace: req.dest_ns.clone(),
                pod_name: req.dest_pod,
                labels: vec![format!("k8s:io.kubernetes.pod.namespace={}", req.dest_ns)],
                ip: req.dest_ip,
                port: req.dest_port,
            },
            l4: L4Info {
                protocol: req.protocol.unwrap_or(L4Protocol::Tcp),
                src_port: req.source_port,
                dst_port: req.dest_port,
            },
            verdict: req.verdict,
            direction: req.direction,
            policy_match_type: 0,
            drop_reason: req.drop_reason,
            dns: req.dns,
            is_reply: None,
            node_name: req.node_name,
            labels: vec![],
        };
        let mut q = self.flows.lock().unwrap();
        q.push_back(flow.clone());
        let len = q.len();
        if len > MAX_FLOWS {
            let excess = len - MAX_FLOWS;
            q.drain(0..excess);
        }
        self.total_ingested.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        flow
    }

    pub fn get(&self, id: &str) -> HubbleResult<Flow> {
        let q = self.flows.lock().unwrap();
        let uuid = id.parse::<Uuid>().map_err(|_| HubbleError::FlowNotFound(id.to_owned()))?;
        q.iter().find(|f| f.id == uuid)
            .cloned()
            .ok_or_else(|| HubbleError::FlowNotFound(id.to_owned()))
    }

    pub fn query(&self, filter: &FlowFilter) -> Vec<Flow> {
        let q = self.flows.lock().unwrap();
        let limit = filter.limit.unwrap_or(100).min(1000);
        q.iter().rev()
            .filter(|f| {
                if let Some(ns) = &filter.source_namespace {
                    if &f.source.namespace != ns { return false; }
                }
                if let Some(ns) = &filter.dest_namespace {
                    if &f.destination.namespace != ns { return false; }
                }
                if let Some(v) = &filter.verdict {
                    if &f.verdict != v { return false; }
                }
                if let Some(d) = &filter.direction {
                    if &f.direction != d { return false; }
                }
                if let Some(n) = &filter.node_name {
                    if f.node_name.as_ref() != Some(n) { return false; }
                }
                true
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn stats(&self) -> FlowStats {
        let q = self.flows.lock().unwrap();
        let mut forwarded = 0u64;
        let mut dropped = 0u64;
        let mut ingress = 0u64;
        let mut egress = 0u64;
        for f in q.iter() {
            match f.verdict { FlowVerdict::Forwarded => forwarded += 1, FlowVerdict::Dropped => dropped += 1, _ => {} }
            match f.direction { TrafficDirection::Ingress => ingress += 1, TrafficDirection::Egress => egress += 1, }
        }
        FlowStats {
            total_flows: self.total_ingested.load(std::sync::atomic::Ordering::Relaxed),
            forwarded, dropped, ingress, egress,
        }
    }
}

impl Default for FlowStore {
    fn default() -> Self { Self::new() }
}
