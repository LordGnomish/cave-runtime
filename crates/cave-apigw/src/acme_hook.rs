// SPDX-License-Identifier: AGPL-3.0-or-later
//! ACME / cave-cert-manager hook.

use crate::error::AGwResult;
use crate::tls::CertEntry;

pub trait AcmeProvider: Send + Sync {
    fn issue(&self, host: &str) -> AGwResult<CertEntry>;
    fn maybe_renew(&self, host: &str, renewal_window_days: i64) -> AGwResult<Option<CertEntry>>;
}

pub struct StubAcme;
impl AcmeProvider for StubAcme {
    fn issue(&self, host: &str) -> AGwResult<CertEntry> {
        let now = chrono::Utc::now();
        Ok(CertEntry {
            host: host.into(),
            leaf_pem: format!("STUB-LEAF-{host}"),
            chain_pem: format!("STUB-CHAIN-{host}"),
            key_pem: format!("STUB-KEY-{host}"),
            not_before: now, not_after: now + chrono::Duration::days(90),
            fingerprint_sha256: hex_fp(host),
        })
    }
    fn maybe_renew(&self, host: &str, renewal_window_days: i64) -> AGwResult<Option<CertEntry>> {
        if renewal_window_days <= 0 || host.is_empty() { return Ok(None); }
        Ok(Some(self.issue(host)?))
    }
}

fn hex_fp(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new(); h.update(input.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn issue_cert() {
        let e = StubAcme.issue("api.example").unwrap();
        assert_eq!(e.host, "api.example");
        assert!((e.not_after - e.not_before).num_days() >= 80);
        assert!(!e.fingerprint_sha256.is_empty());
    }
    #[test] fn renew_within_window() { assert!(StubAcme.maybe_renew("api.example", 30).unwrap().is_some()); }
    #[test] fn no_renew_zero_window() { assert!(StubAcme.maybe_renew("api.example", 0).unwrap().is_none()); }
    #[test] fn no_renew_empty_host() { assert!(StubAcme.maybe_renew("", 30).unwrap().is_none()); }
}
