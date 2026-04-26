use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

pub const CLASS_IN: u16 = 1;

pub const RCODE_OK: u8 = 0;
pub const RCODE_FORMAT: u8 = 1;
pub const RCODE_SERVER: u8 = 2;
pub const RCODE_NXDOMAIN: u8 = 3;
pub const RCODE_NOTIMP: u8 = 4;
pub const RCODE_REFUSED: u8 = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RecordType {
    A,
    AAAA,
    CNAME,
    MX,
    TXT,
    SRV,
    NS,
    SOA,
    PTR,
    CAA,
    Unknown(u16),
}

impl RecordType {
    pub fn to_u16(&self) -> u16 {
        match self {
            RecordType::A => 1,
            RecordType::NS => 2,
            RecordType::CNAME => 5,
            RecordType::SOA => 6,
            RecordType::PTR => 12,
            RecordType::MX => 15,
            RecordType::TXT => 16,
            RecordType::AAAA => 28,
            RecordType::SRV => 33,
            RecordType::CAA => 257,
            RecordType::Unknown(v) => *v,
        }
    }

    pub fn from_u16(v: u16) -> Self {
        match v {
            1 => RecordType::A,
            2 => RecordType::NS,
            5 => RecordType::CNAME,
            6 => RecordType::SOA,
            12 => RecordType::PTR,
            15 => RecordType::MX,
            16 => RecordType::TXT,
            28 => RecordType::AAAA,
            33 => RecordType::SRV,
            257 => RecordType::CAA,
            other => RecordType::Unknown(other),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "A" => RecordType::A,
            "AAAA" => RecordType::AAAA,
            "CNAME" => RecordType::CNAME,
            "MX" => RecordType::MX,
            "TXT" => RecordType::TXT,
            "SRV" => RecordType::SRV,
            "NS" => RecordType::NS,
            "SOA" => RecordType::SOA,
            "PTR" => RecordType::PTR,
            "CAA" => RecordType::CAA,
            _ => RecordType::Unknown(0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceRecord {
    pub name: String,
    pub rtype: RecordType,
    pub class: u16,
    pub ttl: u32,
    pub rdata: RData,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RData {
    A(Ipv4Addr),
    AAAA(Ipv6Addr),
    CNAME(String),
    MX { priority: u16, exchange: String },
    TXT(Vec<Vec<u8>>),
    SRV { priority: u16, weight: u16, port: u16, target: String },
    NS(String),
    SOA {
        mname: String,
        rname: String,
        serial: u32,
        refresh: u32,
        retry: u32,
        expire: u32,
        minimum: u32,
    },
    PTR(String),
    CAA { flags: u8, tag: String, value: String },
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct DnsMessage {
    pub header: Header,
    pub questions: Vec<Question>,
    pub answers: Vec<ResourceRecord>,
    pub authority: Vec<ResourceRecord>,
    pub additional: Vec<ResourceRecord>,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub id: u16,
    pub qr: bool,
    pub opcode: u8,
    pub aa: bool,
    pub tc: bool,
    pub rd: bool,
    pub ra: bool,
    pub z: u8,
    pub rcode: u8,
}

#[derive(Debug, Clone)]
pub struct Question {
    pub name: String,
    pub qtype: RecordType,
    pub qclass: u16,
}
