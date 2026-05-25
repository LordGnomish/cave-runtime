// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::path::Path;

use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};

use crate::{
    config::ZoneType,
    error::{DnsError, DnsResult},
    zone::Zone,
};

/// Load a zone from an RFC 1035 master file using a simple line-by-line parser.
///
/// Supports: $ORIGIN, $TTL directives and standard RR lines.
pub fn load_zone_file(path: &Path, origin: &Name) -> DnsResult<Zone> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| DnsError::Zone(format!("cannot read zone file {}: {}", path.display(), e)))?;

    parse_zone_content(&content, origin)
}

/// Parse zone file content from a string (also used in tests).
pub fn parse_zone_content(content: &str, origin: &Name) -> DnsResult<Zone> {
    let mut current_origin = origin.clone();
    let mut current_ttl: u32 = 3600;
    let mut soa_record: Option<Record> = None;
    let mut records: Vec<Record> = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and blank lines
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        // Strip inline comments
        let line = match line.find(';') {
            Some(pos) => line[..pos].trim(),
            None => line,
        };
        if line.is_empty() {
            continue;
        }

        // $ORIGIN directive
        if let Some(rest) = line.strip_prefix("$ORIGIN") {
            let name_str = rest.trim();
            current_origin = name_str
                .parse()
                .map_err(|e: hickory_proto::error::ProtoError| DnsError::Parse(e.to_string()))?;
            continue;
        }

        // $TTL directive
        if let Some(rest) = line.strip_prefix("$TTL") {
            if let Ok(ttl) = rest.trim().parse::<u32>() {
                current_ttl = ttl;
            }
            continue;
        }

        // $INCLUDE — skip for now
        if line.starts_with("$INCLUDE") {
            continue;
        }

        // Resource record line: [name] [ttl] [class] type rdata
        if let Ok(rec) = parse_rr_line(line, &current_origin, current_ttl) {
            if rec.record_type() == RecordType::SOA && soa_record.is_none() {
                soa_record = Some(rec.clone());
            }
            records.push(rec);
        }
    }

    let soa =
        soa_record.ok_or_else(|| DnsError::Zone("no SOA record found in zone file".into()))?;

    let mut zone = Zone::new(current_origin, soa, ZoneType::Primary);
    for r in records {
        if r.record_type() != RecordType::SOA {
            zone.add_record(r);
        }
    }
    Ok(zone)
}

/// Best-effort single RR line parser.
fn parse_rr_line(line: &str, origin: &Name, default_ttl: u32) -> DnsResult<Record> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(DnsError::Parse(format!("too few fields: {line}")));
    }

    let mut idx = 0;

    // Name field — may be @ (origin), relative, or absolute
    let name: Name = {
        let s = parts[idx];
        idx += 1;
        if s == "@" {
            origin.clone()
        } else if s.ends_with('.') {
            s.parse()
                .map_err(|e: hickory_proto::error::ProtoError| DnsError::Parse(e.to_string()))?
        } else {
            format!("{s}.{origin}")
                .parse()
                .map_err(|e: hickory_proto::error::ProtoError| DnsError::Parse(e.to_string()))?
        }
    };

    // Optional TTL
    let ttl = if let Ok(t) = parts[idx].parse::<u32>() {
        idx += 1;
        t
    } else {
        default_ttl
    };

    // Optional class (IN / CH / etc.)
    let class = if ["IN", "CH", "HS", "ANY"].contains(&parts[idx].to_uppercase().as_str()) {
        idx += 1;
        DNSClass::IN // treat all as IN for simplicity
    } else {
        DNSClass::IN
    };

    if idx >= parts.len() {
        return Err(DnsError::Parse("missing record type".into()));
    }

    let rtype_str = parts[idx].to_uppercase();
    idx += 1;

    let rtype: RecordType = rtype_str
        .parse()
        .map_err(|_| DnsError::Parse(format!("unknown type: {rtype_str}")))?;

    let rdata_str = parts[idx..].join(" ");
    let rdata = parse_rdata(rtype, &rdata_str, origin)?;

    let mut r = Record::new();
    r.set_name(name);
    r.set_ttl(ttl);
    r.set_record_type(rtype);
    r.set_dns_class(class);
    r.set_data(Some(rdata));
    Ok(r)
}

fn parse_rdata(rtype: RecordType, s: &str, origin: &Name) -> DnsResult<RData> {
    let s = s.trim();
    match rtype {
        RecordType::A => {
            let addr: std::net::Ipv4Addr = s
                .parse()
                .map_err(|e: std::net::AddrParseError| DnsError::Parse(e.to_string()))?;
            Ok(RData::A(hickory_proto::rr::rdata::A(addr)))
        }
        RecordType::AAAA => {
            let addr: std::net::Ipv6Addr = s
                .parse()
                .map_err(|e: std::net::AddrParseError| DnsError::Parse(e.to_string()))?;
            Ok(RData::AAAA(hickory_proto::rr::rdata::AAAA(addr)))
        }
        RecordType::CNAME => {
            let n = resolve_name(s, origin)?;
            Ok(RData::CNAME(hickory_proto::rr::rdata::CNAME(n)))
        }
        RecordType::NS => {
            let n = resolve_name(s, origin)?;
            Ok(RData::NS(hickory_proto::rr::rdata::NS(n)))
        }
        RecordType::PTR => {
            let n = resolve_name(s, origin)?;
            Ok(RData::PTR(hickory_proto::rr::rdata::PTR(n)))
        }
        RecordType::MX => {
            let mut parts = s.splitn(2, ' ');
            let pref: u16 = parts
                .next()
                .and_then(|p| p.parse().ok())
                .ok_or_else(|| DnsError::Parse(format!("MX priority missing: {s}")))?;
            let exch = resolve_name(parts.next().unwrap_or("").trim(), origin)?;
            Ok(RData::MX(hickory_proto::rr::rdata::MX::new(pref, exch)))
        }
        RecordType::TXT => {
            // Strip surrounding quotes if present
            let text = s.trim_matches('"').to_string();
            Ok(RData::TXT(hickory_proto::rr::rdata::TXT::new(vec![text])))
        }
        RecordType::SOA => {
            // mname rname serial refresh retry expire minimum
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() < 7 {
                return Err(DnsError::Parse(format!("SOA too few fields: {s}")));
            }
            let mname = resolve_name(parts[0], origin)?;
            let rname = resolve_name(parts[1], origin)?;
            let serial: u32 = parts[2]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            let refresh: i32 = parts[3]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            let retry: i32 = parts[4]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            let expire: i32 = parts[5]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            let minimum: u32 = parts[6]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            Ok(RData::SOA(hickory_proto::rr::rdata::SOA::new(
                mname, rname, serial, refresh, retry, expire, minimum,
            )))
        }
        RecordType::SRV => {
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() < 4 {
                return Err(DnsError::Parse(format!("SRV too few fields: {s}")));
            }
            let priority: u16 = parts[0]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            let weight: u16 = parts[1]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            let port: u16 = parts[2]
                .parse()
                .map_err(|e: std::num::ParseIntError| DnsError::Parse(e.to_string()))?;
            let target = resolve_name(parts[3], origin)?;
            Ok(RData::SRV(hickory_proto::rr::rdata::SRV::new(
                priority, weight, port, target,
            )))
        }
        _ => {
            // For unsupported types, skip
            Err(DnsError::Parse(format!(
                "unsupported type in zone file: {rtype}"
            )))
        }
    }
}

fn resolve_name(s: &str, origin: &Name) -> DnsResult<Name> {
    if s == "@" {
        Ok(origin.clone())
    } else if s.ends_with('.') {
        s.parse()
            .map_err(|e: hickory_proto::error::ProtoError| DnsError::Parse(e.to_string()))
    } else {
        format!("{s}.{origin}")
            .parse()
            .map_err(|e: hickory_proto::error::ProtoError| DnsError::Parse(e.to_string()))
    }
}

/// Save a zone to an RFC 1035 master file.
pub fn save_zone_file(zone: &Zone, path: &Path) -> DnsResult<()> {
    use std::fmt::Write as FmtWrite;
    let mut out = String::new();

    writeln!(out, "; Zone: {}", zone.origin).ok();
    writeln!(out, "; Generated by cave-dns").ok();
    writeln!(out).ok();
    writeln!(out, "$ORIGIN {}", zone.origin).ok();
    writeln!(out, "$TTL 300").ok();
    writeln!(out).ok();

    let soa_str = record_to_zone_format(&zone.soa);
    writeln!(out, "{soa_str}").ok();
    writeln!(out).ok();

    for r in zone.all_records() {
        if r.record_type() != RecordType::SOA {
            writeln!(out, "{}", record_to_zone_format(&r)).ok();
        }
    }

    std::fs::write(path, out).map_err(|e| DnsError::Zone(format!("cannot write zone file: {e}")))
}

fn record_to_zone_format(r: &Record) -> String {
    format!(
        "{} {} {} {} {}",
        r.name(),
        r.ttl(),
        r.dns_class(),
        r.record_type(),
        r.data().map(|d| d.to_string()).unwrap_or_default()
    )
}

/// Create a minimal SOA record for a newly-created empty zone.
pub fn make_default_soa(origin: &Name) -> Record {
    let soa_data = hickory_proto::rr::rdata::SOA::new(
        format!("ns1.{}", origin)
            .parse()
            .unwrap_or_else(|_| origin.clone()),
        format!("hostmaster.{}", origin)
            .parse()
            .unwrap_or_else(|_| origin.clone()),
        1,
        3600,
        900,
        604800,
        300,
    );
    let mut r = Record::new();
    r.set_name(origin.clone());
    r.set_ttl(300);
    r.set_record_type(RecordType::SOA);
    r.set_dns_class(DNSClass::IN);
    r.set_data(Some(RData::SOA(soa_data)));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_default_soa_has_correct_type() {
        let origin: Name = "example.com.".parse().unwrap();
        let soa = make_default_soa(&origin);
        assert_eq!(soa.record_type(), RecordType::SOA);
        assert_eq!(soa.name(), &origin);
    }

    #[test]
    fn parse_simple_zone_content() {
        let content = r#"
$ORIGIN example.com.
$TTL 300
@ IN SOA ns1 hostmaster 2024010100 3600 900 604800 300
@ IN NS ns1
ns1 IN A 1.2.3.4
www IN A 5.6.7.8
"#;
        let origin: Name = "example.com.".parse().unwrap();
        let zone = parse_zone_content(content, &origin).unwrap();
        assert_eq!(zone.serial(), 2024010100);

        let a = zone.lookup(&"www.example.com.".parse().unwrap(), RecordType::A);
        assert_eq!(a.len(), 1);
    }
}
