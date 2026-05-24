// SPDX-License-Identifier: AGPL-3.0-or-later
//! ACME / cave-cert-manager hook.
//!
//! Trait-based bridge so the gateway can request a fresh certificate without
//! depending on `cave-cert-manager` directly. Production wiring is done at the
//! runtime binary layer.

use crate::error::AGwResult;
use crate::tls::CertEntry;

pub trait AcmeProvider: Send + Sync {
    /// Request a fresh certificate for `host`. Returns the new entry on success.
    fn issue(&self, host: &str) -> AGwResult<CertEntry>;

    /// Trigger renewal if the certificate is within `renewal_window_days`.
    fn maybe_renew(&self, host: &str, renewal_window_days: i64) -> AGwResult<Option<CertEntry>>;
}

/// In-memory stub that produces deterministic certs for tests.
pub struct StubAcme;

impl AcmeProvider for StubAcme {
    fn issue(&self, host: &str) -> AGwResult<CertEntry> {
        let now = chrono::Utc::now();
        Ok(CertEntry {
            host: host.into(),
            leaf_pem: format!("STUB-LEAF-{host}"),
            chain_pem: format!("STUB-CHAIN-{host}"),
            key_pem: format!("STUB-KEY-{host}"),
            not_before: now,
            not_after: now + chrono::Duration::days(90),
            fingerprint_sha256: hex_fp(host),
        })
    }

    fn maybe_renew(&self, host: &str, renewal_window_days: i64) -> AGwResult<Option<CertEntry>> {
        // Stub always returns Some when window > 0 and host is non-empty.
        if renewal_window_days <= 0 || host.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.issue(host)?))
    }
}

fn hex_fp(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_issues_cert() {
        let p = StubAcme;
        let e = p.issue("api.example.com").unwrap();
        assert_eq!(e.host, "api.example.com");
        assert!((e.not_after - e.not_before).num_days() >= 80);
        assert!(!e.fingerprint_sha256.is_empty());
    }

    #[test]
    fn stub_renews_within_window() {
        let p = StubAcme;
        let e = p.maybe_renew("api.example.com", 30).unwrap();
        assert!(e.is_some());
    }

    #[test]
    fn stub_skip_renew_when_window_zero() {
        let p = StubAcme;
        let e = p.maybe_renew("api.example.com", 0).unwrap();
        assert!(e.is_none());
    }

    #[test]
    fn stub_skip_renew_when_host_empty() {
        let p = StubAcme;
        let e = p.maybe_renew("", 30).unwrap();
        assert!(e.is_none());
    }
}
