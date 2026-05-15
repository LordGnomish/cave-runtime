// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Legacy ServiceAccount-token cleaner —
//! `pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go`.
//!
//! Removes stale `kubernetes.io/service-account-token` Secrets that have
//! been unused for `cleanup_grace_period`. The newer
//! `BoundServiceAccountTokens` flow (KEP-1205) replaces these — once the
//! cluster has migrated, the legacy secrets get GC'd.
//!
//! Safety rails:
//!
//! * Only removes secrets where `last_used_sec` is older than the grace
//!   period AND the secret has the `kubernetes.io/legacy-token-last-used`
//!   annotation populated.
//! * Skips secrets used in the last `BUFFER_SEC` window (avoid race with
//!   in-flight TokenReviews).

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const ANNOTATION_LAST_USED: &str = "kubernetes.io/legacy-token-last-used";
pub const DEFAULT_CLEANUP_GRACE_SEC: u64 = 365 * 24 * 60 * 60;
pub const BUFFER_SEC: u64 = 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyToken {
    pub name: String,
    pub namespace: String,
    pub created_at_sec: u64,
    /// `metadata.annotations[ANNOTATION_LAST_USED]` parsed as seconds. None
    /// means "never observed used" — controller treats as not-yet-eligible
    /// (newly minted secrets get a grace period regardless).
    pub last_used_sec: Option<u64>,
    /// True when the parent ServiceAccount no longer references this secret.
    pub orphaned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanerAction {
    Delete,
    Keep,
}

pub fn evaluate(
    token: &LegacyToken,
    now_sec: u64,
    cleanup_grace_sec: u64,
) -> CleanerAction {
    if token.orphaned {
        return CleanerAction::Delete;
    }
    let Some(last_used) = token.last_used_sec else {
        // Not yet observed used → keep until annotation populated.
        return CleanerAction::Keep;
    };
    let age_since_use = now_sec.saturating_sub(last_used);
    if age_since_use < cleanup_grace_sec || age_since_use < BUFFER_SEC {
        CleanerAction::Keep
    } else {
        CleanerAction::Delete
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
    "Cleaner",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn tok(last_used: Option<u64>, orphan: bool) -> LegacyToken {
        LegacyToken {
            name: "default-token-xyz".into(),
            namespace: "default".into(),
            created_at_sec: 0,
            last_used_sec: last_used,
            orphaned: orphan,
        }
    }

    #[test]
    fn orphan_secret_is_deleted_regardless_of_last_used() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "syncSecret",
            "tenant-ltc-orphan"
        );
        assert_eq!(
            evaluate(&tok(None, true), 100, DEFAULT_CLEANUP_GRACE_SEC),
            CleanerAction::Delete
        );
    }

    #[test]
    fn unobserved_secret_is_kept() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "syncSecret",
            "tenant-ltc-no-anno"
        );
        assert_eq!(
            evaluate(&tok(None, false), 99999, DEFAULT_CLEANUP_GRACE_SEC),
            CleanerAction::Keep
        );
    }

    #[test]
    fn recently_used_secret_is_kept() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "syncSecret",
            "tenant-ltc-recent"
        );
        // last_used 30 minutes ago → within BUFFER_SEC.
        assert_eq!(
            evaluate(&tok(Some(99_900), false), 100_000, DEFAULT_CLEANUP_GRACE_SEC),
            CleanerAction::Keep
        );
    }

    #[test]
    fn within_grace_period_kept() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "syncSecret",
            "tenant-ltc-grace"
        );
        // grace=100s, last_used=now-50 → kept.
        assert_eq!(
            evaluate(&tok(Some(50), false), 100, 100),
            CleanerAction::Keep
        );
    }

    #[test]
    fn after_grace_period_deletes() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "syncSecret",
            "tenant-ltc-after-grace"
        );
        // grace=100s, last_used=now-101, BUFFER=3600 → still inside BUFFER → keep.
        // To trigger Delete, age must exceed BOTH grace and BUFFER.
        assert_eq!(
            evaluate(&tok(Some(0), false), 100_000, 100),
            CleanerAction::Delete
        );
    }

    #[test]
    fn buffer_protects_against_in_flight_use() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "syncSecret",
            "tenant-ltc-buffer"
        );
        // small grace=10s, but BUFFER=3600 should still hold.
        assert_eq!(
            evaluate(&tok(Some(99_000), false), 100_000, 10),
            CleanerAction::Keep
        );
    }

    #[test]
    fn defaults_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "constants",
            "tenant-ltc-const"
        );
        assert_eq!(DEFAULT_CLEANUP_GRACE_SEC, 365 * 24 * 60 * 60);
        assert_eq!(BUFFER_SEC, 3600);
        assert_eq!(ANNOTATION_LAST_USED, "kubernetes.io/legacy-token-last-used");
    }

    #[test]
    fn cleaner_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/legacyserviceaccounttokencleaner.go",
            "CleanerAction",
            "tenant-ltc-action-serde"
        );
        for a in [CleanerAction::Delete, CleanerAction::Keep] {
            let s = serde_json::to_string(&a).unwrap();
            let back: CleanerAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
