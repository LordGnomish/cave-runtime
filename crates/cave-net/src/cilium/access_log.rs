//! ProxyAccessLog — Envoy L7 access log shape consumed by the agent.
//!
//! Mirrors `pkg/proxy/accesslog/record.go`. Envoy ships every L7
//! request/response decision back to the agent over a unix-socket so
//! the agent can attribute the verdict to a Cilium identity, emit a
//! Hubble flow, and update L7 metrics.

use crate::cilium::types::{Cite, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessLogVerdict {
    Allowed,
    Denied,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AccessLogProtocol {
    Http,
    Grpc,
    Dns,
    Kafka,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowType {
    Request,
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpDetails {
    pub method: String,
    pub path: String,
    pub host: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsDetails {
    pub qname: String,
    pub qtype: String,
    pub rcode: String,
    pub answers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KafkaDetails {
    pub api_key: String,
    pub topic: String,
    pub correlation_id: i32,
    pub error_code: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum L7Protocol {
    Http(HttpDetails),
    Dns(DnsDetails),
    Kafka(KafkaDetails),
    /// Layer-4 only (TLS passthrough, etc.).
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessLogEntry {
    pub tenant: TenantId,
    pub time: DateTime<Utc>,
    pub flow_type: FlowType,
    pub verdict: AccessLogVerdict,
    pub protocol: AccessLogProtocol,
    pub source_identity: u32,
    pub destination_identity: u32,
    pub source_pod: String,
    pub destination_pod: String,
    pub l7: L7Protocol,
    pub policy_name: Option<String>,
    pub redirect_port: Option<u16>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AccessLogError {
    #[error("tenant {tenant} cannot ingest log owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct AccessLog {
    pub tenant: TenantId,
    pub capacity: usize,
    entries: VecDeque<AccessLogEntry>,
    overflow: u64,
    /// Per-protocol counters.
    counters: BTreeMap<AccessLogProtocol, u64>,
}

impl AccessLog {
    pub fn new(tenant: TenantId, capacity: usize) -> Self {
        Self {
            tenant, capacity,
            entries: VecDeque::with_capacity(capacity.min(1024)),
            overflow: 0,
            counters: BTreeMap::new(),
        }
    }

    pub fn ingest(&mut self, entry: AccessLogEntry) -> Result<(), AccessLogError> {
        if entry.tenant != self.tenant {
            return Err(AccessLogError::TenantDenied { tenant: entry.tenant });
        }
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
            self.overflow += 1;
        }
        *self.counters.entry(entry.protocol).or_insert(0) += 1;
        self.entries.push_back(entry);
        Ok(())
    }

    pub fn entries(&self) -> &VecDeque<AccessLogEntry> {
        &self.entries
    }

    pub fn count_for(&self, protocol: AccessLogProtocol) -> u64 {
        self.counters.get(&protocol).copied().unwrap_or(0)
    }

    pub fn overflow_count(&self) -> u64 {
        self.overflow
    }

    pub fn drain(&mut self) -> Vec<AccessLogEntry> {
        std::mem::take(&mut self.entries).into_iter().collect()
    }

    /// Filter entries by verdict (used by `cilium monitor --verdict denied`).
    pub fn by_verdict(&self, v: AccessLogVerdict) -> Vec<&AccessLogEntry> {
        self.entries.iter().filter(|e| e.verdict == v).collect()
    }

    /// Filter by protocol.
    pub fn by_protocol(&self, p: AccessLogProtocol) -> Vec<&AccessLogEntry> {
        self.entries.iter().filter(|e| e.protocol == p).collect()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/proxy/accesslog/record.go", "LogRecord");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn http_req(tenant: &str, status: u16, verdict: AccessLogVerdict) -> AccessLogEntry {
        AccessLogEntry {
            tenant: TenantId::new(tenant), time: Utc::now(),
            flow_type: FlowType::Request, verdict,
            protocol: AccessLogProtocol::Http,
            source_identity: 256, destination_identity: 257,
            source_pod: "ns/client".into(), destination_pod: "ns/server".into(),
            l7: L7Protocol::Http(HttpDetails {
                method: "GET".into(), path: "/api/users".into(), host: "api.example.com".into(),
                status, headers: vec![("user-agent".into(), "test".into())],
                bytes: 1024,
            }),
            policy_name: Some("allow-api".into()),
            redirect_port: Some(15001),
        }
    }

    fn dns_req(tenant: &str) -> AccessLogEntry {
        AccessLogEntry {
            tenant: TenantId::new(tenant), time: Utc::now(),
            flow_type: FlowType::Request, verdict: AccessLogVerdict::Allowed,
            protocol: AccessLogProtocol::Dns,
            source_identity: 256, destination_identity: 2 /* world */,
            source_pod: "ns/client".into(), destination_pod: "kube-system/kube-dns".into(),
            l7: L7Protocol::Dns(DnsDetails {
                qname: "api.example.com".into(), qtype: "A".into(),
                rcode: "NOERROR".into(),
                answers: vec!["1.2.3.4".into()],
            }),
            policy_name: None, redirect_port: None,
        }
    }

    // ── AccessLogEntry shape ────────────────────────────────────────────────

    #[test]
    fn http_entry_carries_status_and_method() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "LogRecord.Http", "tenant-al-h");
        let e = http_req("tenant-al-h", 200, AccessLogVerdict::Allowed);
        match e.l7 {
            L7Protocol::Http(d) => {
                assert_eq!(d.status, 200);
                assert_eq!(d.method, "GET");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn dns_entry_carries_qname_and_answers() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "LogRecord.Dns", "tenant-al-d");
        let e = dns_req("tenant-al-d");
        match e.l7 {
            L7Protocol::Dns(d) => {
                assert_eq!(d.qname, "api.example.com");
                assert_eq!(d.answers, vec!["1.2.3.4".to_string()]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn kafka_entry_round_trips() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "LogRecord.Kafka", "tenant-al-k");
        let k = KafkaDetails { api_key: "Produce".into(), topic: "orders".into(), correlation_id: 42, error_code: 0 };
        let s = serde_json::to_string(&k).unwrap();
        let back: KafkaDetails = serde_json::from_str(&s).unwrap();
        assert_eq!(back, k);
    }

    // ── Ingest ──────────────────────────────────────────────────────────────

    #[test]
    fn ingest_records_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Ingest", "tenant-al-ing");
        let mut log = AccessLog::new(tenant.clone(), 100);
        log.ingest(http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed)).unwrap();
        assert_eq!(log.entries().len(), 1);
    }

    #[test]
    fn ingest_cross_tenant_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Ingest.Tenant", "tenant-al-iso");
        let mut log = AccessLog::new(tenant, 100);
        let err = log.ingest(http_req("other", 200, AccessLogVerdict::Allowed)).unwrap_err();
        assert!(matches!(err, AccessLogError::TenantDenied { .. }));
    }

    #[test]
    fn ingest_increments_protocol_counter() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Ingest.Counter", "tenant-al-cnt");
        let mut log = AccessLog::new(tenant.clone(), 100);
        log.ingest(http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed)).unwrap();
        log.ingest(http_req(tenant.as_str(), 404, AccessLogVerdict::Allowed)).unwrap();
        log.ingest(dns_req(tenant.as_str())).unwrap();
        assert_eq!(log.count_for(AccessLogProtocol::Http), 2);
        assert_eq!(log.count_for(AccessLogProtocol::Dns), 1);
    }

    #[test]
    fn ingest_evicts_oldest_when_full() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Ingest.Eviction", "tenant-al-ev");
        let mut log = AccessLog::new(tenant.clone(), 3);
        for s in [200u16, 201, 202, 203, 204] {
            log.ingest(http_req(tenant.as_str(), s, AccessLogVerdict::Allowed)).unwrap();
        }
        assert_eq!(log.entries().len(), 3);
        assert_eq!(log.overflow_count(), 2);
    }

    // ── Drain ───────────────────────────────────────────────────────────────

    #[test]
    fn drain_returns_all_in_order() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Drain", "tenant-al-drn");
        let mut log = AccessLog::new(tenant.clone(), 100);
        log.ingest(http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed)).unwrap();
        log.ingest(http_req(tenant.as_str(), 201, AccessLogVerdict::Allowed)).unwrap();
        let drained = log.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(log.entries().len(), 0);
    }

    // ── Filters ─────────────────────────────────────────────────────────────

    #[test]
    fn by_verdict_returns_only_matches() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Filter.Verdict", "tenant-al-fv");
        let mut log = AccessLog::new(tenant.clone(), 100);
        log.ingest(http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed)).unwrap();
        log.ingest(http_req(tenant.as_str(), 403, AccessLogVerdict::Denied)).unwrap();
        log.ingest(http_req(tenant.as_str(), 500, AccessLogVerdict::Error)).unwrap();
        assert_eq!(log.by_verdict(AccessLogVerdict::Denied).len(), 1);
        assert_eq!(log.by_verdict(AccessLogVerdict::Allowed).len(), 1);
        assert_eq!(log.by_verdict(AccessLogVerdict::Error).len(), 1);
    }

    #[test]
    fn by_protocol_returns_only_matches() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Filter.Protocol", "tenant-al-fp");
        let mut log = AccessLog::new(tenant.clone(), 100);
        log.ingest(http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed)).unwrap();
        log.ingest(http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed)).unwrap();
        log.ingest(dns_req(tenant.as_str())).unwrap();
        assert_eq!(log.by_protocol(AccessLogProtocol::Http).len(), 2);
        assert_eq!(log.by_protocol(AccessLogProtocol::Dns).len(), 1);
    }

    #[test]
    fn by_protocol_empty_when_no_matches() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Filter.Protocol.None", "tenant-al-fpn");
        let mut log = AccessLog::new(tenant.clone(), 100);
        log.ingest(dns_req(tenant.as_str())).unwrap();
        assert!(log.by_protocol(AccessLogProtocol::Http).is_empty());
    }

    // ── L7Protocol variants ────────────────────────────────────────────────

    #[test]
    fn l7_protocol_none_for_l4_only_traffic() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "L7Protocol.None", "tenant-al-l4");
        let p = L7Protocol::None;
        let s = serde_json::to_string(&p).unwrap();
        let back: L7Protocol = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    // ── Counters ────────────────────────────────────────────────────────────

    #[test]
    fn count_for_returns_zero_for_unrecorded_protocol() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Counter.Zero", "tenant-al-cz");
        let log = AccessLog::new(tenant, 100);
        assert_eq!(log.count_for(AccessLogProtocol::Kafka), 0);
    }

    #[test]
    fn overflow_count_zero_when_under_capacity() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Overflow.Zero", "tenant-al-ovz");
        let mut log = AccessLog::new(tenant.clone(), 100);
        log.ingest(http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed)).unwrap();
        assert_eq!(log.overflow_count(), 0);
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn access_log_entry_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Entry.Serde", "tenant-al-eserde");
        let e = http_req("tenant-al-eserde", 200, AccessLogVerdict::Allowed);
        let s = serde_json::to_string(&e).unwrap();
        let back: AccessLogEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back.tenant, e.tenant);
        assert_eq!(back.verdict, e.verdict);
        assert_eq!(back.protocol, e.protocol);
    }

    #[test]
    fn http_details_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "HttpDetails.Serde", "tenant-al-hserde");
        let h = HttpDetails {
            method: "POST".into(), path: "/api".into(), host: "api.example.com".into(),
            status: 201, headers: vec![("content-type".into(), "application/json".into())],
            bytes: 512,
        };
        let s = serde_json::to_string(&h).unwrap();
        let back: HttpDetails = serde_json::from_str(&s).unwrap();
        assert_eq!(back, h);
    }

    #[test]
    fn dns_details_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "DnsDetails.Serde", "tenant-al-dserde");
        let d = DnsDetails {
            qname: "api.example.com".into(), qtype: "A".into(),
            rcode: "NOERROR".into(),
            answers: vec!["1.2.3.4".into(), "1.2.3.5".into()],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: DnsDetails = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn verdict_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Verdict.Serde", "tenant-al-vserde");
        for v in [AccessLogVerdict::Allowed, AccessLogVerdict::Denied, AccessLogVerdict::Error] {
            let s = serde_json::to_string(&v).unwrap();
            let back: AccessLogVerdict = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn protocol_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "Protocol.Serde", "tenant-al-pserde");
        for p in [AccessLogProtocol::Http, AccessLogProtocol::Grpc, AccessLogProtocol::Dns, AccessLogProtocol::Kafka, AccessLogProtocol::Tcp] {
            let s = serde_json::to_string(&p).unwrap();
            let back: AccessLogProtocol = serde_json::from_str(&s).unwrap();
            assert_eq!(back, p);
        }
    }

    // ── Edge: response flow ────────────────────────────────────────────────

    #[test]
    fn entry_with_response_flow_type() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/accesslog/record.go", "FlowType.Response", "tenant-al-resp");
        let mut log = AccessLog::new(tenant.clone(), 100);
        let mut e = http_req(tenant.as_str(), 200, AccessLogVerdict::Allowed);
        e.flow_type = FlowType::Response;
        log.ingest(e).unwrap();
        assert_eq!(log.entries()[0].flow_type, FlowType::Response);
    }
}
