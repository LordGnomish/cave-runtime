// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::*;
use uuid::Uuid;

pub fn default_scrum_workflow() -> Workflow {
    Workflow {
        id: Uuid::new_v4(),
        name: "Scrum Workflow".to_string(),
        statuses: vec![
            WorkflowStatus { name: "Backlog".to_string(), category: StatusCategory::Todo, description: "Not yet in a sprint".to_string() },
            WorkflowStatus { name: "To Do".to_string(), category: StatusCategory::Todo, description: "In sprint, not started".to_string() },
            WorkflowStatus { name: "In Progress".to_string(), category: StatusCategory::InProgress, description: "Being worked on".to_string() },
            WorkflowStatus { name: "In Review".to_string(), category: StatusCategory::InProgress, description: "Under code review".to_string() },
            WorkflowStatus { name: "Done".to_string(), category: StatusCategory::Done, description: "Completed".to_string() },
        ],
        transitions: vec![
            Transition {
                id: "start".to_string(), name: "Start Progress".to_string(),
                from_status: vec!["Backlog".to_string(), "To Do".to_string()],
                to_status: "In Progress".to_string(),
                conditions: vec![], validators: vec![], post_functions: vec![],
            },
            Transition {
                id: "review".to_string(), name: "Submit for Review".to_string(),
                from_status: vec!["In Progress".to_string()],
                to_status: "In Review".to_string(),
                conditions: vec![], validators: vec![], post_functions: vec![],
            },
            Transition {
                id: "done".to_string(), name: "Mark Done".to_string(),
                from_status: vec!["In Review".to_string(), "In Progress".to_string()],
                to_status: "Done".to_string(),
                conditions: vec![],
                validators: vec![TransitionValidator::SubtasksResolved],
                post_functions: vec![PostFunction::SetResolution("Fixed".to_string()), PostFunction::NotifyWatchers],
            },
            Transition {
                id: "reopen".to_string(), name: "Reopen".to_string(),
                from_status: vec!["Done".to_string()],
                to_status: "To Do".to_string(),
                conditions: vec![], validators: vec![], post_functions: vec![PostFunction::SetField { field: "resolution".to_string(), value: serde_json::json!(null) }],
            },
        ],
        is_default: true,
    }
}

pub fn default_kanban_workflow() -> Workflow {
    Workflow {
        id: Uuid::new_v4(),
        name: "Kanban Workflow".to_string(),
        statuses: vec![
            WorkflowStatus { name: "To Do".to_string(), category: StatusCategory::Todo, description: "Ready to start".to_string() },
            WorkflowStatus { name: "In Progress".to_string(), category: StatusCategory::InProgress, description: "Being worked on".to_string() },
            WorkflowStatus { name: "Done".to_string(), category: StatusCategory::Done, description: "Completed".to_string() },
        ],
        transitions: vec![
            Transition {
                id: "start".to_string(), name: "Start".to_string(),
                from_status: vec!["To Do".to_string()],
                to_status: "In Progress".to_string(),
                conditions: vec![], validators: vec![], post_functions: vec![],
            },
            Transition {
                id: "done".to_string(), name: "Done".to_string(),
                from_status: vec!["In Progress".to_string()],
                to_status: "Done".to_string(),
                conditions: vec![], validators: vec![],
                post_functions: vec![PostFunction::NotifyWatchers],
            },
        ],
        is_default: false,
    }
}

/// Check if a transition is valid from the current status.
pub fn can_transition(workflow: &Workflow, from_status: &str, transition_id: &str) -> bool {
    workflow.transitions.iter().any(|t| {
        t.id == transition_id &&
        (t.from_status.is_empty() || t.from_status.iter().any(|s| s == from_status))
    })
}

/// Get available transitions from a given status.
pub fn available_transitions<'a>(workflow: &'a Workflow, from_status: &str) -> Vec<&'a Transition> {
    workflow.transitions.iter().filter(|t| {
        t.from_status.is_empty() || t.from_status.iter().any(|s| s == from_status)
    }).collect()
}

/// Apply a transition to an issue (update status, run post-functions).
pub fn apply_transition(issue: &mut crate::models::Issue, transition: &Transition) {
    issue.status = transition.to_status.clone();
    issue.updated_at = chrono::Utc::now();
    for pf in &transition.post_functions {
        match pf {
            PostFunction::SetResolution(res) => { issue.resolution = Some(res.clone()); }
            PostFunction::SetField { field, value } => {
                if field == "resolution" {
                    issue.resolution = value.as_str().map(|s| s.to_string());
                }
            }
            PostFunction::NotifyWatchers | PostFunction::NotifyAssignee | PostFunction::ClearAssignee => {}
        }
    }
    if issue.status == "Done" && issue.resolved_at.is_none() {
        issue.resolved_at = Some(chrono::Utc::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrum_workflow_has_5_statuses() {
        let wf = default_scrum_workflow();
        assert_eq!(wf.statuses.len(), 5);
    }

    #[test]
    fn test_can_transition() {
        let wf = default_scrum_workflow();
        assert!(can_transition(&wf, "To Do", "start"));
        assert!(!can_transition(&wf, "Done", "start"));
    }

    #[test]
    fn test_available_transitions_from_backlog() {
        let wf = default_scrum_workflow();
        let transitions = available_transitions(&wf, "Backlog");
        assert!(transitions.iter().any(|t| t.to_status == "In Progress"));
    }

    #[test]
    fn test_apply_transition_updates_status() {
        let wf = default_scrum_workflow();
        let mut issue = make_test_issue();
        let transition = wf.transitions.iter().find(|t| t.id == "start").unwrap();
        apply_transition(&mut issue, transition);
        assert_eq!(issue.status, "In Progress");
    }

    fn make_test_issue() -> crate::models::Issue {
        use std::collections::HashMap;
        Issue {
            id: Uuid::new_v4(), key: "TEST-1".to_string(),
            project_id: Uuid::new_v4(), project_key: "TEST".to_string(),
            issue_type: IssueType::Task, summary: "Test issue".to_string(),
            description: None, status: "To Do".to_string(),
            priority: Priority::Medium, assignee: None, reporter: "admin".to_string(),
            labels: vec![], components: vec![], fix_versions: vec![], affects_versions: vec![],
            epic_id: None, parent_id: None, sprint_id: None,
            story_points: None, time_estimate_seconds: None, time_spent_seconds: 0,
            custom_fields: HashMap::new(), watchers: vec![], votes: 0, rank: 0,
            resolution: None, due_date: None,
            created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(), resolved_at: None,
        }
    }
}
