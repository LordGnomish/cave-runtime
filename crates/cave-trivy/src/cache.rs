// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Scan cache + vuln-DB cache.
//!
//! Mirrors trivy's `pkg/cache` — content-addressed by SHA-256 of the
//! scan artifact identifier. cave-trivy MVP uses an in-process
//! `RwLock<HashMap<_, _>>`; the sled-backed persistent variant is a
//! scope cut.

use crate::models::Report;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub report: Report,
    pub stored_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct ScanCache {
    inner: Arc<RwLock<HashMap<String, CacheEntry>>>,
}

impl ScanCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key(target: &str) -> String {
        let mut h = Sha256::new();
        h.update(target.as_bytes());
        let bytes = h.finalize();
        format!("sha256:{}", hex::encode(bytes))
    }

    pub fn put(&self, target: &str, report: Report) -> String {
        let k = Self::key(target);
        let mut g = self.inner.write().expect("cache poisoned");
        g.insert(
            k.clone(),
            CacheEntry {
                report,
                stored_at: chrono::Utc::now(),
            },
        );
        k
    }

    pub fn get(&self, target: &str) -> Option<Report> {
        let k = Self::key(target);
        let g = self.inner.read().expect("cache poisoned");
        g.get(&k).map(|e| e.report.clone())
    }

    pub fn len(&self) -> usize {
        self.inner.read().expect("cache poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.inner.write().expect("cache poisoned").clear();
    }

    pub fn keys(&self) -> Vec<String> {
        self.inner
            .read()
            .expect("cache poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_round_trip() {
        let c = ScanCache::new();
        let r = Report::new("alpine:3.19", "container_image");
        let k = c.put("alpine:3.19", r.clone());
        assert!(k.starts_with("sha256:"));
        let back = c.get("alpine:3.19").unwrap();
        assert_eq!(back.artifact_name, "alpine:3.19");
    }

    #[test]
    fn miss_returns_none() {
        let c = ScanCache::new();
        assert!(c.get("nope").is_none());
    }

    #[test]
    fn clear_drops_entries() {
        let c = ScanCache::new();
        c.put("a", Report::new("a", "x"));
        c.put("b", Report::new("b", "x"));
        assert_eq!(c.len(), 2);
        c.clear();
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn deterministic_key() {
        let k1 = ScanCache::key("x");
        let k2 = ScanCache::key("x");
        assert_eq!(k1, k2);
        let k3 = ScanCache::key("y");
        assert_ne!(k1, k3);
    }

    #[test]
    fn keys_listing() {
        let c = ScanCache::new();
        c.put("a", Report::new("a", "x"));
        c.put("b", Report::new("b", "x"));
        let mut ks = c.keys();
        ks.sort();
        assert_eq!(ks.len(), 2);
    }
}
