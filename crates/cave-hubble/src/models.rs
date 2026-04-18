use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FlowVerdict {
    Forwarded,
    Dropped,
    Redirected,
    Error,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrafficDirection {
    Ingress,
    Egress,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum L4Protocol {
    Tcp,
    Udp,
    Icmp,
    Sctp,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: u64,
    pub namespace: String,
    pub pod_name: Option<String>,
    pub labels: Vec<String>,
    pub ip: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L4Info {
    pub protocol: L4Protocol,
    pub src_port: Option<u16>,
    pub dst_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsInfo {
    pub query: Option<String>,
    pub response: Option<Vec<String>>,
    pub rcode: Option<u32>,
    pub observation_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: Uuid,
    pub time: DateTime<Utc>,
    pub source: Endpoint,
    pub destination: Endpoint,
    pub l4: L4Info,
    pub verdict: FlowVerdict,
    pub direction: TrafficDirection,
    pub policy_match_type: u32,
    pub drop_reason: Option<String>,
    pub dns: Option<DnsInfo>,
    pub is_reply: Option<bool>,
    pub node_name: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    pub name: String,
    pub record_type: String,
    pub values: Vec<String>,
    pub ttl_secs: u64,
    pub observed_at: DateTime<Utc>,
    pub source_pod: Option<String>,
    pub source_namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStats {
    pub total_flows: u64,
    pub forwarded: u64,
    pub dropped: u64,
    pub ingress: u64,
    pub egress: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowFilter {
    pub source_namespace: Option<String>,
    pub dest_namespace: Option<String>,
    pub verdict: Option<FlowVerdict>,
    pub direction: Option<TrafficDirection>,
    pub node_name: Option<String>,
    pub label: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedFlow {
    pub key: String,
    pub source_namespace: String,
    pub dest_namespace: String,
    pub verdict: FlowVerdict,
    pub l4_protocol: L4Protocol,
    pub count: u64,
    pub last_seen: DateTime<Utc>,
    pub drop_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestFlowRequest {
    pub source_ns: String,
    pub source_pod: Option<String>,
    pub source_ip: String,
    pub source_port: Option<u16>,
    pub dest_ns: String,
    pub dest_pod: Option<String>,
    pub dest_ip: String,
    pub dest_port: Option<u16>,
    pub protocol: Option<L4Protocol>,
    pub verdict: FlowVerdict,
    pub direction: TrafficDirection,
    pub drop_reason: Option<String>,
    pub node_name: Option<String>,
    pub dns: Option<DnsInfo>,
}
