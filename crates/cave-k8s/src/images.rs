// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Image + container garbage collection.
//!
//! Mirrors `pkg/kubelet/images/image_gc_manager.go`.  The umbrella
//! layer maintains the in-memory disk-usage view + LRU set; actual
//! image deletion is delegated to `cave-cri`.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRecord {
    pub id: String,
    pub size_bytes: u64,
    /// Number of containers currently using this image.
    pub used_by: u32,
    /// Last time this image was pulled OR a container using it started.
    pub last_used: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcPolicy {
    pub high_threshold_percent: u8,
    pub low_threshold_percent: u8,
    pub min_image_age: Duration,
}

impl Default for GcPolicy {
    fn default() -> Self {
        Self {
            high_threshold_percent: 85,
            low_threshold_percent: 80,
            min_image_age: Duration::from_secs(120),
        }
    }
}

/// Returns the ordered list of images to delete, oldest unused first.
pub fn plan_image_gc(
    policy: &GcPolicy,
    used_bytes: u64,
    capacity_bytes: u64,
    now: SystemTime,
    mut images: Vec<ImageRecord>,
) -> Vec<ImageRecord> {
    if capacity_bytes == 0 {
        return Vec::new();
    }
    let pct = (used_bytes * 100 / capacity_bytes) as u8;
    if pct < policy.high_threshold_percent {
        return Vec::new();
    }
    let target_used = capacity_bytes * policy.low_threshold_percent as u64 / 100;
    let mut deleted = Vec::new();
    let mut running = used_bytes;
    images.sort_by_key(|i| i.last_used);
    for img in images {
        if running <= target_used {
            break;
        }
        if img.used_by > 0 {
            continue;
        }
        if now.duration_since(img.last_used).unwrap_or_default() < policy.min_image_age {
            continue;
        }
        running = running.saturating_sub(img.size_bytes);
        deleted.push(img);
    }
    deleted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(id: &str, sz: u64, used_by: u32, age_secs: u64) -> ImageRecord {
        ImageRecord {
            id: id.into(),
            size_bytes: sz,
            used_by,
            last_used: SystemTime::now() - Duration::from_secs(age_secs),
        }
    }

    #[test]
    fn below_threshold_no_gc() {
        let plan = plan_image_gc(
            &GcPolicy::default(),
            500,
            1000,
            SystemTime::now(),
            vec![img("a", 200, 0, 1000)],
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn above_threshold_evicts_unused_oldest() {
        let images = vec![
            img("old", 100, 0, 1000),
            img("newer", 100, 0, 200),
        ];
        let plan = plan_image_gc(&GcPolicy::default(), 900, 1000, SystemTime::now(), images);
        // need to remove enough to get below 80% = 800 bytes
        assert!(!plan.is_empty());
        assert_eq!(plan[0].id, "old");
    }

    #[test]
    fn images_in_use_are_skipped() {
        let images = vec![
            img("hot", 200, 5, 5000),
            img("cold", 100, 0, 5000),
        ];
        let plan = plan_image_gc(&GcPolicy::default(), 950, 1000, SystemTime::now(), images);
        assert!(!plan.iter().any(|i| i.id == "hot"));
        assert!(plan.iter().any(|i| i.id == "cold"));
    }

    #[test]
    fn young_images_protected() {
        let images = vec![img("young", 200, 0, 30)];
        let plan = plan_image_gc(&GcPolicy::default(), 950, 1000, SystemTime::now(), images);
        assert!(plan.is_empty());
    }

    #[test]
    fn zero_capacity_no_gc() {
        let plan = plan_image_gc(
            &GcPolicy::default(),
            0,
            0,
            SystemTime::now(),
            vec![img("a", 100, 0, 1000)],
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn default_policy_thresholds() {
        let p = GcPolicy::default();
        assert_eq!(p.high_threshold_percent, 85);
        assert_eq!(p.low_threshold_percent, 80);
    }
}
