use crate::error::{DnsError, DnsResult};
use crate::types::*;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Encode a DNS message to wire format.
pub fn encode(msg: &DnsMessage) -> DnsResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(512);

    // Header
    let id_bytes = msg.header.id.to_be_bytes();
    buf.push(id_bytes[0]);
    buf.push(id_bytes[1]);

    // Flags word 1:
    // QR(1) | OPCODE(4) | AA(1) | TC(1) | RD(1)
    let flags1: u8 = ((msg.header.qr as u8) << 7)
        | ((msg.header.opcode & 0x0F) << 3)
        | ((msg.header.aa as u8) << 2)
        | ((msg.header.tc as u8) << 1)
        | (msg.header.rd as u8);

    // Flags word 2:
    // RA(1) | Z(3) | RCODE(4)
    let flags2: u8 = ((msg.header.ra as u8) << 7)
        | ((msg.header.z & 0x07) << 4)
        | (msg.header.rcode & 0x0F);

    buf.push(flags1);
    buf.push(flags2);

    // Counts
    let qdcount = msg.questions.len() as u16;
    let ancount = msg.answers.len() as u16;
    let nscount = msg.authority.len() as u16;
    let arcount = msg.additional.len() as u16;

    buf.extend_from_slice(&qdcount.to_be_bytes());
    buf.extend_from_slice(&ancount.to_be_bytes());
    buf.extend_from_slice(&nscount.to_be_bytes());
    buf.extend_from_slice(&arcount.to_be_bytes());

    // Questions
    for q in &msg.questions {
        encode_name(&q.name, &mut buf);
        buf.extend_from_slice(&q.qtype.to_u16().to_be_bytes());
        buf.extend_from_slice(&q.qclass.to_be_bytes());
    }

    // Answer / Authority / Additional RRs
    for rr in msg
        .answers
        .iter()
        .chain(msg.authority.iter())
        .chain(msg.additional.iter())
    {
        encode_name(&rr.name, &mut buf);
        buf.extend_from_slice(&rr.rtype.to_u16().to_be_bytes());
        buf.extend_from_slice(&rr.class.to_be_bytes());
        buf.extend_from_slice(&rr.ttl.to_be_bytes());

        // Encode rdata into a temporary buffer, then write rdlength
        let mut rdata_buf = Vec::new();
        encode_rdata(&rr.rdata, &mut rdata_buf);
        let rdlength = rdata_buf.len() as u16;
        buf.extend_from_slice(&rdlength.to_be_bytes());
        buf.extend_from_slice(&rdata_buf);
    }

    Ok(buf)
}

/// Decode a DNS message from wire format.
pub fn decode(data: &[u8]) -> DnsResult<DnsMessage> {
    if data.len() < 12 {
        return Err(DnsError::FormatError);
    }

    let mut pos = 0usize;

    let id = read_u16(data, &mut pos)?;
    let flags1 = read_u8(data, &mut pos)?;
    let flags2 = read_u8(data, &mut pos)?;

    let qr = (flags1 & 0x80) != 0;
    let opcode = (flags1 >> 3) & 0x0F;
    let aa = (flags1 & 0x04) != 0;
    let tc = (flags1 & 0x02) != 0;
    let rd = (flags1 & 0x01) != 0;
    let ra = (flags2 & 0x80) != 0;
    let z = (flags2 >> 4) & 0x07;
    let rcode = flags2 & 0x0F;

    let qdcount = read_u16(data, &mut pos)?;
    let ancount = read_u16(data, &mut pos)?;
    let nscount = read_u16(data, &mut pos)?;
    let arcount = read_u16(data, &mut pos)?;

    let header = Header { id, qr, opcode, aa, tc, rd, ra, z, rcode };

    let mut questions = Vec::with_capacity(qdcount as usize);
    for _ in 0..qdcount {
        let name = decode_name(data, &mut pos)?;
        let qtype_val = read_u16(data, &mut pos)?;
        let qclass = read_u16(data, &mut pos)?;
        questions.push(Question {
            name,
            qtype: RecordType::from_u16(qtype_val),
            qclass,
        });
    }

    let mut answers = Vec::with_capacity(ancount as usize);
    for _ in 0..ancount {
        answers.push(decode_rr(data, &mut pos)?);
    }

    let mut authority = Vec::with_capacity(nscount as usize);
    for _ in 0..nscount {
        authority.push(decode_rr(data, &mut pos)?);
    }

    let mut additional = Vec::with_capacity(arcount as usize);
    for _ in 0..arcount {
        additional.push(decode_rr(data, &mut pos)?);
    }

    Ok(DnsMessage { header, questions, answers, authority, additional })
}

fn decode_rr(data: &[u8], pos: &mut usize) -> DnsResult<ResourceRecord> {
    let name = decode_name(data, pos)?;
    let rtype_val = read_u16(data, pos)?;
    let class = read_u16(data, pos)?;
    let ttl = read_u32(data, pos)?;
    let rdlength = read_u16(data, pos)? as usize;

    let rtype = RecordType::from_u16(rtype_val);
    let rdata = decode_rdata(&rtype, data, pos, rdlength)?;

    Ok(ResourceRecord { name, rtype, class, ttl, rdata })
}

/// Encode a domain name as DNS labels: len + bytes, terminated by 0x00.
fn encode_name(name: &str, buf: &mut Vec<u8>) {
    // Strip trailing dot for processing
    let name = name.trim_end_matches('.');
    if name.is_empty() {
        buf.push(0); // root
        return;
    }
    for label in name.split('.') {
        let bytes = label.as_bytes();
        buf.push(bytes.len() as u8);
        buf.extend_from_slice(bytes);
    }
    buf.push(0); // root terminator
}

/// Decode a domain name from wire format, following compression pointers.
fn decode_name(data: &[u8], pos: &mut usize) -> DnsResult<String> {
    let mut labels = Vec::new();
    let mut jump_limit = 10usize; // prevent infinite loops

    loop {
        if *pos >= data.len() {
            return Err(DnsError::FormatError);
        }
        let len_byte = data[*pos];

        if len_byte == 0 {
            // End of name
            *pos += 1;
            break;
        } else if (len_byte & 0xC0) == 0xC0 {
            // Compression pointer
            if *pos + 1 >= data.len() {
                return Err(DnsError::FormatError);
            }
            let offset = (((len_byte & 0x3F) as usize) << 8) | (data[*pos + 1] as usize);
            *pos += 2;
            jump_limit -= 1;
            if jump_limit == 0 {
                return Err(DnsError::ParseError("compression loop".to_string()));
            }
            // Follow the pointer: we need a separate position
            let mut ptr_pos = offset;
            // Recurse via iterative approach using ptr_pos
            loop {
                if ptr_pos >= data.len() {
                    return Err(DnsError::FormatError);
                }
                let plen = data[ptr_pos];
                if plen == 0 {
                    break;
                } else if (plen & 0xC0) == 0xC0 {
                    if ptr_pos + 1 >= data.len() {
                        return Err(DnsError::FormatError);
                    }
                    let new_offset = (((plen & 0x3F) as usize) << 8) | (data[ptr_pos + 1] as usize);
                    ptr_pos = new_offset;
                } else {
                    let label_len = plen as usize;
                    if ptr_pos + 1 + label_len > data.len() {
                        return Err(DnsError::FormatError);
                    }
                    let label = std::str::from_utf8(&data[ptr_pos + 1..ptr_pos + 1 + label_len])
                        .map_err(|_| DnsError::FormatError)?;
                    labels.push(label.to_string());
                    ptr_pos += 1 + label_len;
                }
            }
            break;
        } else {
            // Normal label
            let label_len = len_byte as usize;
            if *pos + 1 + label_len > data.len() {
                return Err(DnsError::FormatError);
            }
            let label = std::str::from_utf8(&data[*pos + 1..*pos + 1 + label_len])
                .map_err(|_| DnsError::FormatError)?;
            labels.push(label.to_string());
            *pos += 1 + label_len;
        }
    }

    if labels.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(format!("{}.", labels.join(".")))
    }
}

fn encode_rdata(rdata: &RData, buf: &mut Vec<u8>) {
    match rdata {
        RData::A(ip) => {
            buf.extend_from_slice(&ip.octets());
        }
        RData::AAAA(ip) => {
            buf.extend_from_slice(&ip.octets());
        }
        RData::CNAME(name) | RData::NS(name) | RData::PTR(name) => {
            encode_name(name, buf);
        }
        RData::MX { priority, exchange } => {
            buf.extend_from_slice(&priority.to_be_bytes());
            encode_name(exchange, buf);
        }
        RData::TXT(strings) => {
            for s in strings {
                // Each string: 1 byte length + bytes
                let chunk_len = s.len().min(255) as u8;
                buf.push(chunk_len);
                buf.extend_from_slice(&s[..chunk_len as usize]);
            }
        }
        RData::SRV { priority, weight, port, target } => {
            buf.extend_from_slice(&priority.to_be_bytes());
            buf.extend_from_slice(&weight.to_be_bytes());
            buf.extend_from_slice(&port.to_be_bytes());
            encode_name(target, buf);
        }
        RData::SOA { mname, rname, serial, refresh, retry, expire, minimum } => {
            encode_name(mname, buf);
            encode_name(rname, buf);
            buf.extend_from_slice(&serial.to_be_bytes());
            buf.extend_from_slice(&refresh.to_be_bytes());
            buf.extend_from_slice(&retry.to_be_bytes());
            buf.extend_from_slice(&expire.to_be_bytes());
            buf.extend_from_slice(&minimum.to_be_bytes());
        }
        RData::CAA { flags, tag, value } => {
            buf.push(*flags);
            let tag_bytes = tag.as_bytes();
            buf.push(tag_bytes.len() as u8);
            buf.extend_from_slice(tag_bytes);
            buf.extend_from_slice(value.as_bytes());
        }
        RData::Raw(bytes) => {
            buf.extend_from_slice(bytes);
        }
    }
}

fn decode_rdata(rtype: &RecordType, data: &[u8], pos: &mut usize, rdlen: usize) -> DnsResult<RData> {
    let start = *pos;
    let end = start + rdlen;
    if end > data.len() {
        return Err(DnsError::FormatError);
    }

    let rdata = match rtype {
        RecordType::A => {
            if rdlen != 4 {
                return Err(DnsError::FormatError);
            }
            let ip = Ipv4Addr::new(data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]);
            *pos += 4;
            RData::A(ip)
        }
        RecordType::AAAA => {
            if rdlen != 16 {
                return Err(DnsError::FormatError);
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&data[*pos..*pos + 16]);
            *pos += 16;
            RData::AAAA(Ipv6Addr::from(octets))
        }
        RecordType::CNAME => {
            let name = decode_name(data, pos)?;
            RData::CNAME(name)
        }
        RecordType::NS => {
            let name = decode_name(data, pos)?;
            RData::NS(name)
        }
        RecordType::PTR => {
            let name = decode_name(data, pos)?;
            RData::PTR(name)
        }
        RecordType::MX => {
            let priority = read_u16(data, pos)?;
            let exchange = decode_name(data, pos)?;
            RData::MX { priority, exchange }
        }
        RecordType::TXT => {
            let mut strings = Vec::new();
            let mut consumed = 0usize;
            while consumed < rdlen {
                if *pos >= data.len() {
                    return Err(DnsError::FormatError);
                }
                let slen = data[*pos] as usize;
                *pos += 1;
                consumed += 1;
                if consumed + slen > rdlen {
                    return Err(DnsError::FormatError);
                }
                if *pos + slen > data.len() {
                    return Err(DnsError::FormatError);
                }
                strings.push(data[*pos..*pos + slen].to_vec());
                *pos += slen;
                consumed += slen;
            }
            RData::TXT(strings)
        }
        RecordType::SRV => {
            let priority = read_u16(data, pos)?;
            let weight = read_u16(data, pos)?;
            let port = read_u16(data, pos)?;
            let target = decode_name(data, pos)?;
            RData::SRV { priority, weight, port, target }
        }
        RecordType::SOA => {
            let mname = decode_name(data, pos)?;
            let rname = decode_name(data, pos)?;
            let serial = read_u32(data, pos)?;
            let refresh = read_u32(data, pos)?;
            let retry = read_u32(data, pos)?;
            let expire = read_u32(data, pos)?;
            let minimum = read_u32(data, pos)?;
            RData::SOA { mname, rname, serial, refresh, retry, expire, minimum }
        }
        RecordType::CAA => {
            if rdlen < 2 {
                return Err(DnsError::FormatError);
            }
            let flags = data[*pos];
            let tag_len = data[*pos + 1] as usize;
            *pos += 2;
            if *pos + tag_len > data.len() || 2 + tag_len > rdlen {
                return Err(DnsError::FormatError);
            }
            let tag = std::str::from_utf8(&data[*pos..*pos + tag_len])
                .map_err(|_| DnsError::FormatError)?
                .to_string();
            *pos += tag_len;
            let value_len = rdlen - 2 - tag_len;
            if *pos + value_len > data.len() {
                return Err(DnsError::FormatError);
            }
            let value = std::str::from_utf8(&data[*pos..*pos + value_len])
                .map_err(|_| DnsError::FormatError)?
                .to_string();
            *pos += value_len;
            RData::CAA { flags, tag, value }
        }
        RecordType::Unknown(_) => {
            let raw = data[start..end].to_vec();
            *pos = end;
            RData::Raw(raw)
        }
    };

    Ok(rdata)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn read_u8(data: &[u8], pos: &mut usize) -> DnsResult<u8> {
    if *pos >= data.len() {
        return Err(DnsError::FormatError);
    }
    let v = data[*pos];
    *pos += 1;
    Ok(v)
}

fn read_u16(data: &[u8], pos: &mut usize) -> DnsResult<u16> {
    if *pos + 2 > data.len() {
        return Err(DnsError::FormatError);
    }
    let v = u16::from_be_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    Ok(v)
}

fn read_u32(data: &[u8], pos: &mut usize) -> DnsResult<u32> {
    if *pos + 4 > data.len() {
        return Err(DnsError::FormatError);
    }
    let v = u32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    Ok(v)
}
