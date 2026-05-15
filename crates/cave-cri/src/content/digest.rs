//! Content digest — the upstream `digest.Digest` wrapper, with
//! parse + verify + algorithm dispatch.
//!
//! Wire form is `<alg>:<hex>` (e.g. `sha256:e3b0c4...`), matching
//! containerd / OCI image spec. We accept sha256 / sha512 / sha384;
//! other algorithms are rejected at parse time.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DigestError {
    #[error("digest missing `:` separator: {0}")]
    MissingSeparator(String),
    #[error("unknown digest algorithm: {0}")]
    UnknownAlgorithm(String),
    #[error("hex must be {expected} chars for {alg}, got {actual}")]
    HexLength {
        alg: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("hex contains a non-hex character at byte {0}")]
    NonHex(usize),
    #[error("digest mismatch: expected {expected}, got {actual}")]
    Mismatch { expected: String, actual: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DigestAlgorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl DigestAlgorithm {
    pub const fn as_str(self) -> &'static str {
        match self {
            DigestAlgorithm::Sha256 => "sha256",
            DigestAlgorithm::Sha384 => "sha384",
            DigestAlgorithm::Sha512 => "sha512",
        }
    }

    /// Expected hex-string length for this algorithm.
    pub const fn hex_len(self) -> usize {
        match self {
            DigestAlgorithm::Sha256 => 64,
            DigestAlgorithm::Sha384 => 96,
            DigestAlgorithm::Sha512 => 128,
        }
    }
}

impl FromStr for DigestAlgorithm {
    type Err = DigestError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sha256" => Ok(Self::Sha256),
            "sha384" => Ok(Self::Sha384),
            "sha512" => Ok(Self::Sha512),
            other => Err(DigestError::UnknownAlgorithm(other.into())),
        }
    }
}

/// An immutable digest. Constructed via [`Digest::parse`] or
/// [`Digest::compute`]; both validate the inner hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Digest {
    algorithm: DigestAlgorithm,
    /// Lower-case hex.
    hex: String,
}

impl Digest {
    pub fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    pub fn hex(&self) -> &str {
        &self.hex
    }

    /// Parse a wire-form digest like `sha256:e3b0c4...`.
    pub fn parse(s: &str) -> Result<Self, DigestError> {
        let (alg_s, hex) = s
            .split_once(':')
            .ok_or_else(|| DigestError::MissingSeparator(s.into()))?;
        let algorithm: DigestAlgorithm = alg_s.parse()?;
        if hex.len() != algorithm.hex_len() {
            return Err(DigestError::HexLength {
                alg: algorithm.as_str(),
                expected: algorithm.hex_len(),
                actual: hex.len(),
            });
        }
        for (i, b) in hex.bytes().enumerate() {
            if !b.is_ascii_hexdigit() || (b.is_ascii_alphabetic() && b.is_ascii_uppercase()) {
                return Err(DigestError::NonHex(i));
            }
        }
        Ok(Self {
            algorithm,
            hex: hex.into(),
        })
    }

    /// Compute the digest of an in-memory byte slice.
    pub fn compute(algorithm: DigestAlgorithm, bytes: &[u8]) -> Self {
        use sha2::Digest as _;
        let hex = match algorithm {
            DigestAlgorithm::Sha256 => hex_encode(&sha2::Sha256::digest(bytes)),
            DigestAlgorithm::Sha384 => hex_encode(&sha2::Sha384::digest(bytes)),
            DigestAlgorithm::Sha512 => hex_encode(&sha2::Sha512::digest(bytes)),
        };
        Self { algorithm, hex }
    }

    /// Path-friendly form `<alg>/<hex>` used by the on-disk blob
    /// layout (`<root>/blobs/<alg>/<hex>`).
    pub fn fs_path(&self) -> String {
        format!("{}/{}", self.algorithm.as_str(), self.hex)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm.as_str(), self.hex)
    }
}

impl FromStr for Digest {
    type Err = DigestError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_via_display() {
        let s = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let d = Digest::parse(s).unwrap();
        assert_eq!(d.algorithm(), DigestAlgorithm::Sha256);
        assert_eq!(format!("{}", d), s);
    }

    #[test]
    fn parse_rejects_uppercase_hex() {
        let s = "sha256:E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
        assert!(Digest::parse(s).is_err());
    }

    #[test]
    fn parse_rejects_short_hex() {
        let s = "sha256:abc";
        match Digest::parse(s).unwrap_err() {
            DigestError::HexLength { actual: 3, expected: 64, .. } => {}
            e => panic!("unexpected error {e:?}"),
        }
    }

    #[test]
    fn parse_rejects_unknown_algorithm() {
        let s = "md5:00000000000000000000000000000000";
        match Digest::parse(s).unwrap_err() {
            DigestError::UnknownAlgorithm(a) => assert_eq!(a, "md5"),
            e => panic!("unexpected error {e:?}"),
        }
    }

    #[test]
    fn parse_rejects_missing_separator() {
        match Digest::parse("just-some-string").unwrap_err() {
            DigestError::MissingSeparator(_) => {}
            e => panic!("unexpected error {e:?}"),
        }
    }

    #[test]
    fn compute_matches_known_sha256() {
        // Well-known SHA-256 of the empty string.
        let d = Digest::compute(DigestAlgorithm::Sha256, b"");
        assert_eq!(
            d.to_string(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn compute_matches_known_sha512() {
        let d = Digest::compute(DigestAlgorithm::Sha512, b"");
        assert_eq!(
            d.to_string(),
            "sha512:cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
    }

    #[test]
    fn fs_path_uses_slash_separator() {
        let d = Digest::compute(DigestAlgorithm::Sha256, b"abc");
        assert!(d.fs_path().starts_with("sha256/"));
        assert_eq!(d.fs_path(), format!("sha256/{}", d.hex()));
    }

    #[test]
    fn hex_len_matches_each_algorithm() {
        assert_eq!(DigestAlgorithm::Sha256.hex_len(), 64);
        assert_eq!(DigestAlgorithm::Sha384.hex_len(), 96);
        assert_eq!(DigestAlgorithm::Sha512.hex_len(), 128);
    }
}
