// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TSIG transaction signatures for authenticated zone transfer (RFC 8945).
//!
//! Ports the MAC half of miekg/dns `tsig.go` (vendored by CoreDNS v1.14.3):
//! an HMAC-SHA256 keyed digest over the canonical request/response bytes, with
//! a constant-time verify. The key name + algorithm metadata is carried so the
//! secondary/transfer path (`zone::transfer`) can sign AXFR/IXFR exchanges.
use ring::hmac;

/// A named TSIG key. Only HMAC-SHA256 (the RFC 8945 MUST-implement algorithm)
/// is supported.
#[derive(Clone)]
pub struct TsigKey {
    pub name: String,
    secret: Vec<u8>,
}

impl TsigKey {
    pub fn new(name: impl Into<String>, secret: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            secret,
        }
    }

    /// The RFC 8945 algorithm name advertised in the TSIG RR.
    pub fn algorithm(&self) -> &'static str {
        "hmac-sha256."
    }

    /// Compute the HMAC-SHA256 MAC over `message`.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        hmac::sign(&key, message).as_ref().to_vec()
    }

    /// Constant-time verification that `tag` is the MAC of `message`.
    pub fn verify(&self, message: &[u8], tag: &[u8]) -> bool {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        hmac::verify(&key, message, tag).is_ok()
    }
}
