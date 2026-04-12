//! CAVE DNS — DNS record management.
//!
//! Replaces: external-dns
//! Multi-provider DNS sync, drift detection, record validation, health probes.

pub mod manager;
pub mod models;
pub mod routes;

use axum::Router;
use models::{DnsRecord, DnsZone};
use std::sync::{Arc, Mutex};

pub struct DnsState {
    pub zones: Arc<Mutex<Vec<DnsZone>>>,
    pub records: Arc<Mutex<Vec<DnsRecord>>>,
}

impl Default for DnsState {
    fn default() -> Self {
        Self {
            zones: Arc::new(Mutex::new(Vec::new())),
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

pub fn router(state: Arc<DnsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "dns";
//! CAVE DNS — CoreDNS replacement.
//! DNS server with UDP+TCP, all record types, zone management, service discovery.
pub mod cache;
pub mod discovery;
pub mod error;
pub mod forward;
pub mod message;
pub mod plugin;
pub mod resolver;
pub mod server;
pub mod types;
pub mod zone;
pub use error::{DnsError, DnsResult};
pub use server::DnsServer;
pub use zone::ZoneStore;
#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use crate::cache::DnsCache;
    use crate::discovery::{ServiceEndpoint, ServiceRegistry};
    use crate::message::{decode, encode};
    use crate::plugin::{BlocklistPlugin, PluginChain};
    use crate::resolver::Resolver;
    use crate::types::*;
    use crate::zone::ZoneStore;
    // Helper: build a minimal query DnsMessage
    fn make_query(id: u16, name: &str, qtype: RecordType) -> DnsMessage {
        DnsMessage {
            header: Header {
                id,
                qr: false,
                opcode: 0,
                aa: false,
                tc: false,
                rd: true,
                ra: false,
                z: 0,
                rcode: RCODE_OK,
            },
            questions: vec![Question {
                name: name.to_string(),
                qtype,
                qclass: CLASS_IN,
            }],
            answers: vec![],
            authority: vec![],
            additional: vec![],
    // ── Test 1: record_type_to_u16 ──────────────────────────────────────────
    #[test]
    fn record_type_to_u16() {
        assert_eq!(RecordType::A.to_u16(), 1);
        assert_eq!(RecordType::NS.to_u16(), 2);
        assert_eq!(RecordType::CNAME.to_u16(), 5);
        assert_eq!(RecordType::SOA.to_u16(), 6);
        assert_eq!(RecordType::PTR.to_u16(), 12);
        assert_eq!(RecordType::MX.to_u16(), 15);
        assert_eq!(RecordType::TXT.to_u16(), 16);
        assert_eq!(RecordType::AAAA.to_u16(), 28);
        assert_eq!(RecordType::SRV.to_u16(), 33);
        assert_eq!(RecordType::CAA.to_u16(), 257);
    // ── Test 2: encode_decode_a_record ──────────────────────────────────────
    #[test]
    fn encode_decode_a_record() {
        let mut msg = make_query(1234, "example.com.", RecordType::A);
        msg.header.qr = true;
        msg.header.aa = true;
        msg.answers.push(ResourceRecord {
            name: "example.com.".to_string(),
            rtype: RecordType::A,
            class: CLASS_IN,
            ttl: 300,
            rdata: RData::A(Ipv4Addr::new(1, 2, 3, 4)),
        });
        let bytes = encode(&msg).expect("encode failed");
        let decoded = decode(&bytes).expect("decode failed");
        assert_eq!(decoded.header.id, 1234);
        assert_eq!(decoded.header.qr, true);
        assert_eq!(decoded.header.aa, true);
        assert_eq!(decoded.answers.len(), 1);
        assert_eq!(decoded.answers[0].ttl, 300);
        match &decoded.answers[0].rdata {
            RData::A(ip) => assert_eq!(*ip, Ipv4Addr::new(1, 2, 3, 4)),
            other => panic!("expected A record, got {:?}", other),
    // ── Test 3: encode_decode_mx_record ─────────────────────────────────────
    #[test]
    fn encode_decode_mx_record() {
        let mut msg = make_query(42, "example.com.", RecordType::MX);
        msg.header.qr = true;
        msg.answers.push(ResourceRecord {
            name: "example.com.".to_string(),
            rtype: RecordType::MX,
            class: CLASS_IN,
            ttl: 3600,
            rdata: RData::MX {
                priority: 10,
                exchange: "mail.example.com.".to_string(),
            },
        });
        let bytes = encode(&msg).expect("encode");
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded.answers.len(), 1);
        match &decoded.answers[0].rdata {
            RData::MX { priority, exchange } => {
                assert_eq!(*priority, 10);
                assert_eq!(exchange, "mail.example.com.");
            other => panic!("expected MX, got {:?}", other),
    // ── Test 4: encode_decode_txt_record ─────────────────────────────────────
    #[test]
    fn encode_decode_txt_record() {
        let mut msg = make_query(99, "example.com.", RecordType::TXT);
        msg.header.qr = true;
        msg.answers.push(ResourceRecord {
            name: "example.com.".to_string(),
            rtype: RecordType::TXT,
            class: CLASS_IN,
            ttl: 60,
            rdata: RData::TXT(vec![b"v=spf1 include:example.com ~all".to_vec()]),
        });
        let bytes = encode(&msg).expect("encode");
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded.answers.len(), 1);
        match &decoded.answers[0].rdata {
            RData::TXT(strings) => {
                assert_eq!(strings.len(), 1);
                assert_eq!(strings[0], b"v=spf1 include:example.com ~all");
            other => panic!("expected TXT, got {:?}", other),
    // ── Test 5: encode_decode_srv_record ─────────────────────────────────────
    #[test]
    fn encode_decode_srv_record() {
        let mut msg = make_query(77, "_http._tcp.example.com.", RecordType::SRV);
        msg.header.qr = true;
        msg.answers.push(ResourceRecord {
            name: "_http._tcp.example.com.".to_string(),
            rtype: RecordType::SRV,
            class: CLASS_IN,
            ttl: 120,
            rdata: RData::SRV {
                priority: 0,
                weight: 100,
                port: 8080,
                target: "web.example.com.".to_string(),
            },
        });
        let bytes = encode(&msg).expect("encode");
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded.answers.len(), 1);
        match &decoded.answers[0].rdata {
            RData::SRV { priority, weight, port, target } => {
                assert_eq!(*priority, 0);
                assert_eq!(*weight, 100);
                assert_eq!(*port, 8080);
                assert_eq!(target, "web.example.com.");
            other => panic!("expected SRV, got {:?}", other),
    // ── Test 6: encode_decode_soa_record ─────────────────────────────────────
    #[test]
    fn encode_decode_soa_record() {
        let mut msg = make_query(55, "example.com.", RecordType::SOA);
        msg.header.qr = true;
        msg.answers.push(ResourceRecord {
            name: "example.com.".to_string(),
            rtype: RecordType::SOA,
            class: CLASS_IN,
            ttl: 3600,
            rdata: RData::SOA {
                mname: "ns1.example.com.".to_string(),
                rname: "admin.example.com.".to_string(),
                serial: 2024010101,
                refresh: 3600,
                retry: 900,
                expire: 604800,
                minimum: 300,
            },
        });
        let bytes = encode(&msg).expect("encode");
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded.answers.len(), 1);
        match &decoded.answers[0].rdata {
            RData::SOA { mname, rname, serial, refresh, retry, expire, minimum } => {
                assert_eq!(mname, "ns1.example.com.");
                assert_eq!(rname, "admin.example.com.");
                assert_eq!(*serial, 2024010101);
                assert_eq!(*refresh, 3600);
                assert_eq!(*retry, 900);
                assert_eq!(*expire, 604800);
                assert_eq!(*minimum, 300);
            other => panic!("expected SOA, got {:?}", other),
    // ── Test 7: zone_add_lookup ──────────────────────────────────────────────
    #[test]
    fn zone_add_lookup() {
        let store = ZoneStore::new();
        let soa = ResourceRecord {
            name: "example.com.".to_string(),
            rtype: RecordType::SOA,
            class: CLASS_IN,
            ttl: 3600,
            rdata: RData::SOA {
                mname: "ns1.example.com.".to_string(),
                rname: "admin.example.com.".to_string(),
                serial: 1,
                refresh: 3600,
                retry: 900,
                expire: 604800,
                minimum: 300,
            },
        };
        let zone = crate::zone::Zone {
            origin: "example.com.".to_string(),
            soa: soa.clone(),
            records: std::collections::HashMap::new(),
        };
        store.add_zone(zone).expect("add_zone");
        store
            .add_record(
                "example.com.",
                ResourceRecord {
                    name: "www.example.com.".to_string(),
                    rtype: RecordType::A,
                    class: CLASS_IN,
                    ttl: 300,
                    rdata: RData::A(Ipv4Addr::new(192, 168, 1, 100)),
                },
            )
            .expect("add_record");
        let records = store.lookup("www.example.com.", &RecordType::A);
        assert_eq!(records.len(), 1);
        match &records[0].rdata {
            RData::A(ip) => assert_eq!(*ip, Ipv4Addr::new(192, 168, 1, 100)),
            other => panic!("unexpected: {:?}", other),
    // ── Test 8: zone_file_parse ──────────────────────────────────────────────
    #[test]
    fn zone_file_parse() {
        let store = ZoneStore::new();
        let content = "\
$ORIGIN example.com.
$TTL 3600
@   IN  SOA ns1 admin 2024010101 3600 900 604800 300
@   IN  NS  ns1
ns1 IN  A   192.168.1.1
www IN  A   192.168.1.100
";
        let zone = store
            .parse_zone_file(content, "example.com.")
            .expect("parse_zone_file");
        assert_eq!(zone.origin, "example.com.");
        // Should have records
        let ns1_records = store.lookup("ns1.example.com.", &RecordType::A);
        assert!(
            !ns1_records.is_empty() || zone.records.contains_key("ns1.example.com."),
            "ns1 A record should exist"
        );
    // ── Test 9: cname_chain ──────────────────────────────────────────────────
    #[test]
    fn cname_chain() {
        let store = Arc::new(ZoneStore::new());
        let cache = Arc::new(DnsCache::new(100));
        let soa_rr = ResourceRecord {
            name: "example.com.".to_string(),
            rtype: RecordType::SOA,
            class: CLASS_IN,
            ttl: 3600,
            rdata: RData::SOA {
                mname: "ns1.example.com.".to_string(),
                rname: "admin.example.com.".to_string(),
                serial: 1,
                refresh: 3600,
                retry: 900,
                expire: 604800,
                minimum: 300,
            },
        };
        store
            .add_zone(crate::zone::Zone {
                origin: "example.com.".to_string(),
                soa: soa_rr,
                records: std::collections::HashMap::new(),
            })
            .unwrap();
        // alias -> www -> 1.2.3.4
        store
            .add_record(
                "example.com.",
                ResourceRecord {
                    name: "alias.example.com.".to_string(),
                    rtype: RecordType::CNAME,
                    class: CLASS_IN,
                    ttl: 300,
                    rdata: RData::CNAME("www.example.com.".to_string()),
                },
            )
            .unwrap();
        store
            .add_record(
                "example.com.",
                ResourceRecord {
                    name: "www.example.com.".to_string(),
                    rtype: RecordType::A,
                    class: CLASS_IN,
                    ttl: 300,
                    rdata: RData::A(Ipv4Addr::new(1, 2, 3, 4)),
                },
            )
            .unwrap();
        let resolver = Resolver::new(store, cache);
        let query = make_query(1, "alias.example.com.", RecordType::A);
        let response = resolver.resolve(&query);
        // Should have CNAME + A
        assert!(
            response.answers.len() >= 1,
            "expected at least CNAME in answers"
        );
        let has_a = response
            .answers
            .iter()
            .any(|r| matches!(&r.rdata, RData::A(_)));
        assert!(has_a, "expected A record in CNAME chain answers");
    // ── Test 10: service_discovery_register_resolve ──────────────────────────
    #[test]
    fn service_discovery_register_resolve() {
        let registry = ServiceRegistry::new("cluster.local");
        let ep = ServiceEndpoint {
            name: "myservice".to_string(),
            namespace: "default".to_string(),
            cluster_domain: "cluster.local".to_string(),
            ip: Ipv4Addr::new(10, 0, 0, 1),
            port: 8080,
            protocol: "TCP".to_string(),
            ttl: 30,
        };
        let fqdn = ep.fqdn();
        registry.register(ep);
        let a_records = registry.resolve(&fqdn, &RecordType::A);
        assert_eq!(a_records.len(), 1);
        match &a_records[0].rdata {
            RData::A(ip) => assert_eq!(*ip, Ipv4Addr::new(10, 0, 0, 1)),
            other => panic!("expected A, got {:?}", other),
        let srv_records = registry.resolve(&fqdn, &RecordType::SRV);
        assert_eq!(srv_records.len(), 1);
        match &srv_records[0].rdata {
            RData::SRV { port, .. } => assert_eq!(*port, 8080),
            other => panic!("expected SRV, got {:?}", other),
    // ── Test 11: dns_cache_ttl ───────────────────────────────────────────────
    #[test]
    fn dns_cache_ttl() {
        let cache = DnsCache::new(100);
        let record = ResourceRecord {
            name: "test.example.com.".to_string(),
            rtype: RecordType::A,
            class: CLASS_IN,
            ttl: 5,
            rdata: RData::A(Ipv4Addr::new(9, 9, 9, 9)),
        };
        // Insert with 60 second TTL — should be retrievable immediately
        cache.insert("test.example.com.", RecordType::A.to_u16(), vec![record], 60);
        let result = cache.get("test.example.com.", RecordType::A.to_u16());
        assert!(result.is_some(), "record should be in cache");
        assert_eq!(result.unwrap().len(), 1);
        // Insert with 0 second TTL — expires immediately
        let record2 = ResourceRecord {
            name: "expired.example.com.".to_string(),
            rtype: RecordType::A,
            class: CLASS_IN,
            ttl: 0,
            rdata: RData::A(Ipv4Addr::new(1, 1, 1, 1)),
        };
        cache.insert("expired.example.com.", RecordType::A.to_u16(), vec![record2], 0);
        // evict_expired should remove it
        cache.evict_expired();
        let expired = cache.get("expired.example.com.", RecordType::A.to_u16());
        assert!(expired.is_none(), "expired record should be gone");
    // ── Test 12: plugin_chain_blocklist ─────────────────────────────────────
    #[test]
    fn plugin_chain_blocklist() {
        let mut chain = PluginChain::new();
        chain.add(Box::new(BlocklistPlugin::new(vec![
            "malware.example.com.".to_string(),
            "ads.tracking.io.".to_string(),
        ])));
        let query = make_query(1, "malware.example.com.", RecordType::A);
        let response = chain.process(&query);
        assert!(response.is_some(), "blocked domain should get a response");
        let resp = response.unwrap();
        assert_eq!(resp.header.rcode, RCODE_REFUSED);
        let safe_query = make_query(2, "safe.example.com.", RecordType::A);
        let safe_resp = chain.process(&safe_query);
        assert!(safe_resp.is_none(), "non-blocked domain should pass through");
    // ── Test 13: nxdomain_response ──────────────────────────────────────────
    #[test]
    fn nxdomain_response() {
        let store = Arc::new(ZoneStore::new());
        let cache = Arc::new(DnsCache::new(100));
        let soa_rr = ResourceRecord {
            name: "example.com.".to_string(),
            rtype: RecordType::SOA,
            class: CLASS_IN,
            ttl: 3600,
            rdata: RData::SOA {
                mname: "ns1.example.com.".to_string(),
                rname: "admin.example.com.".to_string(),
                serial: 1,
                refresh: 3600,
                retry: 900,
                expire: 604800,
                minimum: 300,
            },
        };
        store
            .add_zone(crate::zone::Zone {
                origin: "example.com.".to_string(),
                soa: soa_rr,
                records: std::collections::HashMap::new(),
            })
            .unwrap();
        let resolver = Resolver::new(store, cache);
        let query = make_query(5, "nonexistent.example.com.", RecordType::A);
        let response = resolver.resolve(&query);
        assert_eq!(response.header.rcode, RCODE_NXDOMAIN);
        assert!(response.header.qr);
