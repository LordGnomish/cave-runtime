// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Roadmap and capacity planning: timeline, dependencies, risk detection.

use crate::models::*;
use crate::TrackerState;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ── View types ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TimelineItem {
    pub issue_id: Uuid,
    pub issue_key: String,
    pub title: String,
    pub issue_type: IssueType,
    pub status: String,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub milestone_id: Option<Uuid>,
    pub dependencies: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct TimelineView {
    pub project_id: Uuid,
    pub epics: Vec<TimelineItem>,
    pub milestones: Vec<Milestone>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SprintCapacity {
    pub sprint_id: Uuid,
    pub sprint_name: String,
    pub planned_points: f32,
    pub capacity_points: f32,
    pub load_pct: f32,
    pub issue_count: usize,
}

#[derive(Debug, Serialize)]
pub struct CapacityPlan {
    pub project_id: Uuid,
    pub sprints: Vec<SprintCapacity>,
    pub avg_velocity: f32,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DependencyNode {
    pub issue_id: Uuid,
    pub issue_key: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct DependencyEdge {
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub blocking: bool,
}

#[derive(Debug, Serialize)]
pub struct DependencyGraph {
    pub project_id: Uuid,
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
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

// ── Roadmap functions ─────────────────────────────────────────────────────────

/// Timeline view of all epics and milestones for a project.
pub async fn timeline_view(state: &TrackerState, project_id: Uuid) -> TimelineView {
    let epics: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id && i.issue_type == IssueType::Epic)
            .cloned()
            .collect()
    };

    let roadmap = {
        let roadmaps = state.roadmaps.lock().await;
        roadmaps.values().find(|r| r.project_id == project_id).cloned()
    };

    let milestones = roadmap.map(|r| r.milestones).unwrap_or_default();

    // Build a set of milestone-epic mappings for annotation.
    let epic_milestone: HashMap<Uuid, Uuid> = milestones.iter()
        .flat_map(|m| m.epic_ids.iter().map(move |eid| (*eid, m.id)))
        .collect();

    let items: Vec<TimelineItem> = epics.iter().map(|epic| TimelineItem {
        issue_id: epic.id,
        issue_key: epic.key.clone(),
        title: epic.title.clone(),
        issue_type: epic.issue_type.clone(),
        status: epic.status.clone(),
        start: Some(epic.created_at),
        end: epic.due_date,
        milestone_id: epic_milestone.get(&epic.id).copied(),
        dependencies: epic.dependencies.clone(),
    }).collect();

    TimelineView {
        project_id,
        epics: items,
        milestones,
        generated_at: Utc::now(),
    }
}

/// Per-sprint capacity plan: planned story points vs average velocity.
pub async fn capacity_planning(state: &TrackerState, project_id: Uuid) -> CapacityPlan {
    let sprints: Vec<Sprint> = {
        let sprints = state.sprints.lock().await;
        sprints.values()
            .filter(|s| s.project_id == project_id)
            .cloned()
            .collect()
    };

    let issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id)
            .cloned()
            .collect()
    };

    // Velocity: average story points completed in done sprints.
    let completed_velocities: Vec<f32> = sprints.iter()
        .filter(|s| s.status == SprintStatus::Completed)
        .map(|s| {
            issues.iter()
                .filter(|i| i.sprint_id == Some(s.id) && i.status == "Done")
                .filter_map(|i| i.story_points)
                .sum()
        })
        .collect();

    let avg_velocity = if completed_velocities.is_empty() {
        20.0f32
    } else {
        completed_velocities.iter().sum::<f32>() / completed_velocities.len() as f32
    };

    let sprint_capacities: Vec<SprintCapacity> = sprints.iter().map(|s| {
        let sprint_issues: Vec<&Issue> = issues.iter()
            .filter(|i| i.sprint_id == Some(s.id))
            .collect();
        let planned: f32 = sprint_issues.iter().filter_map(|i| i.story_points).sum();
        let load_pct = if avg_velocity > 0.0 { (planned / avg_velocity) * 100.0 } else { 0.0 };
        SprintCapacity {
            sprint_id: s.id,
            sprint_name: s.name.clone(),
            planned_points: planned,
            capacity_points: avg_velocity,
            load_pct,
            issue_count: sprint_issues.len(),
        }
    }).collect();

    CapacityPlan {
        project_id,
        sprints: sprint_capacities,
        avg_velocity,
        generated_at: Utc::now(),
    }
}

/// Build a dependency graph (nodes + directed edges) for a project.
pub async fn dependency_graph(state: &TrackerState, project_id: Uuid) -> DependencyGraph {
    let issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id)
            .cloned()
            .collect()
    };

    // Only include issues that are part of at least one dependency relationship.
    let mut node_ids: HashSet<Uuid> = HashSet::new();
    let mut edges: Vec<DependencyEdge> = Vec::new();

    for issue in &issues {
        for dep_id in &issue.dependencies {
            node_ids.insert(issue.id);
            node_ids.insert(*dep_id);
            let blocking = issues.iter()
                .find(|i| i.id == *dep_id)
                .map(|d| d.status != "Done")
                .unwrap_or(false);
            edges.push(DependencyEdge {
                from_id: issue.id,
                to_id: *dep_id,
                blocking,
            });
        }
    }

    let nodes: Vec<DependencyNode> = issues.iter()
        .filter(|i| node_ids.contains(&i.id))
        .map(|i| DependencyNode {
            issue_id: i.id,
            issue_key: i.key.clone(),
            title: i.title.clone(),
            status: i.status.clone(),
        })
        .collect();

    DependencyGraph { project_id, nodes, edges }
}

/// Detect risks: overloaded sprints, upcoming deadlines, unassigned issues, blocker chains.
pub async fn risk_detection(state: &TrackerState, project_id: Uuid) -> RiskReport {
    let sprints: Vec<Sprint> = {
        let sprints = state.sprints.lock().await;
        sprints.values()
            .filter(|s| s.project_id == project_id && s.status != SprintStatus::Completed)
            .cloned()
            .collect()
    };

    let issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id && i.status != "Done")
            .cloned()
            .collect()
    };

    let avg_velocity = capacity_planning(state, project_id).await.avg_velocity;

    let now = Utc::now();
    let mut risks: Vec<RiskItem> = Vec::new();

    // Overloaded sprints (> 120 % of velocity).
    for sprint in &sprints {
        let planned: f32 = issues.iter()
            .filter(|i| i.sprint_id == Some(sprint.id))
            .filter_map(|i| i.story_points)
            .sum();
        if avg_velocity > 0.0 && planned > avg_velocity * 1.2 {
            let affected: Vec<Uuid> = issues.iter()
                .filter(|i| i.sprint_id == Some(sprint.id))
                .map(|i| i.id)
                .collect();
            risks.push(RiskItem {
                kind: RiskKind::OverloadedSprint,
                description: format!(
                    "Sprint '{}' has {:.1} points planned vs {:.1} avg velocity ({:.0}% load).",
                    sprint.name, planned, avg_velocity, (planned / avg_velocity) * 100.0
                ),
                severity: if planned > avg_velocity * 1.5 { RiskSeverity::High } else { RiskSeverity::Medium },
                affected_issue_ids: affected,
                affected_sprint_id: Some(sprint.id),
            });
        }
    }

    // Deadline risk: high-priority issues with due date within 3 days.
    let deadline_threshold = chrono::Duration::days(3);
    let at_risk: Vec<&Issue> = issues.iter()
        .filter(|i| {
            i.due_date.map(|d| d - now < deadline_threshold && d > now).unwrap_or(false)
                && matches!(i.priority, Priority::Critical | Priority::High)
        })
        .collect();
    if !at_risk.is_empty() {
        risks.push(RiskItem {
            kind: RiskKind::DeadlineAtRisk,
            description: format!(
                "{} high/critical issue(s) are due within 3 days.",
                at_risk.len()
            ),
            severity: RiskSeverity::High,
            affected_issue_ids: at_risk.iter().map(|i| i.id).collect(),
            affected_sprint_id: None,
        });
    }

    // Blocker chains.
    let issue_statuses: HashMap<Uuid, &str> = issues.iter().map(|i| (i.id, i.status.as_str())).collect();
    let blocked: Vec<&Issue> = issues.iter()
        .filter(|i| i.dependencies.iter().any(|dep| issue_statuses.get(dep).map(|s| *s != "Done").unwrap_or(false)))
        .collect();
    if !blocked.is_empty() {
        risks.push(RiskItem {
            kind: RiskKind::BlockerChain,
            description: format!("{} issue(s) are blocked by unresolved dependencies.", blocked.len()),
            severity: RiskSeverity::Medium,
            affected_issue_ids: blocked.iter().map(|i| i.id).collect(),
            affected_sprint_id: None,
        });
    }

    // Unassigned issues in active sprints.
    let active_sprint_ids: HashSet<Uuid> = sprints.iter()
        .filter(|s| s.status == SprintStatus::Active)
        .map(|s| s.id)
        .collect();
    let unassigned: Vec<&Issue> = issues.iter()
        .filter(|i| i.assignee.is_none() && i.sprint_id.map(|sid| active_sprint_ids.contains(&sid)).unwrap_or(false))
        .collect();
    if !unassigned.is_empty() {
        risks.push(RiskItem {
            kind: RiskKind::NoAssignee,
            description: format!("{} issue(s) in active sprint(s) have no assignee.", unassigned.len()),
            severity: RiskSeverity::Low,
            affected_issue_ids: unassigned.iter().map(|i| i.id).collect(),
            affected_sprint_id: None,
        });
    }

    RiskReport { project_id, risks, generated_at: now }
}
