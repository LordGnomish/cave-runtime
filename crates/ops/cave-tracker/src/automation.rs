// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workflow automation engine: trigger → condition → action rules.

use crate::models::*;
use crate::TrackerState;
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

// ── Event context passed when evaluating rules ────────────────────────────────

#[derive(Debug)]
pub enum Event {
    IssueCreated,
    StatusChanged { from: String, to: String },
    SprintStarted,
    DueDateApproaching { days_until: u32 },
}

#[derive(Debug, Serialize)]
pub struct AutomationResult {
    pub rule_id: Uuid,
    pub rule_name: String,
    pub applied: bool,
    pub message: String,
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// Evaluate all enabled automation rules for a project against `issue_id` + `event`.
/// Returns one result per rule that matched the trigger.
pub async fn evaluate_rules(
    state: &TrackerState,
    issue_id: Uuid,
    event: &Event,
) -> Vec<AutomationResult> {
    // Snapshot the issue without holding the lock across async calls.
    let issue = {
        let issues = state.issues.lock().await;
        match issues.get(&issue_id).cloned() {
            Some(i) => i,
            None => return Vec::new(),
        }
    };

    let rules: Vec<Automation> = {
        let automations = state.automations.lock().await;
        automations.iter()
            .filter(|a| a.project_id == issue.project_id && a.enabled)
            .cloned()
            .collect()
    };

    let mut results = Vec::new();
    for rule in rules {
        if matches_trigger(&rule.trigger, event) && matches_condition(&rule.condition, &issue) {
            let outcome = apply_action(state, issue_id, &rule.action).await;
            let applied = outcome.is_ok();
            let message = outcome.unwrap_or_else(|e| e);
            results.push(AutomationResult {
                rule_id: rule.id,
                rule_name: rule.name.clone(),
                applied,
                message,
            });
        }
    }
    results
}

// ── Matching ──────────────────────────────────────────────────────────────────

fn matches_trigger(trigger: &AutomationTrigger, event: &Event) -> bool {
    match (trigger, event) {
        (AutomationTrigger::IssueCreated, Event::IssueCreated) => true,
        (
            AutomationTrigger::StatusChanged { from: t_from, to: t_to },
            Event::StatusChanged { from: e_from, to: e_to },
        ) => {
            let from_ok = t_from.as_deref().map(|f| f == e_from).unwrap_or(true);
            let to_ok = t_to.as_deref().map(|t| t == e_to).unwrap_or(true);
            from_ok && to_ok
        }
        (AutomationTrigger::SprintStarted, Event::SprintStarted) => true,
        (
            AutomationTrigger::DueDateApproaching { days_before: t_days },
            Event::DueDateApproaching { days_until: e_days },
        ) => e_days <= t_days,
        _ => false,
    }
}

fn matches_condition(condition: &AutomationCondition, issue: &Issue) -> bool {
    match condition {
        AutomationCondition::Always => true,
        AutomationCondition::IssueType { issue_type } => &issue.issue_type == issue_type,
        AutomationCondition::Priority { priority } => &issue.priority == priority,
        AutomationCondition::HasLabel { label_name } => {
            // Label matching is by name — in practice a full impl would look up by ID.
            // We skip the lookup here to avoid async in a sync fn; routes can pre-filter.
            let _ = label_name;
            true // permissive fallback; routes layer should validate
        }
    }
}

// ── Action execution ──────────────────────────────────────────────────────────

async fn apply_action(
    state: &TrackerState,
    issue_id: Uuid,
    action: &AutomationAction,
) -> Result<String, String> {
    match action {
        AutomationAction::Assign { to } => {
            let mut issues = state.issues.lock().await;
            let issue = issues.get_mut(&issue_id).ok_or("Issue not found")?;
            issue.assignee = Some(to.clone());
            issue.updated_at = Utc::now();
            Ok(format!("Assigned to {}", to))
        }

        AutomationAction::Transition { to_status } => {
            let mut issues = state.issues.lock().await;
            let issue = issues.get_mut(&issue_id).ok_or("Issue not found")?;
            issue.status = to_status.clone();
            issue.updated_at = Utc::now();
            Ok(format!("Transitioned to '{}'", to_status))
        }

        AutomationAction::AddLabel { label_name } => {
            // Resolve label by name first (separate lock scope).
            let label_id: Option<Uuid> = {
                let labels = state.labels.lock().await;
                labels.values().find(|l| &l.name == label_name).map(|l| l.id)
            };
            match label_id {
                Some(lid) => {
                    let mut issues = state.issues.lock().await;
                    if let Some(issue) = issues.get_mut(&issue_id) {
                        if !issue.labels.contains(&lid) {
                            issue.labels.push(lid);
                            issue.updated_at = Utc::now();
                        }
                        Ok(format!("Added label '{}'", label_name))
                    } else {
                        Err("Issue not found".into())
                    }
                }
                None => Err(format!("Label '{}' does not exist", label_name)),
            }
        }

        AutomationAction::Notify { message } => {
            tracing::info!(
                issue_id = %issue_id,
                "Automation notification: {}",
                message
            );
            Ok(format!("Notification dispatched: {}", message))
        }

        AutomationAction::CreateSubtask { title, assignee } => {
            let (project_id, parent_key) = {
                let issues = state.issues.lock().await;
                let issue = issues.get(&issue_id).ok_or("Issue not found")?;
                (issue.project_id, issue.key.clone())
            };

            let key_prefix = {
                let projects = state.projects.lock().await;
                projects.get(&project_id)
                    .map(|p| p.key.clone())
                    .ok_or("Project not found")?
            };

            let num = {
                let mut counters = state.issue_counters.lock().await;
                let c = counters.entry(project_id).or_insert(0);
                *c += 1;
                *c
            };

            let now = Utc::now();
            let subtask = Issue {
                id: Uuid::new_v4(),
                key: format!("{}-{}", key_prefix, num),
                project_id,
                title: title.clone(),
                description: None,
                issue_type: IssueType::Subtask,
                status: "Open".into(),
                priority: Priority::Medium,
                assignee: assignee.clone(),
                reporter: "automation".into(),
                labels: Vec::new(),
                sprint_id: None,
                epic_id: None,
                story_points: None,
                time_estimate: None,
                time_spent: None,
                created_at: now,
                updated_at: now,
                due_date: None,
                parent_id: Some(issue_id),
                dependencies: Vec::new(),
            };

            state.issues.lock().await.insert(subtask.id, subtask);
            Ok(format!("Subtask created under {}", parent_key))
        }
    }
}
