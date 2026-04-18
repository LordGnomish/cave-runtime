//! Filter parsing and evaluation for Hubble flow queries.

use crate::models::{FlowFilter, FlowVerdict, TrafficDirection};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct FlowQuery {
    pub source_namespace: Option<String>,
    pub dest_namespace: Option<String>,
    pub verdict: Option<String>,
    pub direction: Option<String>,
    pub node_name: Option<String>,
    pub limit: Option<usize>,
}

impl FlowQuery {
    pub fn into_filter(self) -> FlowFilter {
        FlowFilter {
            source_namespace: self.source_namespace,
            dest_namespace: self.dest_namespace,
            verdict: self.verdict.as_deref().and_then(parse_verdict),
            direction: self.direction.as_deref().and_then(parse_direction),
            node_name: self.node_name,
            label: None,
            limit: self.limit,
        }
    }
}

fn parse_verdict(s: &str) -> Option<FlowVerdict> {
    match s {
        "forwarded" => Some(FlowVerdict::Forwarded),
        "dropped" => Some(FlowVerdict::Dropped),
        "redirected" => Some(FlowVerdict::Redirected),
        "error" => Some(FlowVerdict::Error),
        "audit" => Some(FlowVerdict::Audit),
        _ => None,
    }
}

fn parse_direction(s: &str) -> Option<TrafficDirection> {
    match s {
        "ingress" => Some(TrafficDirection::Ingress),
        "egress" => Some(TrafficDirection::Egress),
        _ => None,
    }
}
