//! Board and view management: kanban, sprint board, backlog, burndown, CFD.

use crate::models::*;
use crate::TrackerState;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

// ── View types ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct KanbanColumn {
    pub id: String,
    pub name: String,
    pub issues: Vec<Issue>,
    pub issue_count: usize,
    pub wip_limit: Option<usize>,
    pub wip_exceeded: bool,
}

#[derive(Debug, Serialize)]
pub struct KanbanView {
    pub board_id: Uuid,
    pub board_name: String,
    pub columns: Vec<KanbanColumn>,
    pub total_issues: usize,
}

#[derive(Debug, Serialize)]
pub struct BacklogView {
    pub project_id: Uuid,
    pub items: Vec<BacklogIssue>,
    pub unranked: Vec<Issue>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct BacklogIssue {
    pub rank: f64,
    pub issue: Issue,
}

#[derive(Debug, Serialize)]
pub struct BurndownData {
    pub sprint_id: Uuid,
    pub sprint_name: String,
    pub total_points: f32,
    pub completed_points: f32,
    pub remaining_points: f32,
    pub ideal_remaining: f32,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub total_issues: usize,
    pub completed_issues: usize,
}

#[derive(Debug, Serialize)]
pub struct CumulativeFlowData {
    pub project_id: Uuid,
    /// Count of issues currently in each status.
    pub status_counts: HashMap<String, usize>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct WipViolation {
    pub column_id: String,
    pub column_name: String,
    pub current_count: usize,
    pub wip_limit: usize,
    pub overflow: usize,
}

// ── Board functions ───────────────────────────────────────────────────────────

/// Build a kanban view for the project's first registered board.
/// Falls back to a sensible four-column default if no board is configured.
pub async fn kanban_view(state: &TrackerState, project_id: Uuid) -> KanbanView {
    let board = {
        let boards = state.boards.lock().await;
        boards.values()
            .find(|b| b.project_id == project_id)
            .cloned()
            .unwrap_or_else(|| default_board(project_id))
    };

    let issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id)
            .cloned()
            .collect()
    };

    build_kanban_view(board, issues)
}

/// Kanban view scoped to a single sprint.
pub async fn sprint_board(state: &TrackerState, project_id: Uuid, sprint_id: Uuid) -> KanbanView {
    let board = {
        let boards = state.boards.lock().await;
        boards.values()
            .find(|b| b.project_id == project_id)
            .cloned()
            .unwrap_or_else(|| default_board(project_id))
    };

    let issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id && i.sprint_id == Some(sprint_id))
            .cloned()
            .collect()
    };

    build_kanban_view(board, issues)
}

/// Prioritized backlog: ranked items first, then unranked issues not in any sprint.
pub async fn backlog_view(state: &TrackerState, project_id: Uuid) -> BacklogView {
    let backlog_items: Vec<BacklogItem> = {
        let backlogs = state.backlogs.lock().await;
        let mut items = backlogs.get(&project_id).cloned().unwrap_or_default();
        items.sort_by(|a, b| a.rank.partial_cmp(&b.rank).unwrap_or(std::cmp::Ordering::Equal));
        items
    };

    let all_issues: HashMap<Uuid, Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id && i.status != "Done")
            .map(|i| (i.id, i.clone()))
            .collect()
    };

    let ranked_ids: std::collections::HashSet<Uuid> = backlog_items.iter().map(|b| b.issue_id).collect();

    let mut ranked: Vec<BacklogIssue> = backlog_items.into_iter()
        .filter_map(|b| all_issues.get(&b.issue_id).map(|i| BacklogIssue { rank: b.rank, issue: i.clone() }))
        .collect();
    ranked.sort_by(|a, b| a.rank.partial_cmp(&b.rank).unwrap_or(std::cmp::Ordering::Equal));

    let unranked: Vec<Issue> = all_issues.values()
        .filter(|i| !ranked_ids.contains(&i.id) && i.sprint_id.is_none())
        .cloned()
        .collect();

    let total = ranked.len() + unranked.len();

    BacklogView { project_id, items: ranked, unranked, total }
}

/// Burndown chart data for a sprint: total vs completed points, ideal line.
pub async fn burndown_data(state: &TrackerState, sprint_id: Uuid) -> Option<BurndownData> {
    let sprint = {
        let sprints = state.sprints.lock().await;
        sprints.get(&sprint_id).cloned()?
    };

    let sprint_issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.sprint_id == Some(sprint_id))
            .cloned()
            .collect()
    };

    let total_points: f32 = sprint_issues.iter().filter_map(|i| i.story_points).sum();
    let completed_points: f32 = sprint_issues.iter()
        .filter(|i| i.status == "Done")
        .filter_map(|i| i.story_points)
        .sum();
    let remaining = total_points - completed_points;

    let now = Utc::now();
    let start = sprint.start_date.unwrap_or(now);
    let end = sprint.end_date.unwrap_or_else(|| start + chrono::Duration::days(14));

    let sprint_days = (end - start).num_days().max(1) as f32;
    let elapsed_days = (now - start).num_days().clamp(0, sprint_days as i64) as f32;
    let ideal_remaining = (total_points * (1.0 - elapsed_days / sprint_days)).max(0.0);

    Some(BurndownData {
        sprint_id,
        sprint_name: sprint.name,
        total_points,
        completed_points,
        remaining_points: remaining,
        ideal_remaining,
        start,
        end,
        total_issues: sprint_issues.len(),
        completed_issues: sprint_issues.iter().filter(|i| i.status == "Done").count(),
    })
}

/// Cumulative flow: current count of issues in each status for a project.
pub async fn cumulative_flow_data(state: &TrackerState, project_id: Uuid) -> CumulativeFlowData {
    let status_counts = {
        let issues = state.issues.lock().await;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for issue in issues.values().filter(|i| i.project_id == project_id) {
            *counts.entry(issue.status.clone()).or_insert(0) += 1;
        }
        counts
    };

    CumulativeFlowData { project_id, status_counts, generated_at: Utc::now() }
}

/// Return all columns in a project's board that exceed their WIP limit.
pub async fn wip_violations(state: &TrackerState, project_id: Uuid) -> Vec<WipViolation> {
    let board = {
        let boards = state.boards.lock().await;
        boards.values()
            .find(|b| b.project_id == project_id)
            .cloned()
            .unwrap_or_else(|| default_board(project_id))
    };

    let issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id)
            .cloned()
            .collect()
    };

    let mut violations = Vec::new();
    for col in &board.columns {
        if let Some(limit) = col.wip_limit {
            let count = issues.iter()
                .filter(|i| col.status_mappings.contains(&i.status))
                .count();
            if count > limit {
                violations.push(WipViolation {
                    column_id: col.id.clone(),
                    column_name: col.name.clone(),
                    current_count: count,
                    wip_limit: limit,
                    overflow: count - limit,
                });
            }
        }
    }
    violations
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_kanban_view(board: Board, issues: Vec<Issue>) -> KanbanView {
    let columns: Vec<KanbanColumn> = board.columns.iter().map(|col| {
        let col_issues: Vec<Issue> = issues.iter()
            .filter(|i| col.status_mappings.contains(&i.status))
            .cloned()
            .collect();
        let count = col_issues.len();
        let exceeded = col.wip_limit.map(|l| count > l).unwrap_or(false);
        KanbanColumn {
            id: col.id.clone(),
            name: col.name.clone(),
            issue_count: count,
            issues: col_issues,
            wip_limit: col.wip_limit,
            wip_exceeded: exceeded,
        }
    }).collect();

    let total = issues.len();
    KanbanView {
        board_id: board.id,
        board_name: board.name,
        columns,
        total_issues: total,
    }
}

fn default_board(project_id: Uuid) -> Board {
    Board {
        id: Uuid::nil(),
        project_id,
        name: "Default Board".into(),
        columns: vec![
            BoardColumn {
                id: "todo".into(),
                name: "To Do".into(),
                status_mappings: vec!["Open".into(), "To Do".into(), "Backlog".into()],
                wip_limit: None,
            },
            BoardColumn {
                id: "in_progress".into(),
                name: "In Progress".into(),
                status_mappings: vec!["In Progress".into()],
                wip_limit: Some(5),
            },
            BoardColumn {
                id: "review".into(),
                name: "Review".into(),
                status_mappings: vec!["Review".into(), "Code Review".into(), "In Review".into()],
                wip_limit: Some(3),
            },
            BoardColumn {
                id: "done".into(),
                name: "Done".into(),
                status_mappings: vec!["Done".into(), "Closed".into(), "Resolved".into()],
                wip_limit: None,
            },
        ],
        filters: serde_json::Value::Object(serde_json::Map::new()),
    }
}
