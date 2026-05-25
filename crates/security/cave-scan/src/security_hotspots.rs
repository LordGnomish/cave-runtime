// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Security Hotspots — parity with
//! `server/sonar-webserver-webapi/src/main/java/org/sonar/server/hotspot/ws/`
//! (SonarQube v10.4.1).
//!
//! Security Hotspots are findings that *might* be security-sensitive
//! and need human review — distinct from Issues which are confirmed
//! defects. Each hotspot has a review status (TO_REVIEW → REVIEWED)
//! and a resolution (SAFE / FIXED / ACKNOWLEDGED).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HotspotStatus {
    ToReview,
    Reviewed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HotspotResolution {
    Fixed,
    Safe,
    Acknowledged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum HotspotProbability {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityHotspot {
    pub key: Uuid,
    pub rule_key: String,
    pub file_path: String,
    pub line_number: usize,
    pub message: String,
    pub vulnerability_probability: HotspotProbability,
    pub status: HotspotStatus,
    pub resolution: Option<HotspotResolution>,
    pub assignee: Option<String>,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HotspotError {
    #[error("hotspot is already reviewed; status transition rejected")]
    AlreadyReviewed,
    #[error("REVIEWED status requires a resolution (SAFE/FIXED/ACKNOWLEDGED)")]
    ResolutionRequired,
    #[error("TO_REVIEW status must not carry a resolution")]
    ResolutionForbiddenForToReview,
}

impl SecurityHotspot {
    pub fn new_to_review(
        rule_key: impl Into<String>,
        file_path: impl Into<String>,
        line_number: usize,
        message: impl Into<String>,
        prob: HotspotProbability,
    ) -> Self {
        Self {
            key: Uuid::new_v4(),
            rule_key: rule_key.into(),
            file_path: file_path.into(),
            line_number,
            message: message.into(),
            vulnerability_probability: prob,
            status: HotspotStatus::ToReview,
            resolution: None,
            assignee: None,
            comment: None,
            created_at: Utc::now(),
        }
    }

    /// Transition TO_REVIEW → REVIEWED with the given resolution.
    pub fn mark_reviewed(
        &mut self,
        resolution: HotspotResolution,
        assignee: Option<String>,
        comment: Option<String>,
    ) -> Result<(), HotspotError> {
        if matches!(self.status, HotspotStatus::Reviewed) {
            return Err(HotspotError::AlreadyReviewed);
        }
        self.status = HotspotStatus::Reviewed;
        self.resolution = Some(resolution);
        self.assignee = assignee.or_else(|| self.assignee.take());
        self.comment = comment.or_else(|| self.comment.take());
        Ok(())
    }

    /// Reopen — only allowed if currently REVIEWED.
    pub fn reopen(&mut self) -> Result<(), HotspotError> {
        if matches!(self.status, HotspotStatus::ToReview) {
            // already to-review — no-op
            return Ok(());
        }
        self.status = HotspotStatus::ToReview;
        self.resolution = None;
        Ok(())
    }

    /// Validate that the (status, resolution) pair is internally consistent.
    pub fn validate(&self) -> Result<(), HotspotError> {
        match (&self.status, &self.resolution) {
            (HotspotStatus::Reviewed, None) => Err(HotspotError::ResolutionRequired),
            (HotspotStatus::ToReview, Some(_)) => Err(HotspotError::ResolutionForbiddenForToReview),
            _ => Ok(()),
        }
    }
}

/// Aggregate stats for the gate `SecurityHotspotsReviewedPct` metric.
pub fn reviewed_pct(hotspots: &[SecurityHotspot]) -> f64 {
    if hotspots.is_empty() {
        return 100.0;
    }
    let reviewed = hotspots
        .iter()
        .filter(|h| matches!(h.status, HotspotStatus::Reviewed))
        .count() as f64;
    100.0 * reviewed / hotspots.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hot() -> SecurityHotspot {
        SecurityHotspot::new_to_review(
            "S2076",
            "src/runner.rs",
            42,
            "Make sure this OS command is safe.",
            HotspotProbability::High,
        )
    }

    #[test]
    fn new_hotspot_is_to_review_no_resolution() {
        let h = hot();
        assert_eq!(h.status, HotspotStatus::ToReview);
        assert!(h.resolution.is_none());
        assert!(h.validate().is_ok());
    }

    #[test]
    fn mark_reviewed_transitions_status() {
        let mut h = hot();
        h.mark_reviewed(HotspotResolution::Safe, Some("alice".into()), None)
            .unwrap();
        assert_eq!(h.status, HotspotStatus::Reviewed);
        assert_eq!(h.resolution, Some(HotspotResolution::Safe));
        assert_eq!(h.assignee.as_deref(), Some("alice"));
        assert!(h.validate().is_ok());
    }

    #[test]
    fn double_review_rejected() {
        let mut h = hot();
        h.mark_reviewed(HotspotResolution::Safe, None, None).unwrap();
        let err = h
            .mark_reviewed(HotspotResolution::Fixed, None, None)
            .unwrap_err();
        assert_eq!(err, HotspotError::AlreadyReviewed);
    }

    #[test]
    fn reopen_clears_resolution() {
        let mut h = hot();
        h.mark_reviewed(HotspotResolution::Fixed, None, None).unwrap();
        h.reopen().unwrap();
        assert_eq!(h.status, HotspotStatus::ToReview);
        assert!(h.resolution.is_none());
    }

    #[test]
    fn reopen_when_already_to_review_is_noop() {
        let mut h = hot();
        let before = h.clone();
        h.reopen().unwrap();
        assert_eq!(h.status, before.status);
    }

    #[test]
    fn validate_reviewed_without_resolution_errors() {
        let mut h = hot();
        h.status = HotspotStatus::Reviewed;
        assert_eq!(h.validate(), Err(HotspotError::ResolutionRequired));
    }

    #[test]
    fn validate_to_review_with_resolution_errors() {
        let mut h = hot();
        h.resolution = Some(HotspotResolution::Safe);
        assert_eq!(
            h.validate(),
            Err(HotspotError::ResolutionForbiddenForToReview)
        );
    }

    #[test]
    fn reviewed_pct_empty_is_100() {
        assert_eq!(reviewed_pct(&[]), 100.0);
    }

    #[test]
    fn reviewed_pct_mixed() {
        let mut a = hot();
        a.mark_reviewed(HotspotResolution::Safe, None, None).unwrap();
        let b = hot();
        let pct = reviewed_pct(&[a, b]);
        assert_eq!(pct, 50.0);
    }

    #[test]
    fn probability_serde_uppercase() {
        let s = serde_json::to_string(&HotspotProbability::High).unwrap();
        assert_eq!(s, "\"HIGH\"");
    }

    #[test]
    fn status_serde_screaming_snake() {
        let s = serde_json::to_string(&HotspotStatus::ToReview).unwrap();
        assert_eq!(s, "\"TO_REVIEW\"");
    }
}
