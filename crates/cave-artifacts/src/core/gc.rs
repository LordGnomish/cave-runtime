// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: pulpcore/app/tasks/orphan.py + goharbor/harbor src/server/middleware/v2auth/...
//
//! Core garbage collector — sweep orphan blobs left behind when their
//! referencing artifacts are deleted.
//!
//! Operates over an abstract inventory: a list of `Blob` records, plus a
//! list of `Reference` records that tie a blob digest to an owning artifact.
//! Returns the set of orphans plus a summary suitable for emitting through
//! the garbage-collection task surface common to Pulp and Harbor.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub digest: String,
    pub size: u64,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub digest: String,
    pub artifact_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GcReport {
    pub total_blobs: usize,
    pub referenced_blobs: usize,
    pub orphans: Vec<Blob>,
    pub reclaimable_bytes: u64,
}

impl GcReport {
    pub fn orphan_count(&self) -> usize {
        self.orphans.len()
    }
    pub fn orphan_ratio(&self) -> f64 {
        if self.total_blobs == 0 {
            0.0
        } else {
            self.orphans.len() as f64 / self.total_blobs as f64
        }
    }
}

/// Configuration for a sweep. `grace_seconds` lets recently-created blobs
/// stay alive even if no reference points at them yet — protects in-flight
/// uploads against an aggressive garbage collector.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GcConfig {
    pub grace_seconds: i64,
    pub max_orphans: Option<usize>,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            grace_seconds: 600,
            max_orphans: None,
        }
    }
}

pub fn sweep(blobs: &[Blob], refs: &[Reference], cfg: &GcConfig) -> GcReport {
    sweep_at(blobs, refs, cfg, Utc::now())
}

pub fn sweep_at(
    blobs: &[Blob],
    refs: &[Reference],
    cfg: &GcConfig,
    now: DateTime<Utc>,
) -> GcReport {
    let referenced: HashSet<&str> = refs.iter().map(|r| r.digest.as_str()).collect();
    let mut orphans = Vec::new();
    let mut reclaimable: u64 = 0;
    for b in blobs {
        if referenced.contains(b.digest.as_str()) {
            continue;
        }
        let age = now.signed_duration_since(b.created).num_seconds();
        if age < cfg.grace_seconds {
            continue;
        }
        if let Some(max) = cfg.max_orphans {
            if orphans.len() >= max {
                break;
            }
        }
        reclaimable += b.size;
        orphans.push(b.clone());
    }
    GcReport {
        total_blobs: blobs.len(),
        referenced_blobs: blobs.len() - orphans_for_count(blobs, &referenced, cfg, now),
        orphans,
        reclaimable_bytes: reclaimable,
    }
}

fn orphans_for_count(
    blobs: &[Blob],
    referenced: &HashSet<&str>,
    cfg: &GcConfig,
    now: DateTime<Utc>,
) -> usize {
    let mut n = 0;
    for b in blobs {
        if !referenced.contains(b.digest.as_str())
            && now.signed_duration_since(b.created).num_seconds() >= cfg.grace_seconds
        {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn blob(digest: &str, size: u64, age_secs: i64) -> Blob {
        Blob {
            digest: digest.to_string(),
            size,
            created: Utc::now() - Duration::seconds(age_secs),
        }
    }

    #[test]
    fn referenced_blobs_never_orphan() {
        let blobs = vec![blob("a", 100, 3600), blob("b", 200, 3600)];
        let refs = vec![
            Reference {
                digest: "a".into(),
                artifact_id: "art-1".into(),
            },
            Reference {
                digest: "b".into(),
                artifact_id: "art-2".into(),
            },
        ];
        let r = sweep(&blobs, &refs, &GcConfig::default());
        assert_eq!(r.orphans.len(), 0);
        assert_eq!(r.total_blobs, 2);
        assert_eq!(r.referenced_blobs, 2);
    }

    #[test]
    fn unreferenced_blob_collected() {
        let blobs = vec![blob("a", 100, 3600), blob("b", 200, 3600)];
        let refs = vec![Reference {
            digest: "a".into(),
            artifact_id: "art-1".into(),
        }];
        let r = sweep(&blobs, &refs, &GcConfig::default());
        assert_eq!(r.orphans.len(), 1);
        assert_eq!(r.orphans[0].digest, "b");
        assert_eq!(r.reclaimable_bytes, 200);
    }

    #[test]
    fn grace_window_protects_new_blob() {
        let blobs = vec![blob("a", 100, 60)];
        let refs = vec![];
        let cfg = GcConfig {
            grace_seconds: 600,
            max_orphans: None,
        };
        let r = sweep(&blobs, &refs, &cfg);
        assert!(r.orphans.is_empty());
    }

    #[test]
    fn grace_zero_collects_immediately() {
        let blobs = vec![blob("a", 100, 0)];
        let refs = vec![];
        let cfg = GcConfig {
            grace_seconds: 0,
            max_orphans: None,
        };
        let r = sweep(&blobs, &refs, &cfg);
        assert_eq!(r.orphans.len(), 1);
    }

    #[test]
    fn max_orphans_limits_batch() {
        let blobs = vec![
            blob("a", 10, 3600),
            blob("b", 20, 3600),
            blob("c", 30, 3600),
        ];
        let cfg = GcConfig {
            grace_seconds: 0,
            max_orphans: Some(2),
        };
        let r = sweep(&blobs, &[], &cfg);
        assert_eq!(r.orphans.len(), 2);
        assert_eq!(r.reclaimable_bytes, 30);
    }

    #[test]
    fn orphan_ratio_zero_when_empty() {
        let r = sweep(&[], &[], &GcConfig::default());
        assert_eq!(r.orphan_ratio(), 0.0);
    }

    #[test]
    fn orphan_ratio_half_when_half_orphan() {
        let blobs = vec![blob("a", 1, 3600), blob("b", 1, 3600)];
        let refs = vec![Reference {
            digest: "a".into(),
            artifact_id: "k".into(),
        }];
        let r = sweep(&blobs, &refs, &GcConfig::default());
        assert!((r.orphan_ratio() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn reclaimable_bytes_sums_orphan_sizes() {
        let blobs = vec![blob("a", 100, 3600), blob("b", 200, 3600), blob("c", 50, 3600)];
        let refs = vec![Reference {
            digest: "b".into(),
            artifact_id: "art".into(),
        }];
        let r = sweep(&blobs, &refs, &GcConfig::default());
        assert_eq!(r.reclaimable_bytes, 150);
    }

    #[test]
    fn sweep_at_uses_supplied_clock() {
        let created = Utc::now() - Duration::seconds(30);
        let blobs = vec![Blob {
            digest: "x".into(),
            size: 1,
            created,
        }];
        let later = Utc::now() + Duration::seconds(3600);
        let cfg = GcConfig {
            grace_seconds: 60,
            max_orphans: None,
        };
        let r = sweep_at(&blobs, &[], &cfg, later);
        assert_eq!(r.orphans.len(), 1);
    }

    #[test]
    fn report_serde_roundtrip() {
        let r = GcReport {
            total_blobs: 3,
            referenced_blobs: 1,
            orphans: vec![blob("a", 1, 0)],
            reclaimable_bytes: 1,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: GcReport = serde_json::from_str(&j).unwrap();
        assert_eq!(back.total_blobs, 3);
        assert_eq!(back.orphans.len(), 1);
    }
}
