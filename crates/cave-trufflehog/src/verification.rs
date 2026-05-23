// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Live HTTP verification — port of `pkg/verificationcache/` + the
//! `successRanges` / `rotatedRanges` semantics added in upstream #4892.

use crate::models::DetectorType;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One verification verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    Unverified,
    Rotated,
    Indeterminate,
}

#[derive(Debug, Clone)]
pub struct CachedVerdict {
    pub verdict: Verdict,
    pub at: Instant,
}

/// LRU-flavoured cache. Bounded by max entries; on overflow oldest entries
/// evicted. Matches upstream `verificationcache.InMemoryMetrics` shape but
/// without the prometheus surface (we expose Prometheus separately in
/// `crate::metrics`).
pub struct VerificationCache {
    inner: Mutex<HashMap<String, CachedVerdict>>,
    pub capacity: usize,
    pub ttl: Duration,
}

impl VerificationCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            capacity,
            ttl: Duration::from_secs(60 * 60),
        }
    }

    pub fn key(t: DetectorType, raw: &str) -> String {
        format!("{}:{}", t as u32, raw)
    }

    pub fn get(&self, t: DetectorType, raw: &str) -> Option<Verdict> {
        let g = self.inner.lock().unwrap();
        let entry = g.get(&Self::key(t, raw))?;
        if entry.at.elapsed() > self.ttl {
            return None;
        }
        Some(entry.verdict)
    }

    pub fn put(&self, t: DetectorType, raw: &str, verdict: Verdict) {
        let mut g = self.inner.lock().unwrap();
        if g.len() >= self.capacity {
            // Drop oldest entry (linear scan acceptable for small caches).
            if let Some((k, _)) = g.iter().min_by_key(|(_, v)| v.at) {
                let k = k.clone();
                g.remove(&k);
            }
        }
        g.insert(
            Self::key(t, raw),
            CachedVerdict {
                verdict,
                at: Instant::now(),
            },
        );
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Range of HTTP status codes treated by a particular detector's verifier
/// as a positive verdict. Port of upstream #4892 `successRanges`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRange {
    pub lo: u16,
    pub hi: u16,
}

impl StatusRange {
    pub fn new(lo: u16, hi: u16) -> Self {
        debug_assert!(lo <= hi);
        Self { lo, hi }
    }

    pub fn contains(&self, code: u16) -> bool {
        code >= self.lo && code <= self.hi
    }
}

/// Per-detector verifier semantics. `success_ranges` -> Verified, anything
/// inside `rotated_ranges` -> Rotated, anything else -> Indeterminate. The
/// caller still gets to override on body parsing (e.g. Slack's `ok=false`).
#[derive(Debug, Clone, Default)]
pub struct VerifierConfig {
    pub success_ranges: Vec<StatusRange>,
    pub rotated_ranges: Vec<StatusRange>,
}

impl VerifierConfig {
    pub fn ok_2xx() -> Self {
        Self {
            success_ranges: vec![StatusRange::new(200, 299)],
            rotated_ranges: vec![StatusRange::new(401, 401), StatusRange::new(403, 403)],
        }
    }

    pub fn classify(&self, status: u16) -> Verdict {
        if self.success_ranges.iter().any(|r| r.contains(status)) {
            Verdict::Verified
        } else if self.rotated_ranges.iter().any(|r| r.contains(status)) {
            Verdict::Rotated
        } else if (500..=599).contains(&status) {
            Verdict::Indeterminate
        } else {
            Verdict::Unverified
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_round_trip() {
        let c = VerificationCache::new(4);
        assert_eq!(c.get(DetectorType::Stripe, "x"), None);
        c.put(DetectorType::Stripe, "x", Verdict::Verified);
        assert_eq!(c.get(DetectorType::Stripe, "x"), Some(Verdict::Verified));
    }

    #[test]
    fn capacity_evicts_oldest() {
        let c = VerificationCache::new(2);
        c.put(DetectorType::Stripe, "a", Verdict::Verified);
        std::thread::sleep(Duration::from_millis(2));
        c.put(DetectorType::Stripe, "b", Verdict::Verified);
        std::thread::sleep(Duration::from_millis(2));
        c.put(DetectorType::Stripe, "c", Verdict::Verified);
        assert_eq!(c.len(), 2);
        // "a" was oldest, should be gone.
        assert_eq!(c.get(DetectorType::Stripe, "a"), None);
        assert_eq!(c.get(DetectorType::Stripe, "c"), Some(Verdict::Verified));
    }

    #[test]
    fn ok_2xx_classifies_success_and_rotated() {
        let v = VerifierConfig::ok_2xx();
        assert_eq!(v.classify(200), Verdict::Verified);
        assert_eq!(v.classify(204), Verdict::Verified);
        assert_eq!(v.classify(401), Verdict::Rotated);
        assert_eq!(v.classify(403), Verdict::Rotated);
        assert_eq!(v.classify(400), Verdict::Unverified);
        assert_eq!(v.classify(500), Verdict::Indeterminate);
    }

    #[test]
    fn key_includes_detector_type() {
        let a = VerificationCache::key(DetectorType::Aws, "x");
        let b = VerificationCache::key(DetectorType::Github, "x");
        assert_ne!(a, b);
    }

    #[test]
    fn custom_success_range() {
        let v = VerifierConfig {
            success_ranges: vec![StatusRange::new(200, 200), StatusRange::new(204, 204)],
            rotated_ranges: vec![],
        };
        assert_eq!(v.classify(200), Verdict::Verified);
        assert_eq!(v.classify(204), Verdict::Verified);
        assert_eq!(v.classify(201), Verdict::Unverified);
    }
}
