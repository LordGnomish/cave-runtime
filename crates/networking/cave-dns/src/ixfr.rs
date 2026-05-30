// SPDX-License-Identifier: AGPL-3.0-or-later
//! IXFR — incremental zone transfer generation (RFC 1995).
//!
//! Closes the incremental half of the `transfer` feature: computes the diff
//! between two zone versions and renders RFC 1995 §4 format:
//! `SOA(M), SOA(N), <dels>, SOA(M), <adds>, SOA(M)`.

use crate::{DnsError, DnsResult};
use hickory_proto::rr::{RData, Record, RecordType};

/// Extract the SERIAL from an SOA record; `None` for non-SOA / empty records.
#[must_use]
pub fn soa_serial(rr: &Record) -> Option<u32> {
    match rr.data() {
        Some(RData::SOA(soa)) => Some(soa.serial()),
        _ => None,
    }
}

/// A computed incremental difference between two zone versions.
#[derive(Debug, Clone, PartialEq)]
pub struct IxfrDelta {
    /// SOA of the base (older) version.
    pub old_soa: Record,
    /// SOA of the current (newer) version.
    pub new_soa: Record,
    /// SERIAL of the base version.
    pub old_serial: u32,
    /// SERIAL of the current version.
    pub new_serial: u32,
    /// Records removed (excluding SOAs).
    pub deletions: Vec<Record>,
    /// Records added (excluding SOAs).
    pub additions: Vec<Record>,
}

impl IxfrDelta {
    /// Compute the delta; errors if either snapshot lacks an SOA.
    pub fn compute(old: &[Record], new: &[Record]) -> DnsResult<Self> {
        let old_soa = find_soa(old)?;
        let new_soa = find_soa(new)?;
        let old_serial = soa_serial(&old_soa)
            .ok_or_else(|| DnsError::Transfer("old zone SOA has no serial".into()))?;
        let new_serial = soa_serial(&new_soa)
            .ok_or_else(|| DnsError::Transfer("new zone SOA has no serial".into()))?;

        let old_body: Vec<&Record> = old.iter().filter(|r| r.record_type() != RecordType::SOA).collect();
        let new_body: Vec<&Record> = new.iter().filter(|r| r.record_type() != RecordType::SOA).collect();

        let deletions = old_body.iter().filter(|r| !new_body.contains(r)).map(|r| (*r).clone()).collect();
        let additions = new_body.iter().filter(|r| !old_body.contains(r)).map(|r| (*r).clone()).collect();

        Ok(Self { old_soa, new_soa, old_serial, new_serial, deletions, additions })
    }

    /// Whether to fall back to a full AXFR (base serial newer than current).
    #[must_use]
    pub fn needs_axfr_fallback(&self) -> bool {
        self.old_serial > self.new_serial
    }

    /// Render the delta as an RFC 1995 §4 record sequence (single SOA if empty).
    #[must_use]
    pub fn to_wire(&self) -> Vec<Record> {
        if self.old_serial == self.new_serial || self.needs_axfr_fallback() {
            return vec![self.new_soa.clone()];
        }
        let mut out = Vec::with_capacity(4 + self.deletions.len() + self.additions.len());
        out.push(self.new_soa.clone());
        out.push(self.old_soa.clone());
        out.extend(self.deletions.iter().cloned());
        out.push(self.new_soa.clone());
        out.extend(self.additions.iter().cloned());
        out.push(self.new_soa.clone());
        out
    }
}

fn find_soa(records: &[Record]) -> DnsResult<Record> {
    records
        .iter()
        .find(|r| r.record_type() == RecordType::SOA)
        .cloned()
        .ok_or_else(|| DnsError::Transfer("zone snapshot has no SOA record".into()))
}
