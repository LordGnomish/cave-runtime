// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Zone transfer — AXFR (full) and IXFR (incremental), RFC 5936 / RFC 1995.
use std::net::SocketAddr;

use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{DNSClass, Name, RecordType},
    serialize::binary::{BinDecodable, BinEncodable, BinEncoder},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info};

use crate::{
    error::{DnsError, DnsResult},
    protocol::message::{encode_message, parse_message},
    zone::Zone,
};

/// Send a full zone (AXFR) over an already-connected TCP stream.
pub async fn send_axfr(zone: &Zone, stream: &mut TcpStream) -> DnsResult<()> {
    let records = zone.axfr_records();
    info!(zone = %zone.origin, records = records.len(), "sending AXFR");

    for record in &records {
        let mut msg = Message::new();
        msg.set_message_type(MessageType::Response);
        msg.set_op_code(OpCode::Query);
        msg.set_response_code(ResponseCode::NoError);
        msg.set_authoritative(true);
        msg.add_answer(record.clone());

        let bytes = encode_message(&msg)?;
        let len = bytes.len() as u16;
        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(&bytes).await?;
    }
    debug!(zone = %zone.origin, "AXFR complete");
    Ok(())
}

/// Receive a full zone (AXFR) from a master over a TCP stream.
pub async fn receive_axfr(stream: &mut TcpStream, origin: &Name) -> DnsResult<Zone> {
    use crate::{config::ZoneType, zone::file::make_default_soa};

    let mut zone: Option<Zone> = None;
    let mut first_soa = true;
    let mut done = false;

    while !done {
        // TCP DNS: 2-byte length prefix
        let mut len_buf = [0u8; 2];
        stream.read_exact(&mut len_buf).await.map_err(|e| {
            DnsError::Transfer(format!("reading length prefix: {e}"))
        })?;
        let len = u16::from_be_bytes(len_buf) as usize;

        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await.map_err(|e| {
            DnsError::Transfer(format!("reading DNS message: {e}"))
        })?;

        let msg = parse_message(&buf)?;

        for record in msg.answers() {
            if record.record_type() == RecordType::SOA {
                if first_soa {
                    // Begin zone
                    zone = Some(Zone::new(
                        origin.clone(),
                        record.clone(),
                        ZoneType::Secondary,
                    ));
                    first_soa = false;
                } else {
                    // Second SOA = end of AXFR
                    done = true;
                    break;
                }
            } else if let Some(z) = zone.as_mut() {
                z.add_record(record.clone());
            }
        }
    }

    zone.ok_or_else(|| DnsError::Transfer("AXFR produced no zone".into()))
}

/// Check whether a zone needs refresh by comparing SOA serials with the master.
pub async fn check_serial(
    zone: &Zone,
    master: SocketAddr,
) -> DnsResult<Option<u32>> {
    let mut stream = TcpStream::connect(master).await.map_err(|e| {
        DnsError::Transfer(format!("connecting to master {master}: {e}"))
    })?;

    // Send SOA query
    let mut query = Message::new();
    query.set_message_type(MessageType::Query);
    query.set_op_code(OpCode::Query);
    query.set_recursion_desired(false);
    let mut q = Query::new();
    q.set_name(zone.origin.clone());
    q.set_query_type(RecordType::SOA);
    q.set_query_class(DNSClass::IN);
    query.add_query(q);

    let bytes = encode_message(&query)?;
    let len = bytes.len() as u16;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&bytes).await?;

    // Read response
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let len = u16::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;

    let resp = parse_message(&buf)?;
    for record in resp.answers() {
        if record.record_type() == RecordType::SOA {
            if let Some(hickory_proto::rr::RData::SOA(soa)) = record.data() {
                let master_serial = soa.serial();
                if master_serial != zone.serial() {
                    return Ok(Some(master_serial));
                }
            }
        }
    }
    Ok(None)
}
