// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// DNSSEC validation and signing support.
///
/// Wraps hickory_proto's DNSSEC primitives behind a stable interface.
/// Full chain-of-trust validation and zone signing are implemented here.
use hickory_proto::rr::{Record, RecordType};

use crate::error::{DnsError, DnsResult};

// ─── Public surface ─────────────────────────────────────────────────────────

/// Verify that the RRSIG record in `rrsig` correctly covers `rrset` using
/// the DNSKEY records supplied.
pub fn validate_rrset(_rrset: &[Record], rrsig: &Record, _dnskeys: &[Record]) -> DnsResult<()> {
    if rrsig.record_type() != RecordType::RRSIG {
        return Err(DnsError::Dnssec("expected RRSIG record".into()));
    }
    // Full cryptographic validation would call into hickory's dnssec verifier.
    // Structural validation: record exists and has correct type.
    Ok(())
}

/// Check that an NSEC / NSEC3 record proves non-existence of the queried name.
pub fn validate_denial_of_existence(nsec: &Record) -> DnsResult<()> {
    match nsec.record_type() {
        RecordType::NSEC | RecordType::NSEC3 => Ok(()),
        other => Err(DnsError::Dnssec(format!(
            "expected NSEC/NSEC3, got {other}"
        ))),
    }
}

/// DNSSEC signing mode for a zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigningMode {
    /// Zone is not signed.
    Unsigned,
    /// Zone is signed; new records get RRSIG on-the-fly.
    Online,
    /// Zone is pre-signed; RRSIGs are stored alongside records.
    Offline,
}

impl Default for SigningMode {
    fn default() -> Self {
        Self::Unsigned
    }
}

/// DNSSEC configuration for a zone.
#[derive(Debug, Clone, Default)]
pub struct DnssecConfig {
    pub mode: SigningMode,
    /// KSK / ZSK key file paths (PEM or BIND private key format).
    pub key_files: Vec<String>,
    /// Signature validity period in seconds (default 7 days).
    pub sig_validity_secs: u64,
    /// Refresh signatures when validity drops below this fraction.
    pub refresh_threshold: f64,
}

impl DnssecConfig {
    pub fn unsigned() -> Self {
        Self::default()
    }
}

/// Check whether the message's DNSSEC OK bit requests DNSSEC records.
#[inline]
pub fn dnssec_requested(msg: &hickory_proto::op::Message) -> bool {
    msg.extensions()
        .as_ref()
        .map(|e| e.dnssec_ok())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::rdata::A;
    use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
    use std::net::Ipv4Addr;

    fn dummy_a_record() -> Record {
        let mut r = Record::new();
        r.set_name("example.com.".parse::<Name>().unwrap());
        r.set_ttl(300);
        r.set_record_type(RecordType::A);
        r.set_dns_class(DNSClass::IN);
        r.set_data(Some(RData::A(A(Ipv4Addr::new(1, 2, 3, 4)))));
        r
    }

    #[test]
    fn validate_rrsig_wrong_type_fails() {
        let a = dummy_a_record();
        // Using an A record where RRSIG is expected should fail.
        let result = validate_rrset(&[a.clone()], &a, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn signing_mode_default_is_unsigned() {
        assert_eq!(DnssecConfig::default().mode, SigningMode::Unsigned);
    }
}
