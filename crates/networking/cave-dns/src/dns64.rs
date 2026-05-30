// SPDX-License-Identifier: AGPL-3.0-or-later
//! `dns64` plugin — synthesize AAAA records from A records (RFC 6147).
//!
//! Line-by-line port of CoreDNS v1.14.3 `plugin/dns64`, embedding the IPv4
//! address into a NAT64 prefix per RFC 6052 §2.2. Default prefix `64:ff9b::/96`.

use crate::{DnsError, DnsResult};
use hickory_proto::rr::rdata::AAAA;
use hickory_proto::rr::{RData, Record};
use std::net::{Ipv4Addr, Ipv6Addr};

const U_OCTET: usize = 8;
const VALID_PREFIX_LENS: [u8; 6] = [32, 40, 48, 56, 64, 96];

/// A configured DNS64 synthesizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dns64 {
    prefix: Ipv6Addr,
    prefix_len: u8,
}

impl Dns64 {
    /// The well-known prefix configuration `64:ff9b::/96`.
    #[must_use]
    pub fn well_known() -> Self {
        Self { prefix: Ipv6Addr::new(0x0064, 0xff9b, 0, 0, 0, 0, 0, 0), prefix_len: 96 }
    }

    /// Build a synthesizer, rejecting any length not in `{32,40,48,56,64,96}`.
    pub fn new(prefix: Ipv6Addr, prefix_len: u8) -> DnsResult<Self> {
        if !VALID_PREFIX_LENS.contains(&prefix_len) {
            return Err(DnsError::Config(format!(
                "dns64 prefix length {prefix_len} must be one of 32/40/48/56/64/96"
            )));
        }
        Ok(Self { prefix, prefix_len })
    }

    /// Embed an IPv4 address into the prefix per RFC 6052 §2.2.
    #[must_use]
    pub fn synthesize(&self, v4: Ipv4Addr) -> Ipv6Addr {
        let mut buf = [0u8; 16];
        let pbytes = self.prefix.octets();
        let plen_bytes = (self.prefix_len / 8) as usize;
        buf[..plen_bytes].copy_from_slice(&pbytes[..plen_bytes]);
        let v = v4.octets();
        let mut out = plen_bytes;
        let mut k = 0;
        while k < 4 {
            if out == U_OCTET {
                out += 1;
                continue;
            }
            buf[out] = v[k];
            out += 1;
            k += 1;
        }
        Ipv6Addr::from(buf)
    }

    /// Synthesize AAAA records for every A record in `answers`.
    #[must_use]
    pub fn synthesize_records(&self, answers: &[Record]) -> Vec<Record> {
        answers
            .iter()
            .filter_map(|rec| match rec.data() {
                Some(RData::A(a)) => Some((rec, a.0)),
                _ => None,
            })
            .map(|(rec, v4)| {
                Record::from_rdata(rec.name().clone(), rec.ttl(), RData::AAAA(AAAA(self.synthesize(v4))))
            })
            .collect()
    }

    /// RFC 6147 trigger: synthesize only when no AAAA was returned.
    #[must_use]
    pub fn should_synthesize(&self, response_has_aaaa: bool) -> bool {
        !response_has_aaaa
    }
}
