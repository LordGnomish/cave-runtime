// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/kerberos/.../ + MIT krb5 src/lib/krb5/keytab/kt_file.c

//! krb5 keytab file (v0x0502) parser. Format documented in MIT
//! krb5's `kt_file.c` — there's no RFC, but the layout is
//! stable across Heimdal + MIT krb5 + AD's `ktpass.exe` output.
//!
//! ```text
//! keytab {
//!   uint16_t  file_version  (= 0x0502)
//!   keytab_entry entries [ ... ]
//! }
//! keytab_entry {
//!   int32_t   size           // bytes that follow (excluding `size` itself)
//!   uint16_t  num_components // # of name pieces excluding realm
//!   string    realm           // counted_octet_string (uint16_t len + bytes)
//!   string    components[num_components]
//!   uint32_t  name_type      // NT-PRINCIPAL = 1
//!   uint32_t  timestamp      // seconds since Unix epoch
//!   uint8_t   vno8           // key version (legacy, low 8 bits)
//!   key_block {
//!     uint16_t enctype
//!     counted_octet_string contents
//!   }
//!   uint32_t  vno32          // optional — present only when `size`
//!                            // bytes haven't been consumed yet
//! }
//! counted_octet_string := uint16_t length + bytes
//! ```
//!
//! Entries with a *negative* `size` are "holes" (deleted slots);
//! the parser skips them.

use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read};

use super::KerberosError;

/// File signature.
pub const KEYTAB_MAGIC: u16 = 0x0502;
/// Older keytab v0x0501 used little-endian — not supported here
/// (every cave deployment standardises on `v2`).
pub const KEYTAB_V1_MAGIC: u16 = 0x0501;

/// One key entry inside a keytab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeytabEntry {
    pub principal: KrbPrincipal,
    /// Timestamp of when the key was generated (seconds since
    /// 1970-01-01 UTC).
    pub timestamp: u32,
    /// Key version — `vno32` when present, else `vno8`.
    pub vno: u32,
    pub key: KeyBlock,
}

/// Kerberos principal — `name1/name2@REALM`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrbPrincipal {
    pub realm: String,
    pub components: Vec<String>,
    /// `NT-PRINCIPAL = 1`, `NT-SRV-INST = 2`, `NT-SRV-HST = 3` etc.
    pub name_type: u32,
}

impl KrbPrincipal {
    /// `name1/name2@REALM`.
    pub fn to_canonical(&self) -> String {
        format!("{}@{}", self.components.join("/"), self.realm)
    }
}

/// Encrypted key payload — `enctype` is the standard RFC 3961
/// enctype constant (e.g. 18 = `aes256-cts-hmac-sha1-96`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBlock {
    pub enctype: u16,
    pub contents: Vec<u8>,
}

/// Parse a keytab file's bytes into a list of entries.
pub fn parse_keytab(bytes: &[u8]) -> Result<Vec<KeytabEntry>, KerberosError> {
    let mut cur = Cursor::new(bytes);
    let magic = cur
        .read_u16::<BigEndian>()
        .map_err(|e| KerberosError::Keytab(format!("read magic: {e}")))?;
    if magic == KEYTAB_V1_MAGIC {
        return Err(KerberosError::Keytab(
            "keytab v0x0501 (little-endian) not supported — re-emit with v0x0502 (ktutil rkt/write_kt)".into(),
        ));
    }
    if magic != KEYTAB_MAGIC {
        return Err(KerberosError::Keytab(format!(
            "bad keytab magic {magic:#06x} (expected 0x0502)"
        )));
    }
    let mut entries = Vec::new();
    let total_len = bytes.len() as u64;
    while cur.position() < total_len {
        // Each entry begins with a signed int32 `size`.
        let size_signed = cur
            .read_i32::<BigEndian>()
            .map_err(|e| KerberosError::Keytab(format!("read entry size: {e}")))?;
        if size_signed == 0 {
            // The MIT writer sometimes emits trailing zero
            // padding; treat 0 as EOF.
            break;
        }
        if size_signed < 0 {
            // Hole — skip |size| bytes.
            let skip = (-size_signed) as u64;
            let mut throwaway = vec![0u8; skip as usize];
            cur.read_exact(&mut throwaway)
                .map_err(|e| KerberosError::Keytab(format!("skip hole: {e}")))?;
            continue;
        }
        let entry_start = cur.position();
        let entry_end = entry_start + size_signed as u64;
        let entry = parse_entry(&mut cur, size_signed as u64)?;
        // The parser may leave a vno32 unread if it wasn't
        // present — advance to entry_end either way.
        cur.set_position(entry_end);
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_entry(cur: &mut Cursor<&[u8]>, size: u64) -> Result<KeytabEntry, KerberosError> {
    let start = cur.position();
    let num_components = cur
        .read_u16::<BigEndian>()
        .map_err(|e| KerberosError::Keytab(format!("num_components: {e}")))?;
    let realm = read_counted_str(cur)?;
    let mut components = Vec::with_capacity(num_components as usize);
    for _ in 0..num_components {
        components.push(read_counted_str(cur)?);
    }
    let name_type = cur
        .read_u32::<BigEndian>()
        .map_err(|e| KerberosError::Keytab(format!("name_type: {e}")))?;
    let timestamp = cur
        .read_u32::<BigEndian>()
        .map_err(|e| KerberosError::Keytab(format!("timestamp: {e}")))?;
    let vno8 = cur
        .read_u8()
        .map_err(|e| KerberosError::Keytab(format!("vno8: {e}")))?;
    let enctype = cur
        .read_u16::<BigEndian>()
        .map_err(|e| KerberosError::Keytab(format!("enctype: {e}")))?;
    let contents = read_counted_bytes(cur)?;
    // Optional vno32 if there's still room in the entry.
    let mut vno: u32 = vno8 as u32;
    let consumed = cur.position() - start;
    if consumed + 4 <= size {
        let v = cur
            .read_u32::<BigEndian>()
            .map_err(|e| KerberosError::Keytab(format!("vno32: {e}")))?;
        if v != 0 {
            vno = v;
        }
    }
    Ok(KeytabEntry {
        principal: KrbPrincipal {
            realm,
            components,
            name_type,
        },
        timestamp,
        vno,
        key: KeyBlock { enctype, contents },
    })
}

fn read_counted_str(cur: &mut Cursor<&[u8]>) -> Result<String, KerberosError> {
    let bytes = read_counted_bytes(cur)?;
    String::from_utf8(bytes).map_err(|e| KerberosError::Keytab(format!("utf8: {e}")))
}

fn read_counted_bytes(cur: &mut Cursor<&[u8]>) -> Result<Vec<u8>, KerberosError> {
    let len = cur
        .read_u16::<BigEndian>()
        .map_err(|e| KerberosError::Keytab(format!("counted_octet_string length: {e}")))?;
    let mut buf = vec![0u8; len as usize];
    cur.read_exact(&mut buf)
        .map_err(|e| KerberosError::Keytab(format!("counted_octet_string body: {e}")))?;
    Ok(buf)
}

// ── Test helpers ─────────────────────────────────────────────────────────────

/// Encode a single keytab entry — used by the test suite to
/// build synthetic keytab fixtures without shelling out to
/// `ktutil`.
pub fn encode_test_keytab(entries: &[KeytabEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&KEYTAB_MAGIC.to_be_bytes());
    for e in entries {
        let mut body = Vec::new();
        body.extend_from_slice(&(e.principal.components.len() as u16).to_be_bytes());
        write_counted(&mut body, e.principal.realm.as_bytes());
        for c in &e.principal.components {
            write_counted(&mut body, c.as_bytes());
        }
        body.extend_from_slice(&e.principal.name_type.to_be_bytes());
        body.extend_from_slice(&e.timestamp.to_be_bytes());
        body.push((e.vno & 0xff) as u8);
        body.extend_from_slice(&e.key.enctype.to_be_bytes());
        write_counted(&mut body, &e.key.contents);
        body.extend_from_slice(&e.vno.to_be_bytes());
        // entry size := length of body
        out.extend_from_slice(&(body.len() as i32).to_be_bytes());
        out.extend_from_slice(&body);
    }
    out
}

fn write_counted(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> KeytabEntry {
        KeytabEntry {
            principal: KrbPrincipal {
                realm: "EXAMPLE.COM".into(),
                components: vec!["HTTP".into(), "cave-portal.example.com".into()],
                name_type: 1,
            },
            timestamp: 1_700_000_000,
            vno: 7,
            key: KeyBlock {
                enctype: 18, // aes256-cts-hmac-sha1-96
                contents: vec![0xaa; 32],
            },
        }
    }

    #[test]
    fn encode_then_parse_round_trips_single_entry() {
        let original = sample_entry();
        let bytes = encode_test_keytab(&[original.clone()]);
        let parsed = parse_keytab(&bytes).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], original);
    }

    #[test]
    fn principal_canonical_form_uses_slash_and_at() {
        let entry = sample_entry();
        assert_eq!(
            entry.principal.to_canonical(),
            "HTTP/cave-portal.example.com@EXAMPLE.COM"
        );
    }

    #[test]
    fn encode_then_parse_round_trips_multi_entry_keytab() {
        let a = sample_entry();
        let mut b = sample_entry();
        b.principal.components = vec!["host".into(), "kdc.example.com".into()];
        b.vno = 99;
        b.key.enctype = 17; // aes128
        b.key.contents = vec![0xbb; 16];
        let bytes = encode_test_keytab(&[a.clone(), b.clone()]);
        let parsed = parse_keytab(&bytes).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], a);
        assert_eq!(parsed[1], b);
    }

    #[test]
    fn parse_rejects_v1_keytab() {
        let bytes = [0x05, 0x01, 0x00, 0x00];
        let err = parse_keytab(&bytes).unwrap_err();
        assert!(matches!(err, KerberosError::Keytab(_)));
    }

    #[test]
    fn parse_rejects_bad_magic() {
        let bytes = [0xff, 0xff, 0x00, 0x00];
        let err = parse_keytab(&bytes).unwrap_err();
        assert!(matches!(err, KerberosError::Keytab(_)));
    }

    #[test]
    fn parse_returns_empty_for_magic_only_file() {
        let bytes = [0x05, 0x02];
        let parsed = parse_keytab(&bytes).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_skips_deleted_entries() {
        // header + one normal entry + one hole + one normal entry
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&KEYTAB_MAGIC.to_be_bytes());
        let mut entry_bytes = Vec::new();
        // build single normal entry inline to keep test honest.
        let e1 = sample_entry();
        let mut body = Vec::new();
        body.extend_from_slice(&(e1.principal.components.len() as u16).to_be_bytes());
        write_counted(&mut body, e1.principal.realm.as_bytes());
        for c in &e1.principal.components {
            write_counted(&mut body, c.as_bytes());
        }
        body.extend_from_slice(&e1.principal.name_type.to_be_bytes());
        body.extend_from_slice(&e1.timestamp.to_be_bytes());
        body.push((e1.vno & 0xff) as u8);
        body.extend_from_slice(&e1.key.enctype.to_be_bytes());
        write_counted(&mut body, &e1.key.contents);
        body.extend_from_slice(&e1.vno.to_be_bytes());
        entry_bytes.extend_from_slice(&(body.len() as i32).to_be_bytes());
        entry_bytes.extend_from_slice(&body);
        bytes.extend_from_slice(&entry_bytes);
        // hole — size = -10, payload 10 bytes
        bytes.extend_from_slice(&(-10i32).to_be_bytes());
        bytes.extend_from_slice(&[0u8; 10]);
        // another good entry
        bytes.extend_from_slice(&entry_bytes);

        let parsed = parse_keytab(&bytes).unwrap();
        assert_eq!(parsed.len(), 2, "deleted entry must be skipped");
    }

    #[test]
    fn entry_carries_aes256_enctype() {
        let original = sample_entry();
        let bytes = encode_test_keytab(&[original]);
        let parsed = parse_keytab(&bytes).unwrap();
        assert_eq!(parsed[0].key.enctype, 18);
        assert_eq!(parsed[0].key.contents.len(), 32);
    }
}
