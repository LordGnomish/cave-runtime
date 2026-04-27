//! PQC transport KEM negotiation envelope for cave-cache.
//!
//! The negotiation is identical to `cave-pg::wire::pqc` but framed in RESP3:
//! the client issues `HELLO 3 PQC ML-KEM-768 <ek_b64>` and the server replies
//! with a Map-typed reply containing the ciphertext.

pub const SENTINEL: &str = "PQCK";
pub const VERSION_1: u8 = 1;

pub const MLKEM768_NAME: &str = "ML-KEM-768";
pub const MLKEM768_EK_BYTES: usize = 1184;
pub const MLKEM768_DK_BYTES: usize = 2400;
pub const MLKEM768_CT_BYTES: usize = 1088;
pub const MLKEM768_SS_BYTES: usize = 32;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PqcError {
    #[error("invalid ek length: {0} (expected {1})")]
    InvalidEkLen(usize, usize),
    #[error("invalid ct length: {0} (expected {1})")]
    InvalidCtLen(usize, usize),
    #[error("invalid ss length: {0} (expected {1})")]
    InvalidSsLen(usize, usize),
    #[error("unsupported mechanism: {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PqcOffer {
    pub version: u8,
    pub mechanisms: Vec<String>,
}

impl PqcOffer {
    pub fn ml_kem_768_only() -> Self {
        Self {
            version: VERSION_1,
            mechanisms: vec![MLKEM768_NAME.into()],
        }
    }

    pub fn supports(&self, mech: &str) -> bool {
        self.mechanisms.iter().any(|m| m == mech)
    }
}

pub fn negotiate(server_supported: &[&str], client_offer: &PqcOffer) -> Option<String> {
    server_supported
        .iter()
        .find(|m| client_offer.supports(m))
        .map(|s| s.to_string())
}

pub fn validate_ek(ek: &[u8]) -> Result<(), PqcError> {
    if ek.len() != MLKEM768_EK_BYTES {
        return Err(PqcError::InvalidEkLen(ek.len(), MLKEM768_EK_BYTES));
    }
    Ok(())
}

pub fn validate_ct(ct: &[u8]) -> Result<(), PqcError> {
    if ct.len() != MLKEM768_CT_BYTES {
        return Err(PqcError::InvalidCtLen(ct.len(), MLKEM768_CT_BYTES));
    }
    Ok(())
}

pub fn validate_ss(ss: &[u8]) -> Result<(), PqcError> {
    if ss.len() != MLKEM768_SS_BYTES {
        return Err(PqcError::InvalidSsLen(ss.len(), MLKEM768_SS_BYTES));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fips203_constants() {
        assert_eq!(MLKEM768_EK_BYTES, 1184);
        assert_eq!(MLKEM768_DK_BYTES, 2400);
        assert_eq!(MLKEM768_CT_BYTES, 1088);
        assert_eq!(MLKEM768_SS_BYTES, 32);
    }

    #[test]
    fn ml_kem_768_only_offer() {
        let o = PqcOffer::ml_kem_768_only();
        assert_eq!(o.version, VERSION_1);
        assert!(o.supports(MLKEM768_NAME));
    }

    #[test]
    fn negotiate_picks_server_preferred() {
        let o = PqcOffer {
            version: VERSION_1,
            mechanisms: vec!["A".into(), "B".into()],
        };
        assert_eq!(negotiate(&["X", "B", "A"], &o).as_deref(), Some("B"));
    }

    #[test]
    fn negotiate_no_overlap_returns_none() {
        let o = PqcOffer {
            version: VERSION_1,
            mechanisms: vec!["A".into()],
        };
        assert!(negotiate(&["B"], &o).is_none());
    }

    #[test]
    fn validate_ek_correct() {
        assert!(validate_ek(&vec![0u8; MLKEM768_EK_BYTES]).is_ok());
    }

    #[test]
    fn validate_ek_wrong() {
        assert!(validate_ek(&[0; 100]).is_err());
    }

    #[test]
    fn validate_ct_correct() {
        assert!(validate_ct(&vec![0u8; MLKEM768_CT_BYTES]).is_ok());
    }

    #[test]
    fn validate_ct_wrong() {
        assert!(validate_ct(&[0; 1024]).is_err());
    }

    #[test]
    fn validate_ss_correct() {
        assert!(validate_ss(&vec![0u8; MLKEM768_SS_BYTES]).is_ok());
    }

    #[test]
    fn validate_ss_wrong() {
        assert!(validate_ss(&[0; 16]).is_err());
    }

    #[test]
    fn sentinel_constant_matches_pg() {
        // Same sentinel as cave-pg so middleboxes can demux either protocol.
        assert_eq!(SENTINEL, "PQCK");
    }
}
