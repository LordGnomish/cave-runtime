//! Workflow engine for issue status transitions.

use crate::models::{Issue, IssueStatus, StatusTransition, TransitionCondition};

/// Drives issue lifecycle transitions with configurable rules.
pub struct WorkflowEngine {
    transitions: Vec<StatusTransition>,
}

impl WorkflowEngine {
    /// Create a new engine with the default CAVE tracker workflow.
    pub fn new() -> Self {
        let transitions = vec![
            StatusTransition {
                from_status: IssueStatus::ToDo,
                to_status: IssueStatus::InProgress,
                name: "Start".to_string(),
                conditions: vec![],
            },
            StatusTransition {
                from_status: IssueStatus::InProgress,
                to_status: IssueStatus::InReview,
                name: "Submit for Review".to_string(),
                conditions: vec![],
            },
            StatusTransition {
                from_status: IssueStatus::InProgress,
                to_status: IssueStatus::Done,
                name: "Mark Done".to_string(),
                conditions: vec![],
            },
            StatusTransition {
                from_status: IssueStatus::InReview,
                to_status: IssueStatus::Done,
                name: "Approve".to_string(),
                conditions: vec![],
            },
            StatusTransition {
                from_status: IssueStatus::InReview,
                to_status: IssueStatus::InProgress,
                name: "Send Back".to_string(),
                conditions: vec![],
            },
            // Any → Cancelled
            StatusTransition {
                from_status: IssueStatus::ToDo,
                to_status: IssueStatus::Cancelled,
                name: "Cancel".to_string(),
                conditions: vec![],
            },
            StatusTransition {
                from_status: IssueStatus::InProgress,
                to_status: IssueStatus::Cancelled,
                name: "Cancel".to_string(),
                conditions: vec![],
            },
            StatusTransition {
                from_status: IssueStatus::InReview,
                to_status: IssueStatus::Cancelled,
                name: "Cancel".to_string(),
                conditions: vec![],
            },
            // Done → ToDo (reopen)
            StatusTransition {
                from_status: IssueStatus::Done,
                to_status: IssueStatus::ToDo,
                name: "Reopen".to_string(),
                conditions: vec![],
            },
        ];
        Self { transitions }
    }

    /// Check whether a transition from `from` to `to` exists in the engine.
    pub fn can_transition(&self, from: &IssueStatus, to: &IssueStatus) -> bool {
        self.transitions
            .iter()
            .any(|t| &t.from_status == from && &t.to_status == to)
    }

    /// Return all statuses reachable from `from`.
    pub fn valid_transitions(&self, from: &IssueStatus) -> Vec<IssueStatus> {
        self.transitions
            .iter()
            .filter(|t| &t.from_status == from)
            .map(|t| t.to_status.clone())
            .collect()
    }

    /// Validate a transition for a specific issue, checking any attached conditions.
    pub fn validate_transition(
        &self,
        issue: &Issue,
        to: &IssueStatus,
        fields: &serde_json::Value,
    ) -> Result<(), String> {
        let transition = self
            .transitions
            .iter()
            .find(|t| &t.from_status == &issue.status && &t.to_status == to)
            .ok_or_else(|| {
                format!(
                    "No transition from {:?} to {:?}",
                    issue.status, to
                )
            })?;

        for condition in &transition.conditions {
            match condition {
                TransitionCondition::RequiredFields(required) => {
                    for field in required {
                        if fields.get(field).is_none() {
                            return Err(format!("Required field missing: {field}"));
                        }
                    }
                }
                TransitionCondition::MinStoryPoints(min) => {
                    let points = issue.story_points.unwrap_or(0.0);
                    if points < *min {
                        return Err(format!(
                            "Minimum story points required: {min}, issue has: {points}"
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IssueType, Priority};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_issue(status: IssueStatus) -> Issue {
        Issue {
            id: Uuid::new_v4(),
            project_key: "TEST".to_string(),
            issue_number: 1,
            issue_type: IssueType::Task,
            summary: "Test issue".to_string(),
            description: None,
            assignee: None,
            reporter: Uuid::new_v4(),
            priority: Priority::P3,
            status,
            labels: vec![],
            components: vec![],
            sprint_id: None,
            story_points: None,
            due_date: None,
            parent_id: None,
            epic_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: Uuid::new_v4(),
            original_estimate_minutes: None,
            time_spent_minutes: None,
            remaining_estimate_minutes: None,
        }
    }

    #[test]
    fn test_todo_to_in_progress_is_valid() {
        let engine = WorkflowEngine::new();
        assert!(engine.can_transition(&IssueStatus::ToDo, &IssueStatus::InProgress));
    }

    #[test]
    fn test_todo_to_done_is_invalid() {
        let engine = WorkflowEngine::new();
        assert!(!engine.can_transition(&IssueStatus::ToDo, &IssueStatus::Done));
    }

    #[test]
    fn test_in_progress_to_in_review_is_valid() {
        let engine = WorkflowEngine::new();
        assert!(engine.can_transition(&IssueStatus::InProgress, &IssueStatus::InReview));
    }

    #[test]
    fn test_in_review_to_done_is_valid() {
        let engine = WorkflowEngine::new();
        assert!(engine.can_transition(&IssueStatus::InReview, &IssueStatus::Done));
    }

    #[test]
    fn test_in_review_send_back_to_in_progress() {
        let engine = WorkflowEngine::new();
        assert!(engine.can_transition(&IssueStatus::InReview, &IssueStatus::InProgress));
    }

    #[test]
    fn test_any_to_cancelled() {
        let engine = WorkflowEngine::new();
        assert!(engine.can_transition(&IssueStatus::ToDo, &IssueStatus::Cancelled));
        assert!(engine.can_transition(&IssueStatus::InProgress, &IssueStatus::Cancelled));
        assert!(engine.can_transition(&IssueStatus::InReview, &IssueStatus::Cancelled));
    }

    #[test]
    fn test_done_to_todo_reopen() {
        let engine = WorkflowEngine::new();
        assert!(engine.can_transition(&IssueStatus::Done, &IssueStatus::ToDo));
    }

    #[test]
    fn test_cancelled_has_no_outgoing_transitions() {
        let engine = WorkflowEngine::new();
        let transitions = engine.valid_transitions(&IssueStatus::Cancelled);
        assert!(transitions.is_empty());
    }

    #[test]
    fn test_valid_transitions_for_todo() {
        let engine = WorkflowEngine::new();
        let targets = engine.valid_transitions(&IssueStatus::ToDo);
        assert!(targets.contains(&IssueStatus::InProgress));
        assert!(targets.contains(&IssueStatus::Cancelled));
        assert!(!targets.contains(&IssueStatus::Done));
        assert!(!targets.contains(&IssueStatus::InReview));
    }

    #[test]
    fn test_validate_transition_ok() {
        let engine = WorkflowEngine::new();
        let issue = make_issue(IssueStatus::ToDo);
        let result =
            engine.validate_transition(&issue, &IssueStatus::InProgress, &serde_json::Value::Null);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_transition_invalid_returns_err() {
        let engine = WorkflowEngine::new();
        let issue = make_issue(IssueStatus::ToDo);
        let result =
            engine.validate_transition(&issue, &IssueStatus::Done, &serde_json::Value::Null);
        assert!(result.is_err());
    }
}
