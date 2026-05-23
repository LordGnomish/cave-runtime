// SPDX-License-Identifier: AGPL-3.0-or-later
//! TLS termination + SNI-based cert selection.

use crate::error::{AGwError, AGwResult};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CertEntry {
    pub host: String, pub leaf_pem: String, pub chain_pem: String, pub key_pem: String,
    pub not_before: chrono::DateTime<chrono::Utc>, pub not_after: chrono::DateTime<chrono::Utc>,
    pub fingerprint_sha256: String,
}

pub struct CertRegistry {
    entries: std::sync::RwLock<HashMap<String, CertEntry>>,
    default_host: std::sync::RwLock<Option<String>>,
}
impl Default for CertRegistry {
    fn default() -> Self {
        Self { entries: std::sync::RwLock::new(HashMap::new()), default_host: std::sync::RwLock::new(None) }
    }
}
impl CertRegistry {
    pub fn new() -> Self { Self::default() }
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
        *self.default_host.write().unwrap() = Some(host); Ok(())
    }
    pub fn resolve(&self, sni: &str) -> Option<CertEntry> {
        let sni = sni.to_lowercase();
        let g = self.entries.read().unwrap();
        if let Some(e) = g.get(&sni) { return Some(e.clone()); }
        if let Some((_, parent)) = sni.split_once('.') {
            let wildcard = format!("*.{parent}");
            if let Some(e) = g.get(&wildcard) { return Some(e.clone()); }
        }
        self.default_host.read().unwrap().clone().and_then(|h| g.get(&h).cloned())
    }
    pub fn count(&self) -> usize { self.entries.read().unwrap().len() }
    pub fn expiring_within(&self, days: i64) -> Vec<String> {
        let threshold = chrono::Utc::now() + chrono::Duration::days(days);
        self.entries.read().unwrap().iter()
            .filter(|(_, e)| e.not_after <= threshold).map(|(h, _)| h.clone()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsBounds { pub min_version: TlsVersion, pub max_version: TlsVersion }
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TlsVersion { Tls12, Tls13 }
impl Default for TlsBounds { fn default() -> Self { Self { min_version: TlsVersion::Tls12, max_version: TlsVersion::Tls13 } } }
impl TlsBounds {
    pub fn validate(&self) -> AGwResult<()> {
        if self.min_version > self.max_version { return Err(AGwError::BadRequest("min > max TLS version".into())); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(host: &str, days: i64) -> CertEntry {
        let now = chrono::Utc::now();
        CertEntry { host: host.into(), leaf_pem: "P".into(), chain_pem: "C".into(), key_pem: "K".into(),
            not_before: now, not_after: now + chrono::Duration::days(days),
            fingerprint_sha256: "abc".into() }
    }
    #[test] fn exact_match() {
        let r = CertRegistry::new(); r.insert(entry("api.example", 30));
        assert_eq!(r.resolve("api.example").unwrap().host, "api.example");
    }
    #[test] fn wildcard_match() {
        let r = CertRegistry::new(); r.insert(entry("*.example", 30));
        assert!(r.resolve("foo.example").unwrap().host.starts_with("*."));
    }
    #[test] fn default_fallback() {
        let r = CertRegistry::new(); r.insert(entry("def.example", 30));
        r.set_default("def.example").unwrap();
        assert_eq!(r.resolve("nope.example").unwrap().host, "def.example");
    }
    #[test] fn expiring_filter() {
        let r = CertRegistry::new();
        r.insert(entry("near", 3)); r.insert(entry("far", 90));
        let n = r.expiring_within(7);
        assert!(n.contains(&"near".to_string()));
        assert!(!n.contains(&"far".to_string()));
    }
    #[test] fn bounds_validate() {
        TlsBounds::default().validate().unwrap();
        let b = TlsBounds { min_version: TlsVersion::Tls13, max_version: TlsVersion::Tls12 };
        assert!(b.validate().is_err());
    }
}
