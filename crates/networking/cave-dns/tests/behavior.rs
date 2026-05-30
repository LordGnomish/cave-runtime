// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Portable-coverage integration tests for `cave-dns`.
//!
//! Upstream parity target: CoreDNS v1.14.3 (https://github.com/coredns/coredns).
//! These tests exercise the deterministic DNS protocol helper layer
//! (`protocol::message`, `protocol::records`, `protocol::edns`) and the
//! round-robin loadbalance plugin — the public, already-implemented surface
//! that maps directly to CoreDNS unit tests (TestErraticTruncate,
//! TestFilterRRSlice, TestNormalize, TestRoundRobinEmpty, EDNS payload
//! extraction) but had no Rust coverage.
//!
//! Note: the top-level `message`/`types` modules are NOT re-exported from
//! `lib.rs` (no `pub mod message`/`pub mod types`), so `message::encode`/
//! `decode` are unreachable as `cave_dns::...` and are intentionally omitted.

use cave_dns::plugins::{Plugin, PluginChain, Protocol, QueryContext};
use cave_dns::protocol::edns::EdnsOptions;
use cave_dns::protocol::message::{
    aaaa_record, dnssec_ok, edns_payload_size, make_error_response, make_query, truncate_to_udp,
    txt_record,
};
use cave_dns::protocol::records::{build_record, filter_by_type, parse_fqdn};

use hickory_proto::op::{Edns, Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::rdata::{A as HA, AAAA as HAAAA, CNAME as HCNAME};
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

// ── helpers ──────────────────────────────────────────────────────────────────

fn a_rec(name: &str, addr: Ipv4Addr) -> Record {
    let n = Name::from_str(name).unwrap();
    let mut r = Record::new();
    r.set_name(n);
    r.set_ttl(300);
    r.set_record_type(RecordType::A);
    r.set_dns_class(DNSClass::IN);
    r.set_data(Some(RData::A(HA(addr))));
    r
}

// ── protocol::message::truncate_to_udp ───────────────────────────────────────

#[test]
fn truncate_to_udp_sets_tc_and_clears_oversized() {
    // 40 A-records easily exceed a 64-byte UDP budget once encoded.
    let mut msg = Message::new();
    msg.set_id(7);
    for i in 0..40u8 {
        msg.add_answer(a_rec("host.example.com.", Ipv4Addr::new(10, 0, 0, i)));
    }
    assert_eq!(msg.answers().len(), 40);

    truncate_to_udp(&mut msg, 64);

    assert!(msg.truncated(), "TC bit must be set when over budget");
    assert!(
        msg.answers().is_empty(),
        "answers must be cleared on truncation"
    );
}

#[test]
fn truncate_to_udp_leaves_small_message_untouched() {
    // A single answer is well under a 512-byte budget: no TC, no clearing.
    let mut msg = Message::new();
    msg.set_id(7);
    msg.add_answer(a_rec("host.example.com.", Ipv4Addr::new(1, 2, 3, 4)));

    truncate_to_udp(&mut msg, 512);

    assert!(!msg.truncated(), "TC must stay clear when under budget");
    assert_eq!(msg.answers().len(), 1, "answers must be preserved");
}

// ── protocol::message::edns_payload_size + dnssec_ok ─────────────────────────

#[test]
fn edns_payload_size_defaults_512_else_advertised() {
    // No OPT pseudo-record → default 512.
    let plain = Message::new();
    assert_eq!(edns_payload_size(&plain), 512);

    // OPT advertising 4096 → that exact value is returned.
    let mut with_opt = Message::new();
    let mut edns = Edns::new();
    edns.set_max_payload(4096);
    with_opt.set_edns(edns);
    assert_eq!(edns_payload_size(&with_opt), 4096);
}

#[test]
fn dnssec_ok_reflects_do_bit() {
    // No OPT → DO is false.
    let plain = Message::new();
    assert!(!dnssec_ok(&plain));

    // OPT with DO set → true.
    let mut with_do = Message::new();
    let mut edns = Edns::new();
    edns.set_dnssec_ok(true);
    with_do.set_edns(edns);
    assert!(dnssec_ok(&with_do));
}

// ── protocol::message::make_error_response ───────────────────────────────────

#[test]
fn make_error_response_mirrors_query_and_carries_rcode() {
    let mut query = Message::new();
    query.set_id(0xBEEF);
    query.set_message_type(MessageType::Query);
    query.set_op_code(OpCode::Query);
    query.set_recursion_desired(true);
    let mut q = Query::new();
    q.set_name(Name::from_str("fail.example.com.").unwrap());
    q.set_query_type(RecordType::A);
    q.set_query_class(DNSClass::IN);
    query.add_query(q);

    let resp = make_error_response(&query, ResponseCode::ServFail);

    assert_eq!(resp.id(), 0xBEEF, "id mirrored from query");
    assert_eq!(resp.message_type(), MessageType::Response);
    assert_eq!(resp.queries().len(), 1, "query section copied");
    assert_eq!(
        resp.queries()[0].name(),
        &Name::from_str("fail.example.com.").unwrap()
    );
    assert_eq!(resp.response_code(), ResponseCode::ServFail);
    // make_response (the base) sets recursion_available(true).
    assert!(resp.recursion_available());
}

// ── protocol::message::make_query ────────────────────────────────────────────

#[test]
fn make_query_builds_recursive_in_query() {
    let msg = make_query("probe.example.com.", RecordType::AAAA).unwrap();

    assert_eq!(msg.message_type(), MessageType::Query);
    assert_eq!(msg.op_code(), OpCode::Query);
    assert!(msg.recursion_desired(), "RD bit must be set");
    assert_eq!(msg.queries().len(), 1);
    let q = &msg.queries()[0];
    assert_eq!(q.name(), &Name::from_str("probe.example.com.").unwrap());
    assert_eq!(q.query_type(), RecordType::AAAA);
    assert_eq!(q.query_class(), DNSClass::IN);
}

// ── protocol::message::aaaa_record / txt_record ──────────────────────────────

#[test]
fn aaaa_and_txt_records_build_with_expected_rdata() {
    let v6 = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
    let aaaa = aaaa_record("v6.example.com.", 600, v6).unwrap();
    assert_eq!(aaaa.record_type(), RecordType::AAAA);
    assert_eq!(aaaa.ttl(), 600);
    assert_eq!(aaaa.dns_class(), DNSClass::IN);
    match aaaa.data() {
        Some(RData::AAAA(HAAAA(addr))) => assert_eq!(*addr, v6),
        other => panic!("expected AAAA rdata, got {other:?}"),
    }

    let txt = txt_record("txt.example.com.", 120, vec!["hello".into(), "world".into()]).unwrap();
    assert_eq!(txt.record_type(), RecordType::TXT);
    assert_eq!(txt.ttl(), 120);
    assert_eq!(txt.dns_class(), DNSClass::IN);
    match txt.data() {
        Some(RData::TXT(t)) => {
            let chunks = t.txt_data();
            assert_eq!(chunks.len(), 2);
            assert_eq!(&*chunks[0], b"hello");
            assert_eq!(&*chunks[1], b"world");
        }
        other => panic!("expected TXT rdata, got {other:?}"),
    }

    // An invalid name (embedded space) produces a parse error rather than a panic.
    assert!(aaaa_record("not a name", 60, v6).is_err());
}

// ── protocol::records::filter_by_type ────────────────────────────────────────

#[test]
fn filter_by_type_keeps_qtype_and_cname_drops_rest() {
    let name = Name::from_str("mix.example.com.").unwrap();
    let a = build_record(
        name.clone(),
        300,
        DNSClass::IN,
        RData::A(HA(Ipv4Addr::new(1, 1, 1, 1))),
    );
    let aaaa = build_record(
        name.clone(),
        300,
        DNSClass::IN,
        RData::AAAA(HAAAA(Ipv6Addr::LOCALHOST)),
    );
    let cname = build_record(
        name.clone(),
        300,
        DNSClass::IN,
        RData::CNAME(HCNAME(Name::from_str("alias.example.com.").unwrap())),
    );
    let records = vec![a, aaaa, cname];

    // Querying A keeps the A record AND passes the CNAME through; AAAA dropped.
    let filtered = filter_by_type(&records, RecordType::A);
    assert_eq!(filtered.len(), 2);
    let types: Vec<RecordType> = filtered.iter().map(|r| r.record_type()).collect();
    assert!(types.contains(&RecordType::A));
    assert!(types.contains(&RecordType::CNAME));
    assert!(!types.contains(&RecordType::AAAA));

    // Querying MX (absent) still yields the always-passthrough CNAME only.
    let mx_filtered = filter_by_type(&records, RecordType::MX);
    assert_eq!(mx_filtered.len(), 1);
    assert_eq!(mx_filtered[0].record_type(), RecordType::CNAME);
}

// ── protocol::records::parse_fqdn ────────────────────────────────────────────

#[test]
fn parse_fqdn_normalizes_trailing_dot() {
    let with = parse_fqdn("example.com.").unwrap();
    let without = parse_fqdn("example.com").unwrap();
    assert_eq!(with, without, "both forms normalize to the same FQDN");
    assert!(with.is_fqdn(), "result must be a fully-qualified name");
    assert_eq!(with, Name::from_str("example.com.").unwrap());
}

// ── protocol::records::build_record ──────────────────────────────────────────

#[test]
fn build_record_derives_type_from_rdata() {
    let name = Name::from_str("derive.example.com.").unwrap();
    // record_type is taken from the RData, not passed explicitly.
    let rec = build_record(
        name.clone(),
        99,
        DNSClass::IN,
        RData::AAAA(HAAAA(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1))),
    );
    assert_eq!(rec.record_type(), RecordType::AAAA);
    assert_eq!(rec.ttl(), 99);
    assert_eq!(rec.dns_class(), DNSClass::IN);
    assert_eq!(rec.name(), &name);
}

// ── protocol::edns::EdnsOptions ──────────────────────────────────────────────

#[test]
fn edns_options_from_message_and_effective_size() {
    // No OPT → struct defaults (udp_payload_size 0, dnssec_ok false), but the
    // effective size clamps up to the 512 floor.
    let plain = Message::new();
    let opts = EdnsOptions::from_message(&plain);
    assert_eq!(opts.udp_payload_size, 0);
    assert!(!opts.dnssec_ok);
    assert_eq!(opts.effective_udp_size(), 512, "sub-512 clamps to 512");

    // OPT advertising 1232 with DO set → populated and passed through.
    let mut with_opt = Message::new();
    let mut edns = Edns::new();
    edns.set_max_payload(1232);
    edns.set_dnssec_ok(true);
    with_opt.set_edns(edns);
    let opts2 = EdnsOptions::from_message(&with_opt);
    assert_eq!(opts2.udp_payload_size, 1232);
    assert!(opts2.dnssec_ok);
    assert_eq!(opts2.effective_udp_size(), 1232, "larger size passes through");
}

// ── LoadbalancePlugin::handle (round-robin via plugin chain) ─────────────────

#[tokio::test]
async fn loadbalance_round_robin_rotates_by_one_per_query() {
    use cave_dns::config::{LbPolicy, LoadbalanceConfig};
    use cave_dns::plugins::loadbalance::LoadbalancePlugin;

    let plugin: Arc<dyn Plugin> = Arc::new(LoadbalancePlugin::new(LoadbalanceConfig {
        policy: LbPolicy::RoundRobin,
    }));
    let chain = PluginChain::new(vec![plugin]);
    let client: SocketAddr = "127.0.0.1:5353".parse().unwrap();

    // Three distinct A answers for the same name.
    let addrs = [
        Ipv4Addr::new(10, 0, 0, 1),
        Ipv4Addr::new(10, 0, 0, 2),
        Ipv4Addr::new(10, 0, 0, 3),
    ];

    let mut observed: Vec<Vec<Ipv4Addr>> = Vec::new();
    for _ in 0..3 {
        let req = make_query("lb.example.com.", RecordType::A).unwrap();
        let mut ctx = QueryContext::new(req, client, Protocol::Udp);
        // The loadbalance plugin rotates whatever answers already exist on the
        // response, so seed the response with the three A records in order.
        for ip in addrs {
            ctx.response.add_answer(a_rec("lb.example.com.", ip));
        }
        chain.execute(&mut ctx).await.unwrap();
        let order: Vec<Ipv4Addr> = ctx
            .response
            .answers()
            .iter()
            .filter_map(|r| match r.data() {
                Some(RData::A(HA(ip))) => Some(*ip),
                _ => None,
            })
            .collect();
        observed.push(order);
    }

    // counter starts at 0: fetch_add returns 0,1,2 → idx = 0,1,2.
    // Query 1 (idx 0): [.1, .2, .3]  (unrotated)
    // Query 2 (idx 1): [.2, .3, .1]
    // Query 3 (idx 2): [.3, .1, .2]
    assert_eq!(observed[0], vec![addrs[0], addrs[1], addrs[2]]);
    assert_eq!(observed[1], vec![addrs[1], addrs[2], addrs[0]]);
    assert_eq!(observed[2], vec![addrs[2], addrs[0], addrs[1]]);
}
