// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for the pulp_rpm RPM v3 binary header reader.
//!
//! Reference: RPM Reference Manual ch. 5 (Package format) — Lead +
//! Signature Header + Header. Each header is `\x8E\xAD\xE8\x01` + 3-byte
//! reserved + u32 number_of_entries (BE) + u32 store_size (BE), followed
//! by `n × 16-byte IndexEntry` (tag, type, offset, count, all BE u32) and
//! then `store_size` bytes of value store.

use cave_artifacts::pulp::plugins::rpm::{
    parse_rpm_header, parse_rpm_lead, RpmHeader, RpmIndexEntry, RpmLead,
};

/// Construct a synthetic RPM lead. 96 bytes:
/// magic(4) + major(1) + minor(1) + type(2 BE) + archnum(2 BE) +
/// name(66) + osnum(2 BE) + signature_type(2 BE) + reserved(16).
fn make_lead(rpm_type: u16, name: &str) -> Vec<u8> {
    let mut lead = Vec::with_capacity(96);
    lead.extend_from_slice(&[0xED, 0xAB, 0xEE, 0xDB]); // magic
    lead.push(3); // major
    lead.push(0); // minor
    lead.extend_from_slice(&rpm_type.to_be_bytes());
    lead.extend_from_slice(&1u16.to_be_bytes()); // archnum = 1 (x86)
    let mut name_buf = [0u8; 66];
    for (i, b) in name.bytes().take(66).enumerate() {
        name_buf[i] = b;
    }
    lead.extend_from_slice(&name_buf);
    lead.extend_from_slice(&1u16.to_be_bytes()); // osnum
    lead.extend_from_slice(&5u16.to_be_bytes()); // signature_type RPMSIGTYPE_HEADERSIG = 5
    lead.extend_from_slice(&[0u8; 16]);
    assert_eq!(lead.len(), 96);
    lead
}

/// Construct a single-entry RPM header carrying tag=1000 (NAME) value="bash".
/// IndexEntry: tag=1000 u32 BE, type=6 (STRING) u32 BE, offset=0 u32 BE,
/// count=1 u32 BE. Store: "bash\0".
fn make_header_one_string_entry(tag: u32, value: &str) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(&[0x8E, 0xAD, 0xE8, 0x01]); // header magic
    h.extend_from_slice(&[0, 0, 0]); // reserved
    h.push(1); // version byte (per spec, reserved-ish; some readers tolerate)
    h.extend_from_slice(&1u32.to_be_bytes()); // n_entries = 1
    let store_bytes = {
        let mut s = value.as_bytes().to_vec();
        s.push(0);
        s
    };
    h.extend_from_slice(&(store_bytes.len() as u32).to_be_bytes()); // store size
    // Index entry (16 bytes).
    h.extend_from_slice(&tag.to_be_bytes());
    h.extend_from_slice(&6u32.to_be_bytes()); // type = RPM_STRING_TYPE
    h.extend_from_slice(&0u32.to_be_bytes()); // offset = 0
    h.extend_from_slice(&1u32.to_be_bytes()); // count = 1
    h.extend_from_slice(&store_bytes);
    h
}

#[test]
fn parse_lead_basic() {
    let bytes = make_lead(0, "bash-5.1.8-6.el9.x86_64");
    let lead: RpmLead = parse_rpm_lead(&bytes).unwrap();
    assert_eq!(lead.major, 3);
    assert_eq!(lead.minor, 0);
    assert_eq!(lead.rpm_type, 0); // 0 = binary, 1 = source
    assert!(lead.name.starts_with("bash-5.1.8"));
}

#[test]
fn parse_lead_rejects_bad_magic() {
    let mut bytes = make_lead(0, "x");
    bytes[0] = 0;
    assert!(parse_rpm_lead(&bytes).is_err());
}

#[test]
fn parse_header_single_string_tag_name() {
    let h_bytes = make_header_one_string_entry(1000, "bash");
    let header: RpmHeader = parse_rpm_header(&h_bytes).unwrap();
    assert_eq!(header.entries.len(), 1);
    let entry: &RpmIndexEntry = &header.entries[0];
    assert_eq!(entry.tag, 1000);
    assert_eq!(entry.type_id, 6);
    assert_eq!(entry.count, 1);
    // Convenience accessor on RpmHeader: get tag value as string.
    assert_eq!(header.string_tag(1000).as_deref(), Some("bash"));
}

#[test]
fn parse_header_rejects_bad_magic() {
    let mut h = make_header_one_string_entry(1000, "bash");
    h[0] = 0;
    assert!(parse_rpm_header(&h).is_err());
}

#[test]
fn parse_header_rejects_truncated() {
    let h = make_header_one_string_entry(1000, "bash");
    assert!(parse_rpm_header(&h[..12]).is_err());
}
