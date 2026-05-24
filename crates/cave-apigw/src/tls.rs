// SPDX-License-Identifier: AGPL-3.0-or-later
//! TLS termination + SNI-based certificate selection.
//!
//! `CertRegistry` maps a (host, fingerprint) tuple to PEM material loaded
//! from disk or from `cave-cert-manager` via the ACME hook.

use crate::error::{AGwError, AGwResult};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CertEntry {
    pub host: String,
    pub leaf_pem: String,
    pub chain_pem: String,
    pub key_pem: String,
    pub not_before: chrono::DateTime<chrono::Utc>,
    pub not_after: chrono::DateTime<chrono::Utc>,
    pub fingerprint_sha256: String,
}

pub struct CertRegistry {
    entries: std::sync::RwLock<HashMap<String, CertEntry>>,
    default_host: std::sync::RwLock<Option<String>>,
}

impl Default for CertRegistry {
    fn default() -> Self {
        Self {
            entries: std::sync::RwLock::new(HashMap::new()),
            default_host: std::sync::RwLock::new(None),
        }
    }
}

impl CertRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, entry: CertEntry) {
        let host = entry.host.to_lowercase();
        if self.default_host.read().unwrap().is_none() {
            *self.default_host.write().unwrap() = Some(host.clone());
        }
        self.entries.write().unwrap().insert(host, entry);
    }

    pub fn set_default(&self, host: &str) -> AGwResult<()> {
        let host = host.to_lowercase();
        if !self.entries.read().unwrap().contains_key(&host) {
            return Err(AGwError::BadRequest(format!("no cert for default {host}")));
        }
        *self.default_host.write().unwrap() = Some(host);
        Ok(())
    }

    /// Resolve a cert for a TLS SNI value, with optional wildcard match
    /// (`*.example.com` covers `api.example.com`).
    pub fn resolve(&self, sni: &str) -> Option<CertEntry> {
        let sni = sni.to_lowercase();
        let g = self.entries.read().unwrap();
        if let Some(e) = g.get(&sni) {
            return Some(e.clone());
        }
        // wildcard one-label-up
        if let Some((_, parent)) = sni.split_once('.') {
            let wildcard = format!("*.{parent}");
            if let Some(e) = g.get(&wildcard) {
                return Some(e.clone());
            }
        }
        // default fallback
        let def = self.default_host.read().unwrap().clone();
        def.and_then(|h| g.get(&h).cloned())
    }

    pub fn count(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Check if any installed cert expires within `days`.
    pub fn expiring_within(&self, days: i64) -> Vec<String> {
        let now = chrono::Utc::now();
        let threshold = now + chrono::Duration::days(days);
        self.entries
            .read()
            .unwrap()
            .iter()
            .filter(|(_, e)| e.not_after <= threshold)
            .map(|(h, _)| h.clone())
            .collect()
    }
}

/// TLS protocol bounds — applied when wiring rustls in the runtime adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsBounds {
    pub min_version: TlsVersion,
    pub max_version: TlsVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TlsVersion {
    Tls12,
    Tls13,
}

impl Default for TlsBounds {
    fn default() -> Self {
        Self { min_version: TlsVersion::Tls12, max_version: TlsVersion::Tls13 }
    }
}

impl TlsBounds {
    pub fn validate(&self) -> AGwResult<()> {
        if self.min_version > self.max_version {
            return Err(AGwError::BadRequest("min > max TLS version".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(host: &str, days_until_expiry: i64) -> CertEntry {
        let now = chrono::Utc::now();
        CertEntry {
            host: host.into(),
            leaf_pem: "PEM".into(),
            chain_pem: "CHAIN".into(),
            key_pem: "KEY".into(),
            not_before: now,
            not_after: now + chrono::Duration::days(days_until_expiry),
            fingerprint_sha256: "abc".into(),
        }
    }

    #[test]
    fn insert_and_resolve_exact() {
        let r = CertRegistry::new();
        r.insert(entry("api.example.com", 30));
        let got = r.resolve("api.example.com").unwrap();
        assert_eq!(got.host, "api.example.com");
    }

    #[test]
    fn resolve_wildcard() {
        let r = CertRegistry::new();
        r.insert(entry("*.example.com", 30));
        let got = r.resolve("foo.example.com").unwrap();
        assert!(got.host.starts_with("*."));
    }

    #[test]
    fn resolve_default_fallback() {
        let r = CertRegistry::new();
        r.insert(entry("default.example", 30));
        r.set_default("default.example").unwrap();
        let got = r.resolve("nope.example").unwrap();
        assert_eq!(got.host, "default.example");
    }

    #[test]
    fn expiring_within() {
        let r = CertRegistry::new();
        r.insert(entry("near.example", 3));
        r.insert(entry("far.example", 90));
        let n = r.expiring_within(7);
        assert!(n.contains(&"near.example".to_string()));
        assert!(!n.contains(&"far.example".to_string()));
    }

    #[test]
    fn tls_bounds_validate() {
        TlsBounds::default().validate().unwrap();
        let b = TlsBounds { min_version: TlsVersion::Tls13, max_version: TlsVersion::Tls12 };
        assert!(b.validate().is_err());
    }
}
