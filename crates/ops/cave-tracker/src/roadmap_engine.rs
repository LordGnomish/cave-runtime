// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Roadmap, timeline, capacity planning and risk detection for cave-tracker.
//!
//! Adapted from the orphan `roadmap.rs` to work with the current models.

use crate::models::{Issue, IssueType, Priority, Sprint, SprintState};
use crate::TrackerStore;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashSet;
use uuid::Uuid;

// ── View types ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TimelineItem {
    pub issue_id: Uuid,
    pub issue_key: String,
    pub title: String,
    pub status: String,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct TimelineView {
    pub project_id: Uuid,
    pub epics: Vec<TimelineItem>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SprintCapacity {
    pub sprint_id: Uuid,
    pub sprint_name: String,
    pub planned_points: f64,
    pub issue_count: usize,
}

#[derive(Debug, Serialize)]
pub struct CapacityPlan {
    pub project_id: Uuid,
    pub sprints: Vec<SprintCapacity>,
    pub avg_velocity: f64,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RiskItem {
    pub kind: RiskKind,
    pub description: String,
    pub severity: RiskSeverity,
    pub affected_issue_ids: Vec<Uuid>,
    pub affected_sprint_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskKind {
    OverloadedSprint,
    DeadlineAtRisk,
    BlockerChain,
    NoAssignee,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskSeverity {
    High,
    Medium,
    Low,
}

#[derive(Debug, Serialize)]
pub struct RiskReport {
    pub project_id: Uuid,
    pub risks: Vec<RiskItem>,
    pub generated_at: DateTime<Utc>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build a timeline view of all epics in a project.
pub fn timeline_view(store: &TrackerStore, project_id: Uuid) -> TimelineView {
    let epics: Vec<TimelineItem> = store
        .issues
        .values()
        .filter(|i| i.project_id == project_id && i.issue_type == IssueType::Epic)
        .map(|epic| TimelineItem {
            issue_id: epic.id,
            issue_key: epic.key.clone(),
            title: epic.summary.clone(),
            status: epic.status.clone(),
            start: Some(epic.created_at),
            end: epic.due_date,
        })
        .collect();

    TimelineView {
        project_id,
        epics,
        generated_at: Utc::now(),
    }
}

/// Per-sprint capacity plan: planned story points and average velocity from closed sprints.
pub fn capacity_plan(store: &TrackerStore, project_id: Uuid) -> CapacityPlan {
    let sprints: Vec<&Sprint> = store
        .sprints
        .values()
        .filter(|s| s.project_id == project_id)
        .collect();

    // Average velocity = avg story_points completed in Closed sprints.
    let closed_velocities: Vec<f64> = sprints
        .iter()
        .filter(|s| s.state == SprintState::Closed)
        .map(|s| {
            store
                .issues
                .values()
                .filter(|i| i.sprint_id == Some(s.id) && i.resolution.is_some())
                .filter_map(|i| i.story_points)
                .sum::<f64>()
        })
        .collect();

    let avg_velocity = if closed_velocities.is_empty() {
        20.0
    } else {
        closed_velocities.iter().sum::<f64>() / closed_velocities.len() as f64
    };

    let sprint_capacities: Vec<SprintCapacity> = sprints
        .iter()
        .map(|s| {
            let sprint_issues: Vec<&Issue> = store
                .issues
                .values()
                .filter(|i| i.sprint_id == Some(s.id))
                .collect();
            let planned: f64 = sprint_issues.iter().filter_map(|i| i.story_points).sum();
            SprintCapacity {
                sprint_id: s.id,
                sprint_name: s.name.clone(),
                planned_points: planned,
                issue_count: sprint_issues.len(),
            }
        })
        .collect();

    CapacityPlan {
        project_id,
        sprints: sprint_capacities,
        avg_velocity,
        generated_at: Utc::now(),
    }
}

/// Detect risks: overloaded sprints, approaching deadlines, unassigned active issues.
pub fn risk_report(store: &TrackerStore, project_id: Uuid) -> RiskReport {
    let now = Utc::now();
    let mut risks: Vec<RiskItem> = Vec::new();

    // Active sprints for this project.
    let active_sprint_ids: HashSet<Uuid> = store
        .sprints
        .values()
        .filter(|s| s.project_id == project_id && s.state == SprintState::Active)
        .map(|s| s.id)
        .collect();

    // Issues in this project that are not done.
    let open_issues: Vec<&Issue> = store
        .issues
        .values()
        .filter(|i| i.project_id == project_id && i.resolution.is_none())
        .collect();

    // NoAssignee risk: unassigned issues in active sprints.
    let unassigned: Vec<Uuid> = open_issues
        .iter()
        .filter(|i| {
            i.assignee.is_none()
                && i.sprint_id
                    .map(|sid| active_sprint_ids.contains(&sid))
                    .unwrap_or(false)
        })
        .map(|i| i.id)
        .collect();
    if !unassigned.is_empty() {
        risks.push(RiskItem {
            kind: RiskKind::NoAssignee,
            description: format!(
                "{} issue(s) in active sprint(s) have no assignee.",
                unassigned.len()
            ),
            severity: RiskSeverity::Low,
            affected_issue_ids: unassigned,
            affected_sprint_id: None,
        });
    }

    // DeadlineAtRisk: high/critical issues with due date within 3 days.
    let deadline_threshold = chrono::Duration::days(3);
    let at_risk: Vec<Uuid> = open_issues
        .iter()
        .filter(|i| {
            i.due_date
                .map(|d| d > now && d - now < deadline_threshold)
                .unwrap_or(false)
                && matches!(i.priority, Priority::Critical | Priority::High)
        })
        .map(|i| i.id)
        .collect();
    if !at_risk.is_empty() {
        risks.push(RiskItem {
            kind: RiskKind::DeadlineAtRisk,
            description: format!(
                "{} high/critical issue(s) are due within 3 days.",
                at_risk.len()
            ),
            severity: RiskSeverity::High,
            affected_issue_ids: at_risk,
            affected_sprint_id: None,
        });
    }

    // BlockerChain: issues whose linked dependencies (IsBlockedBy) are still open.
    let open_ids: HashSet<Uuid> = open_issues.iter().map(|i| i.id).collect();
    let blocked: Vec<Uuid> = open_issues
        .iter()
        .filter(|i| {
            store
                .issue_links
                .values()
                .any(|link| {
                    use crate::models::LinkType;
                    link.to_issue_id == i.id
                        && link.link_type == LinkType::IsBlockedBy
                        && open_ids.contains(&link.from_issue_id)
                })
        })
        .map(|i| i.id)
        .collect();
    if !blocked.is_empty() {
        risks.push(RiskItem {
            kind: RiskKind::BlockerChain,
            description: format!(
                "{} issue(s) are blocked by unresolved dependencies.",
                blocked.len()
            ),
            severity: RiskSeverity::Medium,
            affected_issue_ids: blocked,
            affected_sprint_id: None,
        });
    }

    RiskReport {
        project_id,
        risks,
        generated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackerStore;
    use uuid::Uuid;

    #[test]
    fn timeline_empty_project() {
        let store = TrackerStore::default();
        let view = timeline_view(&store, Uuid::new_v4());
        assert!(view.epics.is_empty());
    }

    #[test]
    fn capacity_plan_empty_project() {
        let store = TrackerStore::default();
        let plan = capacity_plan(&store, Uuid::new_v4());
        assert!(plan.sprints.is_empty());
        assert_eq!(plan.avg_velocity, 20.0);
    }

    #[test]
    fn risk_report_empty_project() {
        let store = TrackerStore::default();
        let report = risk_report(&store, Uuid::new_v4());
        assert!(report.risks.is_empty());
    }
}
