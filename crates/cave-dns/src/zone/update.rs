/// RFC 2136 Dynamic DNS update processing.
use hickory_proto::{
    op::{Message, MessageType, OpCode, ResponseCode},
    rr::{DNSClass, Name, RData, Record, RecordType},
};

use crate::{
    error::{DnsError, DnsResult},
    zone::Zone,
};

/// Apply a DNS UPDATE message to a zone, returning the update response.
///
/// Implements RFC 2136 Section 3: prerequisites → updates → success/failure.
pub async fn apply_update(zone: &mut Zone, update_msg: &Message) -> DnsResult<Message> {
    if update_msg.op_code() != OpCode::Update {
        return Err(DnsError::Update("not an UPDATE message".into()));
    }

    let mut resp = Message::new();
    resp.set_id(update_msg.id());
    resp.set_message_type(MessageType::Response);
    resp.set_op_code(OpCode::Update);

    // Zone section: first QUESTION entry names the zone
    let zone_name = match update_msg.queries().first() {
        Some(q) => q.name().clone(),
        None => {
            resp.set_response_code(ResponseCode::FormErr);
            return Ok(resp);
        }
    };

    if &zone.origin != &zone_name {
        resp.set_response_code(ResponseCode::NotAuth);
        return Ok(resp);
    }

    // 1. Check prerequisites (ANSWER section in UPDATE)
    for prereq in update_msg.answers() {
        if let Err(e) = check_prerequisite(zone, prereq) {
            resp.set_response_code(ResponseCode::NXRRSet);
            return Ok(resp);
        }
    }

    // 2. Apply updates (AUTHORITY section)
    for update_rr in update_msg.name_servers() {
        apply_update_rr(zone, update_rr)?;
    }

    zone.bump_serial();
    resp.set_response_code(ResponseCode::NoError);
    Ok(resp)
}

/// Evaluate a single prerequisite record (RFC 2136 Section 2.4).
fn check_prerequisite(zone: &Zone, prereq: &Record) -> DnsResult<()> {
    let name = prereq.name();
    let rtype = prereq.record_type();

    match prereq.dns_class() {
        // ANY class: RRset must exist
        DNSClass::ANY => {
            let records = zone.lookup(name, rtype);
            if records.is_empty() {
                return Err(DnsError::Update(format!(
                    "prerequisite failed: {name} {rtype} must exist"
                )));
            }
        }
        // NONE class: RRset must NOT exist
        DNSClass::NONE => {
            let records = zone.lookup(name, rtype);
            if !records.is_empty() {
                return Err(DnsError::Update(format!(
                    "prerequisite failed: {name} {rtype} must not exist"
                )));
            }
        }
        // IN class: specific RR must exist
        DNSClass::IN => {
            let records = zone.lookup(name, rtype);
            if !records.iter().any(|r| r.data() == prereq.data()) {
                return Err(DnsError::Update(format!(
                    "prerequisite failed: exact RR for {name} {rtype} not found"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Apply one update record (RFC 2136 Section 2.5).
fn apply_update_rr(zone: &mut Zone, rr: &Record) -> DnsResult<()> {
    match rr.dns_class() {
        // Add RR (class IN)
        DNSClass::IN => {
            zone.add_record(rr.clone());
        }
        // Delete all RRs for name+type (class ANY, no rdata)
        DNSClass::ANY => {
            if rr.record_type() == RecordType::ANY {
                zone.remove_name(rr.name());
            } else {
                zone.remove_record(rr.name(), rr.record_type(), None);
            }
        }
        // Delete specific RR (class NONE, with rdata)
        DNSClass::NONE => {
            zone.remove_record(rr.name(), rr.record_type(), rr.data());
        }
        _ => {
            return Err(DnsError::Update(format!(
                "unknown class {:?} in update section",
                rr.dns_class()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::rdata::A;
    use std::net::Ipv4Addr;

    fn a_record(name: &str, addr: Ipv4Addr) -> Record {
        let mut r = Record::new();
        r.set_name(name.parse().unwrap());
        r.set_ttl(300);
        r.set_record_type(RecordType::A);
        r.set_dns_class(DNSClass::IN);
        r.set_data(Some(RData::A(A(addr))));
        r
    }

    #[tokio::test]
    async fn add_record_via_update() {
        use crate::zone::file::make_default_soa;
        use crate::config::ZoneType;

        let origin: Name = "example.com.".parse().unwrap();
        let soa = make_default_soa(&origin);
        let mut zone = Zone::new(origin.clone(), soa, ZoneType::Primary);

        let mut update = Message::new();
        update.set_id(1);
        update.set_message_type(MessageType::Query);
        update.set_op_code(OpCode::Update);

        // Zone section
        let mut q = hickory_proto::op::Query::new();
        q.set_name(origin.clone());
        q.set_query_type(RecordType::SOA);
        q.set_query_class(DNSClass::IN);
        update.add_query(q);

        // Update: add A record
        update.add_name_server(a_record("www.example.com.", Ipv4Addr::new(1, 2, 3, 4)));

        let resp = apply_update(&mut zone, &update).await.unwrap();
        assert_eq!(resp.response_code(), ResponseCode::NoError);

        let found = zone.lookup(&"www.example.com.".parse().unwrap(), RecordType::A);
        assert_eq!(found.len(), 1);
    }
}
