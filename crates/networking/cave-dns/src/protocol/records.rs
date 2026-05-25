// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// All DNS record types surfaced for convenience.
///
/// This module re-exports the hickory_proto rdata types used throughout
/// cave-dns so callers only need one import.
pub use hickory_proto::rr::{
    DNSClass, Name, RData, Record, RecordType,
    rdata::{A, AAAA, CNAME, MX, NS, PTR, SOA, SRV, TXT},
};

use crate::error::{DnsError, DnsResult};

/// Build a record with arbitrary RData.
pub fn build_record(name: Name, ttl: u32, class: DNSClass, rdata: RData) -> Record {
    let rtype = rdata.record_type();
    let mut r = Record::new();
    r.set_name(name);
    r.set_ttl(ttl);
    r.set_dns_class(class);
    r.set_record_type(rtype);
    r.set_data(Some(rdata));
    r
}

/// Parse a record type string ("A", "AAAA", "MX", …) case-insensitively.
pub fn parse_record_type(s: &str) -> DnsResult<RecordType> {
    s.parse::<RecordType>()
        .map_err(|_| DnsError::Parse(format!("unknown record type: {s}")))
}

/// Parse a domain name, ensuring it ends with a dot (FQDN).
pub fn parse_fqdn(s: &str) -> DnsResult<Name> {
    let fqdn = if s.ends_with('.') {
        s.to_owned()
    } else {
        format!("{s}.")
    };
    fqdn.parse::<Name>()
        .map_err(|e| DnsError::Parse(e.to_string()))
}

/// Clone records and reduce TTL by `elapsed_secs`, clamping to 0.
pub fn decrement_ttl(records: &[Record], elapsed_secs: u32) -> Vec<Record> {
    records
        .iter()
        .map(|r| {
            let mut cloned = r.clone();
            cloned.set_ttl(r.ttl().saturating_sub(elapsed_secs));
            cloned
        })
        .collect()
}

/// Filter records matching `qtype` (or CNAME, which is always included).
pub fn filter_by_type(records: &[Record], qtype: RecordType) -> Vec<Record> {
    records
        .iter()
        .filter(|r| r.record_type() == qtype || r.record_type() == RecordType::CNAME)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn decrement_ttl_clamps() {
        let addr = Ipv4Addr::new(1, 2, 3, 4);
        let name: Name = "example.com.".parse().unwrap();
        let rec = build_record(name, 10, DNSClass::IN, RData::A(A(addr)));
        let decremented = decrement_ttl(&[rec], 5);
        assert_eq!(decremented[0].ttl(), 5);

        let zeroed = decrement_ttl(&decremented, 100);
        assert_eq!(zeroed[0].ttl(), 0);
    }

    #[test]
    fn parse_record_type_works() {
        assert_eq!(parse_record_type("A").unwrap(), RecordType::A);
        assert_eq!(parse_record_type("AAAA").unwrap(), RecordType::AAAA);
        assert_eq!(parse_record_type("MX").unwrap(), RecordType::MX);
        assert!(parse_record_type("BOGUS").is_err());
    }
}
