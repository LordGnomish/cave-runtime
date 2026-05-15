// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/kerberos (uses the JVM
//   GSS-API; we reimplement the keytab on-disk parser in Rust because
//   the JVM doesn't expose one as public API)
// On-disk format reference: MIT krb5
//   src/lib/krb5/keytab/kt_file.c
//
// MIT keytab format (version 0x0502, big-endian for keytab v2):
//
//   byte  0x05         magic byte 1
//   byte  0x02         keytab version (only 0x01 & 0x02 known)
//   then a stream of entries:
//
//     int32  size       (length of the rest of the entry; if
//                        negative, this slot is a "hole" of |size|
//                        bytes that should be skipped)
//     int16  num_components (excluding realm)
//     counted-string realm
//     counted-string * num_components
//     uint32 name_type
//     uint32 timestamp  (seconds since epoch)
//     uint8  vno8       (low byte of kvno)
//     uint16 enctype
//     counted-string key_bytes
//     uint32 vno32      OPTIONAL — present if size accounts for it

use std::fs;
use std::path::Path;

use super::principal::Principal;

/// One keytab entry — the same shape `klist -k -e` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeytabEntry {
    pub principal: Principal,
    pub timestamp_unix: u32,
    pub kvno: u32,
    pub enctype: u16,
    pub key: Vec<u8>,
    pub name_type: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Keytab {
    pub version: u8,
    pub entries: Vec<KeytabEntry>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeytabError {
    #[error("truncated keytab")]
    Truncated,
    #[error("not a keytab — bad magic 0x{0:02x}")]
    BadMagic(u8),
    #[error("unsupported keytab version 0x{0:02x}")]
    BadVersion(u8),
    #[error("malformed entry: {0}")]
    MalformedEntry(String),
    #[error("io error: {0}")]
    Io(String),
}

/// AES enctype codes (RFC 3962, RFC 8009).  Mirrors MIT
/// `ENCTYPE_*` constants.
pub mod enctype {
    pub const DES_CBC_CRC: u16 = 1;
    pub const DES_CBC_MD5: u16 = 3;
    pub const DES3_CBC_SHA1: u16 = 16;
    pub const AES128_CTS_HMAC_SHA1_96: u16 = 17;
    pub const AES256_CTS_HMAC_SHA1_96: u16 = 18;
    pub const AES128_CTS_HMAC_SHA256_128: u16 = 19;
    pub const AES256_CTS_HMAC_SHA384_192: u16 = 20;
    pub const ARCFOUR_HMAC: u16 = 23;

    pub fn name(code: u16) -> &'static str {
        match code {
            DES_CBC_CRC => "des-cbc-crc",
            DES_CBC_MD5 => "des-cbc-md5",
            DES3_CBC_SHA1 => "des3-cbc-sha1",
            AES128_CTS_HMAC_SHA1_96 => "aes128-cts-hmac-sha1-96",
            AES256_CTS_HMAC_SHA1_96 => "aes256-cts-hmac-sha1-96",
            AES128_CTS_HMAC_SHA256_128 => "aes128-cts-hmac-sha256-128",
            AES256_CTS_HMAC_SHA384_192 => "aes256-cts-hmac-sha384-192",
            ARCFOUR_HMAC => "arcfour-hmac",
            _ => "unknown",
        }
    }
}

impl Keytab {
    /// Parse from a file on disk.
    pub fn from_path(path: &Path) -> Result<Self, KeytabError> {
        let bytes = fs::read(path).map_err(|e| KeytabError::Io(e.to_string()))?;
        Self::parse(&bytes)
    }

    /// Parse from an in-memory byte buffer.
    pub fn parse(input: &[u8]) -> Result<Self, KeytabError> {
        if input.len() < 2 {
            return Err(KeytabError::Truncated);
        }
        if input[0] != 0x05 {
            return Err(KeytabError::BadMagic(input[0]));
        }
        let version = input[1];
        if version != 0x01 && version != 0x02 {
            return Err(KeytabError::BadVersion(version));
        }

        let mut p = Parser { buf: input, pos: 2, le: version == 0x01 };
        let mut entries = Vec::new();
        while !p.eof() {
            let size_i32 = p.read_i32()?;
            if size_i32 == 0 {
                // Some implementations end on a zero record.
                break;
            }
            if size_i32 < 0 {
                p.advance(size_i32.unsigned_abs() as usize)?;
                continue;
            }
            let end = p.pos.checked_add(size_i32 as usize).ok_or_else(|| KeytabError::MalformedEntry("size overflow".into()))?;
            if end > input.len() {
                return Err(KeytabError::Truncated);
            }
            let body_start = p.pos;
            // num_components stored as int16; in v1 it INCLUDES realm,
            // in v2 it EXCLUDES realm.
            let n_raw = p.read_u16()?;
            let n_components = if version == 0x01 { n_raw.saturating_sub(1) } else { n_raw } as usize;
            let realm_bytes = p.read_counted()?;
            let realm = String::from_utf8(realm_bytes.to_vec()).map_err(|_| KeytabError::MalformedEntry("non-utf8 realm".into()))?;
            let mut comps = Vec::with_capacity(n_components);
            for _ in 0..n_components {
                let c = p.read_counted()?;
                comps.push(String::from_utf8(c.to_vec()).map_err(|_| KeytabError::MalformedEntry("non-utf8 component".into()))?);
            }
            let name_type = if version == 0x01 { 0 } else { p.read_u32()? };
            let timestamp = p.read_u32()?;
            let vno8 = p.read_u8()?;
            let enctype = p.read_u16()?;
            let key = p.read_counted()?.to_vec();
            let vno32 = if end - p.pos >= 4 {
                p.read_u32()?
            } else {
                vno8 as u32
            };
            // Skip any trailing bytes inside the entry.
            p.pos = end;

            let _ = body_start; // (retained for clarity)
            entries.push(KeytabEntry {
                principal: Principal { components: comps, realm },
                timestamp_unix: timestamp,
                kvno: vno32,
                enctype,
                key,
                name_type,
            });
        }
        Ok(Keytab { version, entries })
    }

    /// Reverse of `parse` — write the v2 (network byte order) shape.
    /// Used by fixtures + the portal "Generate template keytab"
    /// helper.
    pub fn encode_v2(&self) -> Vec<u8> {
        let mut out = vec![0x05, 0x02];
        for e in &self.entries {
            let mut body = Vec::new();
            // num_components (excluding realm).
            body.extend_from_slice(&(e.principal.components.len() as u16).to_be_bytes());
            // counted-string realm
            write_counted(&mut body, e.principal.realm.as_bytes());
            for c in &e.principal.components {
                write_counted(&mut body, c.as_bytes());
            }
            body.extend_from_slice(&e.name_type.to_be_bytes());
            body.extend_from_slice(&e.timestamp_unix.to_be_bytes());
            body.push((e.kvno & 0xff) as u8);
            body.extend_from_slice(&e.enctype.to_be_bytes());
            write_counted(&mut body, &e.key);
            body.extend_from_slice(&e.kvno.to_be_bytes());

            out.extend_from_slice(&(body.len() as i32).to_be_bytes());
            out.extend_from_slice(&body);
        }
        out
    }

    /// Find an entry by principal + enctype.  Used by SPNEGO
    /// verification to pick the right session key.
    pub fn find(&self, principal: &Principal, enctype: u16) -> Option<&KeytabEntry> {
        self.entries.iter().find(|e| e.principal == *principal && e.enctype == enctype)
    }

    /// Group entries by principal for the portal UI.
    pub fn by_principal(&self) -> Vec<(Principal, Vec<&KeytabEntry>)> {
        let mut out: Vec<(Principal, Vec<&KeytabEntry>)> = Vec::new();
        for e in &self.entries {
            if let Some(slot) = out.iter_mut().find(|(p, _)| p == &e.principal) {
                slot.1.push(e);
            } else {
                out.push((e.principal.clone(), vec![e]));
            }
        }
        out
    }
}

fn write_counted(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
    out.extend_from_slice(b);
}

struct Parser<'a> {
    buf: &'a [u8],
    pos: usize,
    /// keytab v1 uses native byte order; v2 uses big-endian.  We
    /// detect via the version byte and assume hosts are
    /// little-endian for v1.
    le: bool,
}

impl<'a> Parser<'a> {
    fn eof(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn read_u8(&mut self) -> Result<u8, KeytabError> {
        let b = *self.buf.get(self.pos).ok_or(KeytabError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, KeytabError> {
        if self.pos + 2 > self.buf.len() {
            return Err(KeytabError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + 2];
        self.pos += 2;
        Ok(if self.le {
            u16::from_le_bytes([s[0], s[1]])
        } else {
            u16::from_be_bytes([s[0], s[1]])
        })
    }

    fn read_u32(&mut self) -> Result<u32, KeytabError> {
        if self.pos + 4 > self.buf.len() {
            return Err(KeytabError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + 4];
        self.pos += 4;
        Ok(if self.le {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        } else {
            u32::from_be_bytes([s[0], s[1], s[2], s[3]])
        })
    }

    fn read_i32(&mut self) -> Result<i32, KeytabError> {
        Ok(self.read_u32()? as i32)
    }

    fn read_counted(&mut self) -> Result<&'a [u8], KeytabError> {
        let n = self.read_u16()? as usize;
        if self.pos + n > self.buf.len() {
            return Err(KeytabError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn advance(&mut self, n: usize) -> Result<(), KeytabError> {
        if self.pos + n > self.buf.len() {
            return Err(KeytabError::Truncated);
        }
        self.pos += n;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> KeytabEntry {
        KeytabEntry {
            principal: Principal {
                components: vec!["HTTP".into(), "portal.acme.corp".into()],
                realm: "ACME.CORP".into(),
            },
            timestamp_unix: 1_700_000_000,
            kvno: 7,
            enctype: enctype::AES256_CTS_HMAC_SHA1_96,
            key: vec![0u8; 32],
            name_type: 1,
        }
    }

    #[test]
    fn round_trip_v2_keytab() {
        let kt = Keytab {
            version: 2,
            entries: vec![sample_entry()],
        };
        let bytes = kt.encode_v2();
        assert_eq!(&bytes[..2], &[0x05, 0x02]);
        let parsed = Keytab::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0], sample_entry());
    }

    #[test]
    fn parse_rejects_bad_magic() {
        let bytes = vec![0x06, 0x02];
        assert!(matches!(Keytab::parse(&bytes), Err(KeytabError::BadMagic(0x06))));
    }

    #[test]
    fn parse_rejects_bad_version() {
        let bytes = vec![0x05, 0x09];
        assert!(matches!(Keytab::parse(&bytes), Err(KeytabError::BadVersion(0x09))));
    }

    #[test]
    fn parse_truncated_returns_truncated() {
        // size says 100 bytes but only 4 follow.
        let bytes = vec![0x05, 0x02, 0x00, 0x00, 0x00, 0x64, 0xff, 0xff, 0xff, 0xff];
        assert!(matches!(Keytab::parse(&bytes), Err(KeytabError::Truncated)));
    }

    #[test]
    fn parse_skips_hole_records() {
        // Build: header + hole(-12, 12 padding bytes) + one real entry.
        let real = Keytab { version: 2, entries: vec![sample_entry()] };
        let mut bytes = real.encode_v2();
        // Insert a hole right after the header (offset 2).
        let mut prefix = bytes.drain(..2).collect::<Vec<u8>>();
        // size = -12, then 12 bytes of garbage.
        prefix.extend_from_slice(&(-12i32).to_be_bytes());
        prefix.extend_from_slice(&[0u8; 12]);
        prefix.extend_from_slice(&bytes);
        let parsed = Keytab::parse(&prefix).unwrap();
        assert_eq!(parsed.entries.len(), 1);
    }

    #[test]
    fn enctype_name_known_values() {
        assert_eq!(enctype::name(enctype::AES256_CTS_HMAC_SHA1_96), "aes256-cts-hmac-sha1-96");
        assert_eq!(enctype::name(0xffff), "unknown");
    }

    #[test]
    fn find_picks_matching_principal_and_enctype() {
        let p = Principal {
            components: vec!["HTTP".into(), "portal.acme.corp".into()],
            realm: "ACME.CORP".into(),
        };
        let kt = Keytab { version: 2, entries: vec![sample_entry()] };
        assert!(kt.find(&p, enctype::AES256_CTS_HMAC_SHA1_96).is_some());
        assert!(kt.find(&p, enctype::AES128_CTS_HMAC_SHA1_96).is_none());
    }

    #[test]
    fn by_principal_groups_entries() {
        let mut e1 = sample_entry();
        e1.enctype = enctype::AES128_CTS_HMAC_SHA1_96;
        let mut e2 = sample_entry();
        e2.enctype = enctype::AES256_CTS_HMAC_SHA1_96;
        let kt = Keytab { version: 2, entries: vec![e1, e2] };
        let grouped = kt.by_principal();
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].1.len(), 2);
    }
}
