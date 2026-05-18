// SPDX-License-Identifier: AGPL-3.0-or-later
//! Core tracker engine: issue lifecycle, assignment, metrics, AI-assisted triage.

use crate::models::*;
use crate::TrackerState;
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct CreateIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub issue_type: IssueType,
    pub priority: Priority,
    pub reporter: String,
    pub assignee: Option<String>,
    pub sprint_id: Option<Uuid>,
    pub epic_id: Option<Uuid>,
    pub story_points: Option<f32>,
    pub time_estimate: Option<u64>,
    pub due_date: Option<chrono::DateTime<Utc>>,
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub dependencies: Vec<Uuid>,
    #[serde(default)]
    pub labels: Vec<Uuid>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateIssueRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<Priority>,
    /// Empty string clears the assignee.
    pub assignee: Option<String>,
    pub sprint_id: Option<Uuid>,
    pub epic_id: Option<Uuid>,
    pub story_points: Option<f32>,
    pub time_estimate: Option<u64>,
    pub time_spent: Option<u64>,
    pub due_date: Option<chrono::DateTime<Utc>>,
    pub labels: Option<Vec<Uuid>>,
    pub dependencies: Option<Vec<Uuid>>,
}

// ── Result types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BlockerInfo {
    pub issue_id: Uuid,
    pub issue_key: String,
    pub reason: String,
    pub blocker_type: BlockerType,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerType {
    Dependency,
    Stale,
}

#[derive(Debug, Serialize)]
pub struct SprintSuggestion {
    pub recommended_velocity: f32,
    pub based_on_sprints: usize,
    pub suggested_issue_ids: Vec<Uuid>,
    pub estimated_points: f32,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct TriageSuggestion {
    pub issue_id: Uuid,
    pub suggested_priority: Priority,
    pub suggested_assignee: Option<String>,
    pub suggested_labels: Vec<String>,
    pub reasoning: String,
}

// ── Core engine ───────────────────────────────────────────────────────────────

/// Create a new issue under `project_id`, auto-generating the key (e.g. CAVE-7).
pub async fn create_issue(
    state: &TrackerState,
    project_id: Uuid,
    req: CreateIssueRequest,
) -> Result<Issue, String> {
    // Collect project data, then drop the lock.
    let (key_prefix, workflow_id) = {
        let projects = state.projects.lock().await;
        let project = projects.get(&project_id).ok_or("Project not found")?;
        (project.key.clone(), project.default_workflow)
    };

    let initial_status = if let Some(wf_id) = workflow_id {
        let workflows = state.workflows.lock().await;
        workflows
            .get(&wf_id)
            .map(|w| w.initial_state.clone())
            .unwrap_or_else(|| "Open".to_string())
    } else {
        "Open".to_string()
    };

    let issue_num = {
        let mut counters = state.issue_counters.lock().await;
        let c = counters.entry(project_id).or_insert(0);
        *c += 1;
        *c
    };

    let now = Utc::now();
    let issue = Issue {
        id: Uuid::new_v4(),
        key: format!("{}-{}", key_prefix, issue_num),
        project_id,
        title: req.title,
        description: req.description,
        issue_type: req.issue_type,
        status: initial_status,
        priority: req.priority,
        assignee: req.assignee,
        reporter: req.reporter,
        labels: req.labels,
        sprint_id: req.sprint_id,
        epic_id: req.epic_id,
        story_points: req.story_points,
        time_estimate: req.time_estimate,
        time_spent: None,
        created_at: now,
        updated_at: now,
        due_date: req.due_date,
        parent_id: req.parent_id,
        dependencies: req.dependencies,
    };

    state.issues.lock().await.insert(issue.id, issue.clone());
    record_activity(state, issue.id, "system".into(), "created", None, None, None).await;

    Ok(issue)
}

/// Update mutable fields on an issue, recording an activity entry per changed field.
pub async fn update_issue(
    state: &TrackerState,
    issue_id: Uuid,
    req: UpdateIssueRequest,
    actor: String,
) -> Result<Issue, String> {
    let mut changed: Vec<(String, String, String)> = Vec::new();

    let updated = {
        let mut issues = state.issues.lock().await;
        let issue = issues.get_mut(&issue_id).ok_or("Issue not found")?;

        if let Some(title) = req.title {
            changed.push(("title".into(), issue.title.clone(), title.clone()));
            issue.title = title;
        }
        if let Some(desc) = req.description {
            issue.description = Some(desc);
        }
        if let Some(priority) = req.priority {
            changed.push(("priority".into(), format!("{:?}", issue.priority), format!("{:?}", priority)));
            issue.priority = priority;
        }
        if let Some(assignee) = req.assignee {
            let old = issue.assignee.clone().unwrap_or_default();
            let new = if assignee.is_empty() { None } else { Some(assignee.clone()) };
            changed.push(("assignee".into(), old, assignee));
            issue.assignee = new;
        }
        if let Some(sprint_id) = req.sprint_id {
            issue.sprint_id = Some(sprint_id);
        }
        if let Some(epic_id) = req.epic_id {
            issue.epic_id = Some(epic_id);
        }
        if let Some(sp) = req.story_points {
            issue.story_points = Some(sp);
        }
        if let Some(te) = req.time_estimate {
            issue.time_estimate = Some(te);
        }
        if let Some(ts) = req.time_spent {
            issue.time_spent = Some(ts);
        }
        if let Some(dd) = req.due_date {
            issue.due_date = Some(dd);
        }
        if let Some(labels) = req.labels {
            issue.labels = labels;
        }
        if let Some(deps) = req.dependencies {
            issue.dependencies = deps;
        }

        issue.updated_at = Utc::now();
        issue.clone()
    };

    for (field, old, new) in changed {
        record_activity(state, issue_id, actor.clone(), "updated", Some(field), Some(old), Some(new)).await;
    }

    Ok(updated)
}

/// Transition an issue to a new status, enforcing workflow rules when a workflow is configured.
pub async fn transition_issue(
    state: &TrackerState,
    issue_id: Uuid,
    to_status: String,
    actor: String,
) -> Result<Issue, String> {
    // Read issue metadata without holding the lock across async calls.
    let (project_id, from_status) = {
        let issues = state.issues.lock().await;
        let issue = issues.get(&issue_id).ok_or("Issue not found")?;
        (issue.project_id, issue.status.clone())
    };

    // Validate transition against project workflow (if any).
    let workflow_id = {
        let projects = state.projects.lock().await;
        projects.get(&project_id).and_then(|p| p.default_workflow)
    };

    if let Some(wf_id) = workflow_id {
        let workflows = state.workflows.lock().await;
        if let Some(wf) = workflows.get(&wf_id) {
            let valid = wf.transitions.iter().any(|t| t.from == from_status && t.to == to_status);
            if !valid {
                return Err(format!(
                    "Workflow '{}' does not allow transition from '{}' to '{}'",
                    wf.name, from_status, to_status
                ));
            }
        }
    }

    // Apply the transition.
    let updated = {
        let mut issues = state.issues.lock().await;
        let issue = issues.get_mut(&issue_id).ok_or("Issue not found")?;
        issue.status = to_status.clone();
        issue.updated_at = Utc::now();
        issue.clone()
    };

    record_activity(
        state, issue_id, actor, "transitioned",
        Some("status".into()), Some(from_status), Some(to_status),
    ).await;

    Ok(updated)
}

/// Assign an issue to the team member with the fewest open issues in the project.
pub async fn auto_assign(
    state: &TrackerState,
    project_id: Uuid,
    issue_id: Uuid,
    team: &[String],
) -> Result<String, String> {
    if team.is_empty() {
        return Err("No team members provided".into());
    }

    let assignee = {
        let issues = state.issues.lock().await;
        let mut load: HashMap<&str, usize> = team.iter().map(|m| (m.as_str(), 0)).collect();
        for issue in issues.values() {
            if issue.project_id == project_id && issue.status != "Done" {
                if let Some(a) = &issue.assignee {
                    if let Some(cnt) = load.get_mut(a.as_str()) {
                        *cnt += 1;
                    }
                }
            }
        }
        load.iter()
            .min_by_key(|(_, cnt)| *cnt)
            .map(|(name, _)| name.to_string())
            .unwrap_or_else(|| team[0].clone())
    };

    {
        let mut issues = state.issues.lock().await;
        if let Some(issue) = issues.get_mut(&issue_id) {
            issue.assignee = Some(assignee.clone());
            issue.updated_at = Utc::now();
        }
    }

    Ok(assignee)
}

/// Average hours between an issue first entering "In Progress" and reaching "Done".
pub async fn calculate_cycle_time(state: &TrackerState, project_id: Uuid) -> f64 {
    let done_ids: Vec<Uuid> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id && i.status == "Done")
            .map(|i| i.id)
            .collect()
    };

    if done_ids.is_empty() {
        return 0.0;
    }

    let activities = state.activities.lock().await;
    let mut total_hours = 0.0f64;
    let mut count = 0usize;

    for id in &done_ids {
        if let Some(acts) = activities.get(id) {
            let start = acts.iter()
                .filter(|a| a.field_changed.as_deref() == Some("status") && a.new_value.as_deref() == Some("In Progress"))
                .map(|a| a.timestamp)
                .min();
            let end = acts.iter()
                .filter(|a| a.field_changed.as_deref() == Some("status") && a.new_value.as_deref() == Some("Done"))
                .map(|a| a.timestamp)
                .max();
            if let (Some(s), Some(e)) = (start, end) {
                let h = (e - s).num_minutes() as f64 / 60.0;
                if h > 0.0 {
                    total_hours += h;
                    count += 1;
                }
            }
        }
    }

    if count == 0 { 0.0 } else { total_hours / count as f64 }
}

/// Average hours from issue creation to reaching "Done".
pub async fn calculate_lead_time(state: &TrackerState, project_id: Uuid) -> f64 {
    let done_issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id && i.status == "Done")
            .cloned()
            .collect()
    };

    if done_issues.is_empty() {
        return 0.0;
    }

    let activities = state.activities.lock().await;
    let mut total_hours = 0.0f64;
    let mut count = 0usize;

    for issue in &done_issues {
        if let Some(acts) = activities.get(&issue.id) {
            if let Some(done_at) = acts.iter()
                .filter(|a| a.field_changed.as_deref() == Some("status") && a.new_value.as_deref() == Some("Done"))
                .map(|a| a.timestamp)
                .max()
            {
                let h = (done_at - issue.created_at).num_minutes() as f64 / 60.0;
                if h > 0.0 {
                    total_hours += h;
                    count += 1;
                }
            }
        }
    }

    if count == 0 { 0.0 } else { total_hours / count as f64 }
}

/// Find issues blocked by unresolved dependencies or stale for > 3 days in "In Progress".
pub async fn detect_blockers(state: &TrackerState, project_id: Uuid) -> Vec<BlockerInfo> {
    let issues: Vec<Issue> = {
        let issues = state.issues.lock().await;
        issues.values()
            .filter(|i| i.project_id == project_id && i.status != "Done")
            .cloned()
            .collect()
    };

    let all_issues: HashMap<Uuid, String> = {
        let issues = state.issues.lock().await;
        issues.values().map(|i| (i.id, i.status.clone())).collect()
    };

    let now = Utc::now();
    let stale_threshold = chrono::Duration::days(3);
    let mut blockers = Vec::new();

    for issue in &issues {
        // Blocked by unresolved dependency.
        for dep_id in &issue.dependencies {
            if let Some(dep_status) = all_issues.get(dep_id) {
                if dep_status != "Done" {
                    blockers.push(BlockerInfo {
                        issue_id: issue.id,
                        issue_key: issue.key.clone(),
                        reason: format!("Blocked by dependency {} (status: {})", dep_id, dep_status),
                        blocker_type: BlockerType::Dependency,
                    });
                    break;
                }
            }
        }

        // Stale "In Progress" issues.
        if issue.status == "In Progress" && now - issue.updated_at > stale_threshold {
            blockers.push(BlockerInfo {
                issue_id: issue.id,
                issue_key: issue.key.clone(),
                reason: format!("Stale: no update in {} days", (now - issue.updated_at).num_days()),
                blocker_type: BlockerType::Stale,
            });
        }
    }

    blockers
}

/// Suggest sprint scope using average velocity from the last 3 completed sprints.
/// Selects backlog items (by rank) that fit within the velocity budget.
pub async fn suggest_sprint_scope(state: &TrackerState, project_id: Uuid) -> SprintSuggestion {
    // Step 1: completed sprint IDs (last 3).
    let completed_sprint_ids: Vec<Uuid> = {
        let sprints = state.sprints.lock().await;
        sprints.values()
            .filter(|s| s.project_id == project_id && s.status == SprintStatus::Completed)
            .map(|s| s.id)
            .take(3)
            .collect()
    };

    // Step 2: velocity from completed sprints.
    let velocity: f32 = {
        let issues = state.issues.lock().await;
        if completed_sprint_ids.is_empty() {
            20.0 // default velocity
        } else {
            let total: f32 = completed_sprint_ids.iter()
                .map(|sid| {
                    issues.values()
                        .filter(|i| i.sprint_id == Some(*sid) && i.status == "Done")
                        .filter_map(|i| i.story_points)
                        .sum::<f32>()
                })
                .sum();
            total / completed_sprint_ids.len() as f32
        }
    };

    // Step 3: sorted backlog.
    let backlog_items: Vec<BacklogItem> = {
        let backlogs = state.backlogs.lock().await;
        let mut items = backlogs.get(&project_id).cloned().unwrap_or_default();
        items.sort_by(|a, b| a.rank.partial_cmp(&b.rank).unwrap_or(std::cmp::Ordering::Equal));
        items
    };

    // Step 4: fill sprint up to ~110 % of velocity.
    let suggested_ids: Vec<Uuid> = {
        let issues = state.issues.lock().await;
        let mut accumulated = 0.0f32;
        let mut suggested = Vec::new();
        for item in &backlog_items {
            if let Some(issue) = issues.get(&item.issue_id) {
                if issue.status == "Done" {
                    continue;
                }
                let pts = issue.story_points.unwrap_or(2.0);
                if accumulated + pts <= velocity * 1.1 {
                    suggested.push(item.issue_id);
                    accumulated += pts;
                }
            }
        }
        suggested
    };

    let note = if completed_sprint_ids.is_empty() {
        "No completed sprints found; using default velocity of 20 points.".into()
    } else {
        format!("Velocity averaged over {} completed sprint(s).", completed_sprint_ids.len())
    };

    SprintSuggestion {
        recommended_velocity: velocity,
        based_on_sprints: completed_sprint_ids.len(),
        suggested_issue_ids: suggested_ids,
        estimated_points: velocity,
        note,
    }
}

/// Heuristic-based bug triage: infers priority, labels, and a suggested assignee
/// from the issue's title and description. In production this would call an LLM.
pub async fn auto_triage(state: &TrackerState, issue_id: Uuid) -> TriageSuggestion {
    let issue = {
        let issues = state.issues.lock().await;
        match issues.get(&issue_id).cloned() {
            Some(i) => i,
            None => {
                return TriageSuggestion {
                    issue_id,
                    suggested_priority: Priority::Medium,
                    suggested_assignee: None,
                    suggested_labels: vec![],
                    reasoning: "Issue not found".into(),
                }
            }
        }
    };

    let combined = format!(
        "{} {}",
        issue.title.to_lowercase(),
        issue.description.as_deref().unwrap_or("").to_lowercase()
    );

    // Priority heuristics.
    let priority = if combined.contains("critical") || combined.contains("outage") || combined.contains("production down") {
        Priority::Critical
    } else if combined.contains("urgent") || combined.contains("security") || combined.contains("data loss") || combined.contains("breach") {
        Priority::High
    } else if combined.contains("slow") || combined.contains("performance") || combined.contains("regression") || combined.contains("broken") {
        Priority::Medium
    } else {
        Priority::Low
    };

    // Label heuristics.
    let mut labels = Vec::new();
    if combined.contains("security") || combined.contains("vulnerability") || combined.contains("cve") {
        labels.push("security".into());
    }
    if combined.contains("performance") || combined.contains("slow") || combined.contains("latency") {
        labels.push("performance".into());
    }
    if combined.contains("ui") || combined.contains("frontend") || combined.contains("ux") || combined.contains("css") {
        labels.push("frontend".into());
    }
    if combined.contains("api") || combined.contains("backend") || combined.contains("server") {
        labels.push("backend".into());
    }
    if combined.contains("database") || combined.contains(" db ") || combined.contains("query") || combined.contains("migration") {
        labels.push("database".into());
    }
    if combined.contains("crash") || combined.contains("panic") || combined.contains("exception") {
        labels.push("crash".into());
    }

    let suggested_assignee = suggest_assignee(state, issue.project_id).await;

    let reasoning = format!(
        "Heuristic analysis: priority={:?} based on keyword matching; {} label(s) inferred from description.",
        priority, labels.len()
    );

    TriageSuggestion {
        issue_id,
        suggested_priority: priority,
        suggested_assignee,
        suggested_labels: labels,
        reasoning,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Find the least-loaded assignee in a project (fewest open issues).
async fn suggest_assignee(state: &TrackerState, project_id: Uuid) -> Option<String> {
    let issues = state.issues.lock().await;
    let mut load: HashMap<String, usize> = HashMap::new();
    for issue in issues.values() {
        if issue.project_id == project_id && issue.status != "Done" {
            if let Some(a) = &issue.assignee {
                *load.entry(a.clone()).or_insert(0) += 1;
            }
        }
    }
    load.into_iter().min_by_key(|(_, c)| *c).map(|(name, _)| name)
}

/// Append an activity record for an issue. Never fails.
pub async fn record_activity(
    state: &TrackerState,
    issue_id: Uuid,
    actor: String,
    action: &str,
    field: Option<String>,
    old_val: Option<String>,
    new_val: Option<String>,
) {
    let entry = Activity {
        id: Uuid::new_v4(),
        issue_id,
        actor,
        action: action.to_string(),
        field_changed: field,
        old_value: old_val,
        new_value: new_val,
        timestamp: Utc::now(),
    };
    state.activities.lock().await
        .entry(issue_id)
        .or_insert_with(Vec::new)
        .push(entry);
}
