// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{DNSClass, Name, RData, Record, RecordType},
    serialize::binary::{BinDecodable, BinEncodable, BinEncoder},
};
use std::str::FromStr;

use crate::error::{DnsError, DnsResult};

/// Parse a DNS message from raw bytes (UDP datagram or after 2-byte TCP length).
pub fn parse_message(buf: &[u8]) -> DnsResult<Message> {
    Message::from_vec(buf).map_err(DnsError::from)
}

/// Encode a DNS message to bytes.
pub fn encode_message(msg: &Message) -> DnsResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(512);
    let mut encoder = BinEncoder::new(&mut buf);
    msg.emit(&mut encoder)?;
    Ok(buf)
}

/// Create a response skeleton mirroring the query header.
pub fn make_response(query: &Message) -> Message {
    let mut resp = Message::new();
    resp.set_id(query.id());
    resp.set_message_type(MessageType::Response);
    resp.set_op_code(query.op_code());
    resp.set_recursion_desired(query.recursion_desired());
    resp.set_recursion_available(true);
    resp.set_response_code(ResponseCode::NoError);

    for q in query.queries() {
        resp.add_query(q.clone());
    }
    resp
}

/// Create an error response with the given RCODE.
pub fn make_error_response(query: &Message, rcode: ResponseCode) -> Message {
    let mut resp = make_response(query);
    resp.set_response_code(rcode);
    resp
}

/// Truncate message to fit within `max_bytes`, setting the TC bit.
pub fn truncate_to_udp(msg: &mut Message, max_bytes: usize) {
    // Encode, check length; if over budget, clear answers and set TC.
    if let Ok(encoded) = encode_message(msg) {
        if encoded.len() > max_bytes {
            msg.set_truncated(true);
            msg.take_answers();
            msg.take_additionals();
            msg.take_name_servers();
        }
    }
}

/// Extract the EDNS0 advertised payload size from a request (defaults to 512).
pub fn edns_payload_size(msg: &Message) -> u16 {
    msg.extensions()
        .as_ref()
        .map(|e| e.max_payload())
        .unwrap_or(512)
}

/// Return true when the DNSSEC OK bit is set in the request.
pub fn dnssec_ok(msg: &Message) -> bool {
    msg.extensions()
        .as_ref()
        .map(|e| e.dnssec_ok())
        .unwrap_or(false)
}

/// Build a simple A record.
pub fn a_record(name: &str, ttl: u32, addr: std::net::Ipv4Addr) -> DnsResult<Record> {
    let n = Name::from_str(name).map_err(|e| DnsError::Parse(e.to_string()))?;
    let mut r = Record::new();
    r.set_name(n);
    r.set_ttl(ttl);
    r.set_record_type(RecordType::A);
    r.set_dns_class(DNSClass::IN);
    r.set_data(Some(RData::A(hickory_proto::rr::rdata::A(addr))));
    Ok(r)
}

/// Build a simple AAAA record.
pub fn aaaa_record(name: &str, ttl: u32, addr: std::net::Ipv6Addr) -> DnsResult<Record> {
    let n = Name::from_str(name).map_err(|e| DnsError::Parse(e.to_string()))?;
    let mut r = Record::new();
    r.set_name(n);
    r.set_ttl(ttl);
    r.set_record_type(RecordType::AAAA);
    r.set_dns_class(DNSClass::IN);
    r.set_data(Some(RData::AAAA(hickory_proto::rr::rdata::AAAA(addr))));
    Ok(r)
}

/// Build a TXT record.
pub fn txt_record(name: &str, ttl: u32, texts: Vec<String>) -> DnsResult<Record> {
    let n = Name::from_str(name).map_err(|e| DnsError::Parse(e.to_string()))?;
    let mut r = Record::new();
    r.set_name(n);
    r.set_ttl(ttl);
    r.set_record_type(RecordType::TXT);
    r.set_dns_class(DNSClass::IN);
    r.set_data(Some(RData::TXT(hickory_proto::rr::rdata::TXT::new(texts))));
    Ok(r)
}

/// Build a query (useful for health checks / forwarder probes).
pub fn make_query(name: &str, qtype: RecordType) -> DnsResult<Message> {
    let n = Name::from_str(name).map_err(|e| DnsError::Parse(e.to_string()))?;
    let mut q = Query::new();
    q.set_name(n);
    q.set_query_type(qtype);
    q.set_query_class(DNSClass::IN);

    let mut msg = Message::new();
    msg.set_id(rand_id());
    msg.set_message_type(MessageType::Query);
    msg.set_op_code(OpCode::Query);
    msg.set_recursion_desired(true);
    msg.add_query(q);
    Ok(msg)
}

fn rand_id() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (ns & 0xffff) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_empty_message() {
        let mut msg = Message::new();
        msg.set_id(42);
        msg.set_message_type(MessageType::Query);
        msg.set_op_code(OpCode::Query);
        msg.set_recursion_desired(true);

        let encoded = encode_message(&msg).unwrap();
        let decoded = parse_message(&encoded).unwrap();
        assert_eq!(decoded.id(), 42);
        assert_eq!(decoded.message_type(), MessageType::Query);
    }

    #[test]
    fn make_response_copies_id_and_queries() {
        let mut query = Message::new();
        query.set_id(1234);
        query.set_message_type(MessageType::Query);

        let mut q = Query::new();
        q.set_name(Name::from_str("example.com.").unwrap());
        q.set_query_type(RecordType::A);
        q.set_query_class(DNSClass::IN);
        query.add_query(q);

        let resp = make_response(&query);
        assert_eq!(resp.id(), 1234);
        assert_eq!(resp.message_type(), MessageType::Response);
        assert_eq!(resp.queries().len(), 1);
    }

    #[test]
    fn a_record_encodes() {
        let r = a_record("foo.example.com.", 300, "1.2.3.4".parse().unwrap()).unwrap();
        assert_eq!(r.record_type(), RecordType::A);
        assert_eq!(r.ttl(), 300);
    }
}
