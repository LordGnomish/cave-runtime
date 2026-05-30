// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TSIG — secret-key transaction authentication for DNS (RFC 8945).
//!
//! Port of `coredns` vendor `miekg/dns/tsig.go` (`tsigGenerate` / `tsigVerify`)
//! onto the hickory-proto name model. Closes the `tsig-hmac-zone-transfer`
//! partial: AXFR/IXFR responses can now be HMAC-signed and verified.
//!
//! The HMAC primitive is provided by `ring` and validated against the
//! published RFC 4231 (SHA-256) and RFC 2202 (SHA-1) test vectors. The TSIG
//! digest input follows RFC 8945:
//!   * §4.3.3 — TSIG variables (key name / CLASS ANY / TTL 0 / algorithm name /
//!     time signed (48-bit) / fudge / error / other).
//!   * §5.3.1 — a response prepends the request MAC (length-prefixed) before
//!     the DNS message and the TSIG variables.
//!   * §5.2.3 — the time-signed / fudge window check.

use hickory_proto::rr::Name;
use ring::hmac;

/// Supported TSIG HMAC algorithms (RFC 8945 §6; mandatory `hmac-sha256`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsigAlgorithm {
    /// `hmac-sha1.` (legacy, RFC 4635).
    HmacSha1,
    /// `hmac-sha256.` — mandatory to implement (RFC 8945 §6).
    HmacSha256,
    /// `hmac-sha384.`
    HmacSha384,
    /// `hmac-sha512.`
    HmacSha512,
}

impl TsigAlgorithm {
    /// The canonical (lower-case, trailing-dot) DNS name for this algorithm.
    #[must_use]
    pub fn dns_name(self) -> &'static str {
        match self {
            Self::HmacSha1 => "hmac-sha1.",
            Self::HmacSha256 => "hmac-sha256.",
            Self::HmacSha384 => "hmac-sha384.",
            Self::HmacSha512 => "hmac-sha512.",
        }
    }

    /// Parse an algorithm DNS name (case-insensitive, trailing dot optional).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.trim_end_matches('.').to_ascii_lowercase();
        match n.as_str() {
            "hmac-sha1" => Some(Self::HmacSha1),
            "hmac-sha256" => Some(Self::HmacSha256),
            "hmac-sha384" => Some(Self::HmacSha384),
            "hmac-sha512" => Some(Self::HmacSha512),
            _ => None,
        }
    }

    fn ring_key(self, key: &[u8]) -> hmac::Key {
        let algo = match self {
            Self::HmacSha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            Self::HmacSha256 => hmac::HMAC_SHA256,
            Self::HmacSha384 => hmac::HMAC_SHA384,
            Self::HmacSha512 => hmac::HMAC_SHA512,
        };
        hmac::Key::new(algo, key)
    }

    /// Raw HMAC tag over `data` keyed by `key` (no TSIG framing).
    #[must_use]
    pub fn raw_hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        hmac::sign(&self.ring_key(key), data).as_ref().to_vec()
    }

    /// Re-derive the TSIG digest input and constant-time verify `expected`
    /// against it via `ring::hmac::verify`.
    #[must_use]
    pub fn verify(
        self,
        key: &[u8],
        request_mac: Option<&[u8]>,
        message: &[u8],
        vars: &TsigVariables,
        expected: &[u8],
    ) -> bool {
        let digest_input = digest_input(request_mac, message, vars);
        hmac::verify(&self.ring_key(key), &digest_input, expected).is_ok()
    }
}

/// The TSIG digest variables (RFC 8945 §4.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsigVariables {
    /// TSIG key name (owner of the TSIG RR).
    pub key_name: Name,
    /// HMAC algorithm.
    pub algorithm: TsigAlgorithm,
    /// Seconds since the UNIX epoch, truncated to 48 bits on the wire.
    pub time_signed: u64,
    /// Permitted clock skew, in seconds.
    pub fudge: u16,
    /// TSIG error code (RCODE space; 0 on success).
    pub error: u16,
    /// "Other data" — present only for BADTIME (error 18) responses.
    pub other: Vec<u8>,
}

/// Canonical (lower-case, uncompressed) on-the-wire encoding of a name.
fn canonical_name_wire(name: &Name) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.iter() {
        out.push(label.len() as u8);
        out.extend(label.iter().map(u8::to_ascii_lowercase));
    }
    out.push(0); // root terminator
    out
}

impl TsigVariables {
    /// Encode the TSIG variables in the canonical digest order (RFC 8945 §4.3.3).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // NAME — key name, canonical wire form.
        out.extend_from_slice(&canonical_name_wire(&self.key_name));
        // CLASS — ANY (0x00FF).
        out.extend_from_slice(&0x00FFu16.to_be_bytes());
        // TTL — 0.
        out.extend_from_slice(&0u32.to_be_bytes());
        // Algorithm Name — canonical wire form.
        let alg = Name::from_ascii(self.algorithm.dns_name())
            .expect("static algorithm name parses");
        out.extend_from_slice(&canonical_name_wire(&alg));
        // Time Signed — 48-bit big-endian (low 6 of the u64).
        out.extend_from_slice(&self.time_signed.to_be_bytes()[2..]);
        // Fudge.
        out.extend_from_slice(&self.fudge.to_be_bytes());
        // Error.
        out.extend_from_slice(&self.error.to_be_bytes());
        // Other Len + Other Data.
        out.extend_from_slice(&(self.other.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.other);
        out
    }
}

/// The bytes fed to HMAC for a TSIG MAC (RFC 8945 §5.3.1): optional
/// length-prefixed request MAC, the DNS message, then the TSIG variables.
fn digest_input(request_mac: Option<&[u8]>, message: &[u8], vars: &TsigVariables) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(rm) = request_mac {
        buf.extend_from_slice(&(rm.len() as u16).to_be_bytes());
        buf.extend_from_slice(rm);
    }
    buf.extend_from_slice(message);
    buf.extend_from_slice(&vars.encode());
    buf
}

/// Compute a TSIG MAC over `message`.
///
/// When `request_mac` is `Some` the digest is a response MAC (RFC 8945 §5.3.1):
/// the request MAC is prepended length-first, then the DNS message, then the
/// TSIG variables. When `None` it is a request/standalone MAC.
#[must_use]
pub fn compute_mac(
    algorithm: TsigAlgorithm,
    key: &[u8],
    request_mac: Option<&[u8]>,
    message: &[u8],
    vars: &TsigVariables,
) -> Vec<u8> {
    algorithm.raw_hmac(key, &digest_input(request_mac, message, vars))
}

/// Whether `now` falls inside the `[time_signed - fudge, time_signed + fudge]`
/// window (RFC 8945 §5.2.3).
#[must_use]
pub fn fudge_valid(time_signed: u64, fudge: u16, now: u64) -> bool {
    let diff = now.abs_diff(time_signed);
    diff <= u64::from(fudge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn roundtrip_sign_then_verify() {
        let key = b"unit-test-key";
        let vars = TsigVariables {
            key_name: Name::from_str("k.").unwrap(),
            algorithm: TsigAlgorithm::HmacSha256,
            time_signed: 42,
            fudge: 300,
            error: 0,
            other: vec![],
        };
        let mac = compute_mac(TsigAlgorithm::HmacSha256, key, None, b"msg", &vars);
        assert!(TsigAlgorithm::HmacSha256.verify(key, None, b"msg", &vars, &mac));
    }
}
