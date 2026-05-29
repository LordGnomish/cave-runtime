// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for DORA engine rating functions — written BEFORE implementation (TDD).

use cave_devlake::engine::{
    dora_cfr_rating, dora_deployment_frequency_rating, dora_lead_time_rating, dora_mttr_rating,
    overall_dora_rating, pr_cycle_time_avg_secs, commit_frequency_per_day, sprint_velocity_avg,
};
use cave_devlake::models::{Commit, DoraRating, Issue, IssueStatus, IssueType, PrState, PullRequest, Sprint, SprintState};
use chrono::{Duration, Utc};
use uuid::Uuid;

// ── Deployment frequency ratings ──────────────────────────────────────────────
// DevLake/DORA thresholds:
//   Elite:  >= 1.0/day (multiple deploys per day)
//   High:   >= 1/week = 1/7 per day ≈ 0.143
//   Medium: >= 1/month = 1/30 per day ≈ 0.033
//   Low:    < 1/month

#[test]
fn df_elite_multiple_per_day() {
    assert_eq!(dora_deployment_frequency_rating(3.0), DoraRating::Elite);
}

#[test]
fn df_elite_exactly_one_per_day() {
    assert_eq!(dora_deployment_frequency_rating(1.0), DoraRating::Elite);
}

#[test]
fn df_high_weekly() {
    // ~1 per week = 1/7 ≈ 0.143
    assert_eq!(dora_deployment_frequency_rating(0.143), DoraRating::High);
}

#[test]
fn df_medium_monthly() {
    // ~1 per month = 1/30 ≈ 0.033
    assert_eq!(dora_deployment_frequency_rating(0.05), DoraRating::Medium);
}

#[test]
fn df_low_rare() {
    assert_eq!(dora_deployment_frequency_rating(0.01), DoraRating::Low);
}

// ── Lead time ratings ─────────────────────────────────────────────────────────
// DevLake/DORA thresholds (in seconds):
//   Elite:  < 1 hour  = 3_600
//   High:   < 1 day   = 86_400
//   Medium: < 1 week  = 604_800
//   Low:    >= 1 week

#[test]
fn lt_elite_under_one_hour() {
    assert_eq!(dora_lead_time_rating(3_000.0), DoraRating::Elite);
}

#[test]
fn lt_high_under_one_day() {
    assert_eq!(dora_lead_time_rating(43_200.0), DoraRating::High);
}

#[test]
fn lt_medium_under_one_week() {
    assert_eq!(dora_lead_time_rating(259_200.0), DoraRating::Medium);
}

#[test]
fn lt_low_over_one_week() {
    assert_eq!(dora_lead_time_rating(700_000.0), DoraRating::Low);
}

// ── Change failure rate ratings ───────────────────────────────────────────────
// DevLake/DORA thresholds (percentage):
//   Elite:  <= 5%
//   High:   <= 10%
//   Medium: <= 30%
//   Low:    > 30%

#[test]
fn cfr_elite_under_5pct() {
    assert_eq!(dora_cfr_rating(3.0), DoraRating::Elite);
}

#[test]
fn cfr_high_under_10pct() {
    assert_eq!(dora_cfr_rating(8.0), DoraRating::High);
}

#[test]
fn cfr_medium_under_30pct() {
    assert_eq!(dora_cfr_rating(20.0), DoraRating::Medium);
}

#[test]
fn cfr_low_over_30pct() {
    assert_eq!(dora_cfr_rating(45.0), DoraRating::Low);
}

// ── MTTR ratings ──────────────────────────────────────────────────────────────
// DevLake/DORA thresholds (in seconds):
//   Elite:  < 1 hour  = 3_600
//   High:   < 1 day   = 86_400
//   Medium: < 1 week  = 604_800
//   Low:    >= 1 week

#[test]
fn mttr_elite_under_one_hour() {
    assert_eq!(dora_mttr_rating(1_800.0), DoraRating::Elite);
}

#[test]
fn mttr_high_under_one_day() {
    assert_eq!(dora_mttr_rating(50_000.0), DoraRating::High);
}

#[test]
fn mttr_medium_under_one_week() {
    assert_eq!(dora_mttr_rating(300_000.0), DoraRating::Medium);
}

#[test]
fn mttr_low_over_one_week() {
    assert_eq!(dora_mttr_rating(700_000.0), DoraRating::Low);
}

// ── Overall DORA rating ───────────────────────────────────────────────────────

#[test]
fn overall_rating_is_minimum_of_four() {
    let ratings = [
        DoraRating::Elite,
        DoraRating::High,
        DoraRating::Medium,
        DoraRating::Low,
    ];
    assert_eq!(overall_dora_rating(&ratings), DoraRating::Low);
}

#[test]
fn overall_rating_all_elite() {
    let ratings = [
        DoraRating::Elite,
        DoraRating::Elite,
        DoraRating::Elite,
        DoraRating::Elite,
    ];
    assert_eq!(overall_dora_rating(&ratings), DoraRating::Elite);
}

#[test]
fn overall_rating_mixed_high_medium() {
    let ratings = [
        DoraRating::High,
        DoraRating::High,
        DoraRating::Medium,
        DoraRating::High,
    ];
    assert_eq!(overall_dora_rating(&ratings), DoraRating::Medium);
}

// ── PR cycle time ─────────────────────────────────────────────────────────────

#[test]
fn pr_cycle_time_avg_empty_returns_zero() {
    let prs: Vec<PullRequest> = vec![];
    assert_eq!(pr_cycle_time_avg_secs(&prs), 0.0);
}

#[test]
fn pr_cycle_time_avg_merged_only() {
    let now = Utc::now();
    let prs = vec![
        PullRequest {
            id: Uuid::new_v4(),
            number: 1,
            title: "PR 1".to_string(),
            author: "alice".to_string(),
            source_branch: "feature/a".to_string(),
            target_branch: "main".to_string(),
            state: PrState::Merged,
            created_at: now - Duration::hours(4),
            merged_at: Some(now),
            closed_at: None,
            cycle_time_secs: Some(14400.0),
            review_count: 1,
            comment_count: 2,
            additions: 50,
            deletions: 10,
        },
        PullRequest {
            id: Uuid::new_v4(),
            number: 2,
            title: "PR 2".to_string(),
            author: "bob".to_string(),
            source_branch: "feature/b".to_string(),
            target_branch: "main".to_string(),
            state: PrState::Merged,
            created_at: now - Duration::hours(2),
            merged_at: Some(now),
            closed_at: None,
            cycle_time_secs: Some(7200.0),
            review_count: 1,
            comment_count: 0,
            additions: 20,
            deletions: 5,
        },
    ];
    let avg = pr_cycle_time_avg_secs(&prs);
    assert!((avg - 10800.0).abs() < 1.0, "Expected ~10800 got {avg}");
}

// ── Commit frequency ──────────────────────────────────────────────────────────

#[test]
fn commit_frequency_empty_is_zero() {
    let commits: Vec<Commit> = vec![];
    assert_eq!(commit_frequency_per_day(&commits, 30), 0.0);
}

#[test]
fn commit_frequency_calculated() {
    let now = Utc::now();
    let commits: Vec<Commit> = (0..10)
        .map(|i| Commit {
            sha: format!("sha{i}"),
            author: "alice".to_string(),
            message: format!("commit {i}"),
            committed_at: now - Duration::days(i),
            additions: 5,
            deletions: 2,
            files_changed: 1,
        })
        .collect();
    let freq = commit_frequency_per_day(&commits, 10);
    assert!((freq - 1.0).abs() < 0.01, "Expected 1.0/day got {freq}");
}

// ── Sprint velocity ────────────────────────────────────────────────────────────

#[test]
fn sprint_velocity_empty_is_zero() {
    let sprints: Vec<Sprint> = vec![];
    assert_eq!(sprint_velocity_avg(&sprints), 0.0);
}

#[test]
fn sprint_velocity_avg_computed() {
    let now = Utc::now();
    let sprints = vec![
        Sprint {
            id: Uuid::new_v4(),
            name: "Sprint 1".to_string(),
            state: SprintState::Closed,
            start_date: now - Duration::days(28),
            end_date: now - Duration::days(14),
            completed_points: 20,
            planned_points: 25,
            completed_issues: 8,
            planned_issues: 10,
        },
        Sprint {
            id: Uuid::new_v4(),
            name: "Sprint 2".to_string(),
            state: SprintState::Closed,
            start_date: now - Duration::days(14),
            end_date: now,
            completed_points: 30,
            planned_points: 30,
            completed_issues: 10,
            planned_issues: 10,
        },
    ];
    let avg = sprint_velocity_avg(&sprints);
    assert!((avg - 25.0).abs() < 0.01, "Expected avg 25 completed points, got {avg}");
}
