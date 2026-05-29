// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sprint velocity, cycle time, lead time and triage analytics for cave-tracker.
//!
//! Adapted from the orphan `tracker.rs` to work with the current models.

use crate::models::{Issue, Priority, SprintState};
use crate::TrackerStore;
use serde::Serialize;
use uuid::Uuid;

// ── Result types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SprintVelocityRecord {
    pub sprint_id: Uuid,
    pub sprint_name: String,
    /// Story points of issues with `resolution.is_some()` in this sprint.
    pub completed_points: f64,
    /// Total story points planned for the sprint.
    pub planned_points: f64,
}

#[derive(Debug, Serialize)]
pub struct CycleTimeStats {
    /// Number of resolved issues used for the calculation.
    pub count: usize,
    /// Average hours from `created_at` to `resolved_at` for resolved issues.
    pub avg_hours: f64,
    /// Minimum hours (0 if no issues).
    pub min_hours: f64,
    /// Maximum hours (0 if no issues).
    pub max_hours: f64,
}

#[derive(Debug, Serialize)]
pub struct TriageSuggestion {
    pub issue_id: Uuid,
    pub suggested_priority: Priority,
    pub suggested_labels: Vec<String>,
    pub reasoning: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Velocity per closed sprint in a project.
pub fn sprint_velocity(store: &TrackerStore, project_id: Uuid) -> Vec<SprintVelocityRecord> {
    store
        .sprints
        .values()
        .filter(|s| s.project_id == project_id && s.state == SprintState::Closed)
        .map(|sprint| {
            let sprint_issues: Vec<&Issue> = store
                .issues
                .values()
                .filter(|i| i.sprint_id == Some(sprint.id))
                .collect();
            let planned: f64 = sprint_issues.iter().filter_map(|i| i.story_points).sum();
            let completed: f64 = sprint_issues
                .iter()
                .filter(|i| i.resolution.is_some())
                .filter_map(|i| i.story_points)
                .sum();
            SprintVelocityRecord {
                sprint_id: sprint.id,
                sprint_name: sprint.name.clone(),
                completed_points: completed,
                planned_points: planned,
            }
        })
        .collect()
}

/// Cycle time statistics for resolved issues in a project.
/// Uses `resolved_at - created_at` as a proxy since we don't have per-field event logs.
pub fn cycle_time_stats(store: &TrackerStore, project_id: Uuid) -> CycleTimeStats {
    let cycle_times: Vec<f64> = store
        .issues
        .values()
        .filter(|i| i.project_id == project_id && i.resolved_at.is_some())
        .filter_map(|i| {
            i.resolved_at.map(|resolved| {
                let hours = (resolved - i.created_at).num_minutes() as f64 / 60.0;
                hours.max(0.0)
            })
        })
        .collect();

    if cycle_times.is_empty() {
        return CycleTimeStats {
            count: 0,
            avg_hours: 0.0,
            min_hours: 0.0,
            max_hours: 0.0,
        };
    }

    let sum: f64 = cycle_times.iter().sum();
    let min = cycle_times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = cycle_times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    CycleTimeStats {
        count: cycle_times.len(),
        avg_hours: sum / cycle_times.len() as f64,
        min_hours: min,
        max_hours: max,
    }
}

/// Heuristic triage: infer priority and labels from issue title + description keywords.
/// In production this would call an LLM via cave-local-llm or cave-llm-gateway.
pub fn triage_issue(store: &TrackerStore, issue_id: Uuid) -> Option<TriageSuggestion> {
    let issue = store.issues.get(&issue_id)?;

    let combined = format!(
        "{} {}",
        issue.summary.to_lowercase(),
        issue.description.as_deref().unwrap_or("").to_lowercase()
    );

    // Priority heuristics.
    let priority = if combined.contains("critical")
        || combined.contains("outage")
        || combined.contains("production down")
        || combined.contains("p0")
    {
        Priority::Critical
    } else if combined.contains("urgent")
        || combined.contains("security")
        || combined.contains("data loss")
        || combined.contains("breach")
        || combined.contains("p1")
    {
        Priority::High
    } else if combined.contains("slow")
        || combined.contains("performance")
        || combined.contains("regression")
        || combined.contains("broken")
    {
        Priority::Medium
    } else {
        Priority::Low
    };

    // Label heuristics.
    let mut labels: Vec<String> = Vec::new();
    if combined.contains("security") || combined.contains("vulnerability") || combined.contains("cve") {
        labels.push("security".into());
    }
    if combined.contains("performance") || combined.contains("slow") || combined.contains("latency") {
        labels.push("performance".into());
    }
    if combined.contains(" ui ") || combined.contains("frontend") || combined.contains("css") {
        labels.push("frontend".into());
    }
    if combined.contains("api") || combined.contains("backend") || combined.contains("server") {
        labels.push("backend".into());
    }
    if combined.contains("database") || combined.contains(" db ") || combined.contains("migration") {
        labels.push("database".into());
    }
    if combined.contains("crash") || combined.contains("panic") || combined.contains("exception") {
        labels.push("crash".into());
    }

    let reasoning = format!(
        "Heuristic: priority={:?} from keyword match; {} label(s) inferred.",
        priority,
        labels.len()
    );

    Some(TriageSuggestion {
        issue_id,
        suggested_priority: priority,
        suggested_labels: labels,
        reasoning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Issue, IssueType, Priority};
    use crate::TrackerStore;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_issue(project_id: Uuid, summary: &str) -> Issue {
        Issue {
            id: Uuid::new_v4(),
            key: "TEST-1".to_string(),
            project_id,
            project_key: "TEST".to_string(),
            issue_type: IssueType::Task,
            summary: summary.to_string(),
            description: None,
            status: "To Do".to_string(),
            priority: Priority::Medium,
            assignee: None,
            reporter: "admin".to_string(),
            labels: vec![],
            components: vec![],
            fix_versions: vec![],
            affects_versions: vec![],
            epic_id: None,
            parent_id: None,
            sprint_id: None,
            story_points: None,
            time_estimate_seconds: None,
            time_spent_seconds: 0,
            custom_fields: HashMap::new(),
            watchers: vec![],
            votes: 0,
            rank: 0,
            resolution: None,
            due_date: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            resolved_at: None,
        }
    }

    #[test]
    fn velocity_empty_project() {
        let store = TrackerStore::default();
        assert!(sprint_velocity(&store, Uuid::new_v4()).is_empty());
    }

    #[test]
    fn cycle_time_no_issues() {
        let store = TrackerStore::default();
        let stats = cycle_time_stats(&store, Uuid::new_v4());
        assert_eq!(stats.count, 0);
    }

    #[test]
    fn triage_production_outage() {
        let project_id = Uuid::new_v4();
        let mut store = TrackerStore::default();
        let issue = make_issue(project_id, "production outage all services down");
        let id = issue.id;
        store.issues.insert(id, issue);
        let s = triage_issue(&store, id).unwrap();
        assert_eq!(s.suggested_priority, Priority::Critical);
    }

    #[test]
    fn triage_security_label() {
        let project_id = Uuid::new_v4();
        let mut store = TrackerStore::default();
        let issue = make_issue(project_id, "security vulnerability in authentication");
        let id = issue.id;
        store.issues.insert(id, issue);
        let s = triage_issue(&store, id).unwrap();
        assert!(s.suggested_labels.iter().any(|l| l == "security"));
    }

    #[test]
    fn triage_missing_issue_returns_none() {
        let store = TrackerStore::default();
        assert!(triage_issue(&store, Uuid::new_v4()).is_none());
    }
}
