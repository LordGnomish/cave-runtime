// SPDX-License-Identifier: AGPL-3.0-or-later
//! CRI image GC manager — LRU eviction with disk-pressure trigger.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `pkg/kubelet/images/image_gc_manager.go`
//!     (`realImageGCManager.GarbageCollect`,
//!      `realImageGCManager.detectImages`,
//!      `freeSpace`).
//!
//! Behavior reproduced here:
//!
//!   * GC fires when disk usage ≥ `high_threshold_percent`; targets
//!     bringing usage down to `low_threshold_percent`.
//!   * Currently-in-use images (held by any running container) are
//!     never evicted.
//!   * Among free images, eviction order is "least-recently-used"
//!     (oldest `last_used_at`) first; ties break on largest size first
//!     (free more disk per eviction).
//!   * Images younger than `min_age_secs` are never evicted (avoid
//!     thrashing newly pulled images).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageGcError {
    #[error("threshold inversion: high {high} ≤ low {low}")]
    ThresholdInversion { high: u8, low: u8 },
    #[error("threshold over 100%: {0}")]
    OverHundred(u8),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRecord {
    pub id: String,
    pub size_bytes: u64,
    pub last_used_at: DateTime<Utc>,
    pub pulled_at: DateTime<Utc>,
    pub tenant_id: String,
}

#[derive(Debug, Clone)]
pub struct ImageGcPolicy {
    pub high_threshold_percent: u8,
    pub low_threshold_percent: u8,
    pub min_age_secs: i64,
}

impl ImageGcPolicy {
    pub fn validate(&self) -> Result<(), ImageGcError> {
        if self.high_threshold_percent > 100 {
            return Err(ImageGcError::OverHundred(self.high_threshold_percent));
        }
        if self.low_threshold_percent > 100 {
            return Err(ImageGcError::OverHundred(self.low_threshold_percent));
        }
        if self.high_threshold_percent <= self.low_threshold_percent {
            return Err(ImageGcError::ThresholdInversion {
                high: self.high_threshold_percent,
                low: self.low_threshold_percent,
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ImageGcManager {
    pub disk_capacity_bytes: u64,
    /// id → record
    pub images: BTreeMap<String, ImageRecord>,
    /// image ids currently referenced by ≥1 running container
    pub in_use: BTreeSet<String>,
    pub policy: ImageGcPolicy,
}

impl ImageGcManager {
    pub fn new(disk_capacity_bytes: u64, policy: ImageGcPolicy) -> Self {
        Self {
            disk_capacity_bytes,
            images: BTreeMap::new(),
            in_use: BTreeSet::new(),
            policy,
        }
    }

    pub fn admit(&mut self, rec: ImageRecord) {
        self.images.insert(rec.id.clone(), rec);
    }

    pub fn mark_in_use(&mut self, id: &str) {
        self.in_use.insert(id.into());
    }

    pub fn release(&mut self, id: &str) {
        self.in_use.remove(id);
    }

    pub fn used_bytes(&self) -> u64 {
        self.images.values().map(|i| i.size_bytes).sum()
    }

    pub fn used_percent(&self) -> u8 {
        if self.disk_capacity_bytes == 0 {
            return 0;
        }
        (self.used_bytes() as u128 * 100u128 / self.disk_capacity_bytes as u128) as u8
    }

    pub fn under_pressure(&self) -> bool {
        self.used_percent() >= self.policy.high_threshold_percent
    }

    /// Run the GC pass. Returns the list of evicted image IDs.
    pub fn collect(&mut self, now: DateTime<Utc>) -> Vec<String> {
        if !self.under_pressure() {
            return vec![];
        }
        let target_bytes = (self.disk_capacity_bytes as u128
            * self.policy.low_threshold_percent as u128
            / 100u128) as u64;
        // Pool of evictable images: not in use AND age ≥ min_age.
        let min_age = Duration::seconds(self.policy.min_age_secs);
        let mut pool: Vec<ImageRecord> = self
            .images
            .values()
            .filter(|i| !self.in_use.contains(&i.id))
            .filter(|i| now.signed_duration_since(i.pulled_at) >= min_age)
            .cloned()
            .collect();
        // LRU first (oldest last_used). Ties → larger size first.
        pool.sort_by(|a, b| {
            a.last_used_at
                .cmp(&b.last_used_at)
                .then(b.size_bytes.cmp(&a.size_bytes))
        });
        let mut evicted = Vec::new();
        for rec in pool {
            if self.used_bytes() <= target_bytes {
                break;
            }
            self.images.remove(&rec.id);
            evicted.push(rec.id);
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(id: &str, size: u64, last_used_secs_ago: i64, pulled_secs_ago: i64) -> ImageRecord {
        let now = Utc::now();
        ImageRecord {
            id: id.into(),
            size_bytes: size,
            last_used_at: now - Duration::seconds(last_used_secs_ago),
            pulled_at: now - Duration::seconds(pulled_secs_ago),
            tenant_id: "acme".into(),
        }
    }

    fn default_policy() -> ImageGcPolicy {
        ImageGcPolicy {
            high_threshold_percent: 85,
            low_threshold_percent: 80,
            min_age_secs: 60,
        }
    }

    #[test]
    fn policy_threshold_inversion_rejected() {
        let p = ImageGcPolicy {
            high_threshold_percent: 70,
            low_threshold_percent: 80,
            min_age_secs: 60,
        };
        assert!(matches!(
            p.validate(),
            Err(ImageGcError::ThresholdInversion { .. })
        ));
    }

    #[test]
    fn used_percent_zero_capacity_is_zero() {
        let m = ImageGcManager::new(0, default_policy());
        assert_eq!(m.used_percent(), 0);
    }

    #[test]
    fn under_pressure_only_at_or_above_high() {
        let mut m = ImageGcManager::new(100, default_policy());
        m.admit(img("a", 80, 100, 200));
        assert!(!m.under_pressure());
        m.admit(img("b", 6, 100, 200));
        assert!(m.under_pressure());
    }

    #[test]
    fn no_pressure_no_eviction() {
        let mut m = ImageGcManager::new(100, default_policy());
        m.admit(img("a", 50, 1000, 2000));
        let now = Utc::now();
        let ev = m.collect(now);
        assert!(ev.is_empty());
    }

    #[test]
    fn evicts_lru_until_target() {
        let mut m = ImageGcManager::new(1000, default_policy());
        // 90% used → above 85 high; target = 80% = 800 bytes
        m.admit(img("old", 200, 5000, 5000));
        m.admit(img("mid", 300, 1000, 5000));
        m.admit(img("new", 400, 100, 5000));
        let now = Utc::now();
        let ev = m.collect(now);
        // Need to drop 100+ bytes; oldest "old" is 200 → enough alone.
        assert_eq!(ev, vec!["old".to_string()]);
        assert!(m.used_bytes() <= 800);
    }

    #[test]
    fn never_evicts_in_use() {
        let mut m = ImageGcManager::new(1000, default_policy());
        m.admit(img("pinned", 600, 9999, 5000));
        m.admit(img("other", 350, 1000, 5000));
        m.mark_in_use("pinned");
        let ev = m.collect(Utc::now());
        assert_eq!(ev, vec!["other".to_string()]);
        assert!(m.images.contains_key("pinned"));
    }

    #[test]
    fn never_evicts_younger_than_min_age() {
        let mut m = ImageGcManager::new(1000, default_policy());
        // Both above pressure but the "fresh" one was pulled 10s ago (< 60s min_age)
        m.admit(img("fresh", 600, 100, 10));
        m.admit(img("stale", 350, 5000, 9999));
        let ev = m.collect(Utc::now());
        assert_eq!(ev, vec!["stale".to_string()]);
        assert!(m.images.contains_key("fresh"));
    }

    #[test]
    fn ties_break_on_size() {
        // Same last_used → larger image wins eviction (more bytes per pass)
        let now = Utc::now();
        let same = now - Duration::seconds(2000);
        let mut m = ImageGcManager::new(1000, default_policy());
        m.admit(ImageRecord {
            id: "small".into(),
            size_bytes: 100,
            last_used_at: same,
            pulled_at: now - Duration::seconds(5000),
            tenant_id: "acme".into(),
        });
        m.admit(ImageRecord {
            id: "big".into(),
            size_bytes: 800,
            last_used_at: same,
            pulled_at: now - Duration::seconds(5000),
            tenant_id: "acme".into(),
        });
        m.admit(img("filler", 60, 100, 5000));
        // used_bytes = 960 → above 85 high; target = 800 → drop 160+
        let ev = m.collect(now);
        assert_eq!(ev[0], "big");
    }

    #[test]
    fn release_unmarks_pin() {
        let mut m = ImageGcManager::new(1000, default_policy());
        m.admit(img("a", 900, 5000, 5000));
        m.mark_in_use("a");
        let _ = m.collect(Utc::now());
        assert!(m.images.contains_key("a"));
        m.release("a");
        let ev = m.collect(Utc::now());
        assert_eq!(ev, vec!["a".to_string()]);
    }
}
