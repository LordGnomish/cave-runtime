// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DORA metrics engine — deployment frequency, lead time, CFR, MTTR ratings + analytics.

use crate::models::{
    Commit, DeployStatus, DeploymentRecord, DoraRating, PullRequest, Sprint,
};

// ── DORA Deployment Frequency Rating ─────────────────────────────────────────
// Thresholds (deploys per day):
//   Elite:  >= 1.0  (multiple deploys per day)
//   High:   >= 1/7  (at least weekly, ≈ 0.143)
//   Medium: >= 1/30 (at least monthly, ≈ 0.033)
//   Low:    < 1/30

pub fn dora_deployment_frequency_rating(deploys_per_day: f64) -> DoraRating {
    if deploys_per_day >= 1.0 {
        DoraRating::Elite
    } else if deploys_per_day >= 1.0 / 7.0 {
        DoraRating::High
    } else if deploys_per_day >= 1.0 / 30.0 {
        DoraRating::Medium
    } else {
        DoraRating::Low
    }
}

// ── DORA Lead Time Rating ─────────────────────────────────────────────────────
// Thresholds (seconds):
//   Elite:  < 1 hour    = 3_600
//   High:   < 1 day     = 86_400
//   Medium: < 1 week    = 604_800
//   Low:    >= 1 week

pub fn dora_lead_time_rating(lead_time_secs: f64) -> DoraRating {
    if lead_time_secs < 3_600.0 {
        DoraRating::Elite
    } else if lead_time_secs < 86_400.0 {
        DoraRating::High
    } else if lead_time_secs < 604_800.0 {
        DoraRating::Medium
    } else {
        DoraRating::Low
    }
}

// ── DORA Change Failure Rate Rating ──────────────────────────────────────────
// Thresholds (percentage 0–100):
//   Elite:  <= 5%
//   High:   <= 10%
//   Medium: <= 30%
//   Low:    > 30%

pub fn dora_cfr_rating(cfr_pct: f64) -> DoraRating {
    if cfr_pct <= 5.0 {
        DoraRating::Elite
    } else if cfr_pct <= 10.0 {
        DoraRating::High
    } else if cfr_pct <= 30.0 {
        DoraRating::Medium
    } else {
        DoraRating::Low
    }
}

// ── DORA MTTR Rating ──────────────────────────────────────────────────────────
// Thresholds (seconds):
//   Elite:  < 1 hour  = 3_600
//   High:   < 1 day   = 86_400
//   Medium: < 1 week  = 604_800
//   Low:    >= 1 week

pub fn dora_mttr_rating(mttr_secs: f64) -> DoraRating {
    if mttr_secs < 3_600.0 {
        DoraRating::Elite
    } else if mttr_secs < 86_400.0 {
        DoraRating::High
    } else if mttr_secs < 604_800.0 {
        DoraRating::Medium
    } else {
        DoraRating::Low
    }
}

// ── Overall DORA Rating ───────────────────────────────────────────────────────
// The overall rating is the minimum (worst) across the four metrics.

pub fn overall_dora_rating(ratings: &[DoraRating]) -> DoraRating {
    ratings
        .iter()
        .min()
        .cloned()
        .unwrap_or(DoraRating::Low)
}

// ── PR Cycle Time ─────────────────────────────────────────────────────────────

/// Average cycle time (seconds) across all PRs that have `cycle_time_secs` set.
pub fn pr_cycle_time_avg_secs(prs: &[PullRequest]) -> f64 {
    let times: Vec<f64> = prs
        .iter()
        .filter_map(|pr| pr.cycle_time_secs)
        .collect();
    if times.is_empty() {
        return 0.0;
    }
    times.iter().sum::<f64>() / times.len() as f64
}

// ── Commit Frequency ─────────────────────────────────────────────────────────

/// Commits-per-day over a rolling `period_days` window.
pub fn commit_frequency_per_day(commits: &[Commit], period_days: u32) -> f64 {
    if commits.is_empty() || period_days == 0 {
        return 0.0;
    }
    commits.len() as f64 / period_days as f64
}

// ── Sprint Velocity ───────────────────────────────────────────────────────────

/// Average completed story-points per sprint (closed sprints only).
pub fn sprint_velocity_avg(sprints: &[Sprint]) -> f64 {
    use crate::models::SprintState;
    let closed: Vec<&Sprint> = sprints
        .iter()
        .filter(|s| s.state == SprintState::Closed)
        .collect();
    if closed.is_empty() {
        return 0.0;
    }
    let total: f64 = closed.iter().map(|s| s.completed_points as f64).sum();
    total / closed.len() as f64
}

// ── Legacy functions (kept for engine.rs backwards compatibility) ─────────────

pub fn deployment_frequency(records: &[DeploymentRecord], days: u32) -> f64 {
    let successful = records
        .iter()
        .filter(|r| r.status == DeployStatus::Success)
        .count();
    successful as f64 / days as f64
}

pub fn change_failure_rate(records: &[DeploymentRecord]) -> f64 {
    if records.is_empty() {
        return 0.0;
    }
    let failures = records
        .iter()
        .filter(|r| r.status == DeployStatus::Failure)
        .count();
    failures as f64 / records.len() as f64
}

pub fn successful_deployments(records: &[DeploymentRecord]) -> Vec<&DeploymentRecord> {
    records
        .iter()
        .filter(|r| r.status == DeployStatus::Success)
        .collect()
}

pub fn average_duration_secs(records: &[DeploymentRecord]) -> Option<f64> {
    let completed: Vec<f64> = records
        .iter()
        .filter_map(|r| {
            r.finished_at
                .map(|end| (end - r.started_at).num_seconds() as f64)
        })
        .collect();
    if completed.is_empty() {
        None
    } else {
        Some(completed.iter().sum::<f64>() / completed.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DeployStatus, DeploymentRecord};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn make_record(status: DeployStatus, duration_secs: Option<i64>) -> DeploymentRecord {
        let started_at = Utc::now();
        let finished_at = duration_secs.map(|d| started_at + Duration::seconds(d));
        DeploymentRecord {
            id: Uuid::new_v4(),
            pipeline: "main".to_string(),
            environment: "prod".to_string(),
            status,
            started_at,
            finished_at,
            commit_sha: "abc123".to_string(),
        }
    }

    #[test]
    fn test_deployment_frequency() {
        let records = vec![
            make_record(DeployStatus::Success, Some(120)),
            make_record(DeployStatus::Success, Some(90)),
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(80)),
            make_record(DeployStatus::Success, Some(70)),
            make_record(DeployStatus::Failure, Some(30)),
        ];
        let freq = deployment_frequency(&records, 5);
        assert!((freq - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_change_failure_rate() {
        let records = vec![
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Failure, Some(30)),
        ];
        let rate = change_failure_rate(&records);
        assert!((rate - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_change_failure_rate_empty() {
        let records: Vec<DeploymentRecord> = vec![];
        assert_eq!(change_failure_rate(&records), 0.0);
    }

    #[test]
    fn test_successful_deployments_filter() {
        let records = vec![
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Failure, Some(30)),
            make_record(DeployStatus::Aborted, None),
            make_record(DeployStatus::Success, Some(90)),
        ];
        let successful = successful_deployments(&records);
        assert_eq!(successful.len(), 2);
        for r in &successful {
            assert_eq!(r.status, DeployStatus::Success);
        }
    }

    #[test]
    fn test_average_duration_secs_none_for_pending() {
        let records = vec![
            make_record(DeployStatus::Success, None),
            make_record(DeployStatus::Failure, None),
        ];
        assert_eq!(average_duration_secs(&records), None);
    }

    #[test]
    fn test_average_duration_secs_calculated() {
        let records = vec![
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(200)),
        ];
        let avg = average_duration_secs(&records).unwrap();
        assert!((avg - 150.0).abs() < 1.0);
    }
}
