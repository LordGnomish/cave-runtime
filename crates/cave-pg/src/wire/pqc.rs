//! Post-quantum transport KEM negotiation envelope.
//!
//! cave-pg layers an out-of-band KEM negotiation on top of the standard
//! `SSLRequest` handshake so that PQC-ready clients can advertise an
//! ML-KEM-768 (NIST FIPS 203 Kyber-768) capsule before the TLS hello fires.
//!
//! The negotiation is a 4-byte sentinel + 1-byte version + N-byte mechanism
//! list (cstring sequence terminated by an empty cstring), wrapped in a
//! single CopyData-style frame so middleboxes that don't speak PQC fall back
//! to plain TLS.
//!
//! ```text
//! ┌────────┬────────┬─────────────────────────────────┐
//! │ "PQCK" │ ver:1  │ mech_1\0 mech_2\0 ... \0        │
//! └────────┴────────┴─────────────────────────────────┘
//! ```
//!
//! ML-KEM-768 parameters (FIPS 203):
//!
//! | name              | bytes |
//! |-------------------|-------|
//! | encapsulation key | 1184  |
//! | decapsulation key | 2400  |
//! | ciphertext        | 1088  |
//! | shared secret     | 32    |

use bytes::{Buf, BufMut, BytesMut};

use super::codec::{ProtocolError, ProtocolResult, put_cstr, read_cstr};

pub const SENTINEL: &[u8; 4] = b"PQCK";
pub const VERSION_1: u8 = 1;

pub const MLKEM768_NAME: &str = "ML-KEM-768";
pub const MLKEM768_EK_BYTES: usize = 1184;
pub const MLKEM768_DK_BYTES: usize = 2400;
pub const MLKEM768_CT_BYTES: usize = 1088;
pub const MLKEM768_SS_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PqcOffer {
    pub version: u8,
    pub mechanisms: Vec<String>,
}

impl PqcOffer {
    pub fn ml_kem_768_only() -> Self {
        Self {
            version: VERSION_1,
            mechanisms: vec![MLKEM768_NAME.to_string()],
        }
    }

    pub fn supports(&self, mech: &str) -> bool {
        self.mechanisms.iter().any(|m| m == mech)
    }
}

pub fn encode_offer(offer: &PqcOffer, buf: &mut BytesMut) {
    buf.put_slice(SENTINEL);
    buf.put_u8(offer.version);
    for m in &offer.mechanisms {
        put_cstr(buf, m);
    }
    buf.put_u8(0);
}

pub fn decode_offer(buf: &mut BytesMut) -> ProtocolResult<PqcOffer> {
    if buf.len() < 5 {
        return Err(ProtocolError::Incomplete {
            needed: 5 - buf.len(),
        });
    }
    if &buf[..4] != SENTINEL {
        return Err(ProtocolError::InvalidValue {
            field: "pqc.sentinel",
            reason: "mismatch".into(),
        });
    }
    buf.advance(4);
    let version = buf.get_u8();
    if version == 0 || version > VERSION_1 {
        return Err(ProtocolError::InvalidValue {
            field: "pqc.version",
            reason: format!("{}", version),
        });
    }
    let mut mechanisms = Vec::new();
    while !buf.is_empty() && buf[0] != 0 {
        mechanisms.push(read_cstr(buf)?);
    }
    if !buf.is_empty() && buf[0] == 0 {
        buf.advance(1);
    }
    Ok(PqcOffer {
        version,
        mechanisms,
    })
}

/// Negotiate the strongest mechanism the two peers have in common, ordered
/// by **server preference**. Returns `None` if no overlap exists, in which
/// case callers should drop to standard TLS.
pub fn negotiate(server_supported: &[&str], client_offer: &PqcOffer) -> Option<String> {
    server_supported
        .iter()
        .find(|m| client_offer.supports(m))
        .map(|s| s.to_string())
}

/// Validate ML-KEM-768 byte sizes per FIPS 203 §7.
pub fn validate_mlkem768_ek(ek: &[u8]) -> ProtocolResult<()> {
    if ek.len() != MLKEM768_EK_BYTES {
        return Err(ProtocolError::InvalidValue {
            field: "mlkem768.ek",
            reason: format!("{} != {}", ek.len(), MLKEM768_EK_BYTES),
        });
    }
    Ok(())
}

pub fn validate_mlkem768_ct(ct: &[u8]) -> ProtocolResult<()> {
    if ct.len() != MLKEM768_CT_BYTES {
        return Err(ProtocolError::InvalidValue {
            field: "mlkem768.ct",
            reason: format!("{} != {}", ct.len(), MLKEM768_CT_BYTES),
        });
    }
    Ok(())
}

pub fn validate_mlkem768_ss(ss: &[u8]) -> ProtocolResult<()> {
    if ss.len() != MLKEM768_SS_BYTES {
        return Err(ProtocolError::InvalidValue {
            field: "mlkem768.ss",
            reason: format!("{} != {}", ss.len(), MLKEM768_SS_BYTES),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_is_four_ascii_bytes() {
        assert_eq!(SENTINEL.len(), 4);
        assert_eq!(SENTINEL, b"PQCK");
    }

    #[test]
    fn mlkem768_constants_match_fips203() {
        // FIPS 203 §8 table 4: ML-KEM-768.
        assert_eq!(MLKEM768_EK_BYTES, 1184);
        assert_eq!(MLKEM768_DK_BYTES, 2400);
        assert_eq!(MLKEM768_CT_BYTES, 1088);
        assert_eq!(MLKEM768_SS_BYTES, 32);
    }

    #[test]
    fn mlkem768_only_offer_round_trip() {
        let offer = PqcOffer::ml_kem_768_only();
        let mut buf = BytesMut::new();
        encode_offer(&offer, &mut buf);
        let decoded = decode_offer(&mut buf).unwrap();
        assert_eq!(decoded, offer);
    }

    #[test]
    fn multi_mechanism_offer_round_trip() {
        let offer = PqcOffer {
            version: VERSION_1,
            mechanisms: vec![
                MLKEM768_NAME.to_string(),
                "ML-KEM-1024".to_string(),
                "X25519MLKEM768".to_string(),
            ],
        };
        let mut buf = BytesMut::new();
        encode_offer(&offer, &mut buf);
        let decoded = decode_offer(&mut buf).unwrap();
        assert_eq!(decoded, offer);
    }

    #[test]
    fn supports_check_works() {
        let offer = PqcOffer::ml_kem_768_only();
        assert!(offer.supports(MLKEM768_NAME));
        assert!(!offer.supports("ML-KEM-512"));
    }

    #[test]
    fn negotiate_picks_server_preferred() {
        let offer = PqcOffer {
            version: VERSION_1,
            mechanisms: vec!["A".into(), "B".into(), "C".into()],
        };
        let chosen = negotiate(&["X", "B", "A"], &offer);
        assert_eq!(chosen.as_deref(), Some("B"));
    }

    #[test]
    fn negotiate_no_overlap_returns_none() {
        let offer = PqcOffer {
            version: VERSION_1,
            mechanisms: vec!["X".into()],
        };
        assert!(negotiate(&["Y", "Z"], &offer).is_none());
    }

    #[test]
    fn decode_wrong_sentinel_errors() {
        let mut buf = BytesMut::from(&b"NOPE\x01\x00"[..]);
        let err = decode_offer(&mut buf).unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => assert_eq!(field, "pqc.sentinel"),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn decode_unsupported_version_errors() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(SENTINEL);
        buf.put_u8(99);
        buf.put_u8(0);
        let err = decode_offer(&mut buf).unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => assert_eq!(field, "pqc.version"),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn decode_zero_version_errors() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(SENTINEL);
        buf.put_u8(0);
        buf.put_u8(0);
        let err = decode_offer(&mut buf).unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => assert_eq!(field, "pqc.version"),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn validate_mlkem768_ek_correct_size() {
        let ek = vec![0u8; MLKEM768_EK_BYTES];
        assert!(validate_mlkem768_ek(&ek).is_ok());
    }

    #[test]
    fn validate_mlkem768_ek_wrong_size_errors() {
        let ek = vec![0u8; 100];
        assert!(validate_mlkem768_ek(&ek).is_err());
    }

    #[test]
    fn validate_mlkem768_ct_correct_size() {
        let ct = vec![0u8; MLKEM768_CT_BYTES];
        assert!(validate_mlkem768_ct(&ct).is_ok());
    }

    #[test]
    fn validate_mlkem768_ct_wrong_size_errors() {
        assert!(validate_mlkem768_ct(&[0u8; 1024]).is_err());
    }

    #[test]
    fn validate_mlkem768_ss_correct_size() {
        let ss = vec![0u8; MLKEM768_SS_BYTES];
        assert!(validate_mlkem768_ss(&ss).is_ok());
    }

    #[test]
    fn validate_mlkem768_ss_wrong_size_errors() {
        assert!(validate_mlkem768_ss(&[0u8; 16]).is_err());
    }

    #[test]
    fn truncated_offer_errors() {
        let mut buf = BytesMut::from(&b"PQ"[..]);
        let err = decode_offer(&mut buf).unwrap_err();
        match err {
            ProtocolError::Incomplete { .. } => {}
            other => panic!("expected Incomplete, got {:?}", other),
        }
    }
}
