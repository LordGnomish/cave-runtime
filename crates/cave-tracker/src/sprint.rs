use crate::models::{Issue, Sprint, SprintState};
use uuid::Uuid;

/// Convert SprintState to a human-readable string.
fn sprint_state_to_str(state: &SprintState) -> &'static str {
    match state {
        SprintState::Future => "Future",
        SprintState::Active => "Active",
        SprintState::Closed => "Closed",
    }
}

pub fn start_sprint(sprint: &mut Sprint) -> Result<(), String> {
    if sprint.state != SprintState::Future {
        return Err(format!(
            "Sprint '{}' is not in Future state (current: {:?})",
            sprint.name, sprint.state
        ));
    }
    sprint.state = SprintState::Active;
    sprint.start_date = Some(chrono::Utc::now());
    Ok(())
}

pub fn complete_sprint(sprint: &mut Sprint, issues: &[&Issue]) -> Result<(), String> {
    if sprint.state != SprintState::Active {
        return Err(format!("Sprint '{}' is not Active", sprint.name));
    }
    let velocity: f64 = issues
        .iter()
         .filter(|i| i.resolution.is_some())
         .filter_map(|i| i.story_points)
         .sum();
    sprint.state = SprintState::Closed;
    sprint.velocity = Some(velocity);
    sprint.completed_at = Some(chrono::Utc::now());
    Ok(())
}

pub fn sprint_stats(sprint: &Sprint, issues: &[&Issue]) -> serde_json::Value {
    let total = issues.len();
    let done = issues.iter().filter(|i| i.resolution.is_some()).count();
    let in_progress = issues
         .iter()
         .filter(|i| i.resolution.is_none() && i.status != "To Do" && i.status != "Backlog")
         .count();
    let todo = total - done - in_progress;
    let total_points: f64 = issues.iter().filter_map(|i| i.story_points).sum();
    let done_points: f64 = issues
         .iter()
         .filter(|i| i.resolution.is_some())
         .filter_map(|i| i.story_points)
         .sum();
    serde_json::json!({
         "sprint_id": sprint.id,
         "sprint_name": sprint.name,
         "state": sprint_state_to_str(&sprint.state),
         "total_issues": total,
         "done": done,
         "in_progress": in_progress,
         "todo": todo,
         "total_story_points": total_points,
         "completed_story_points": done_points,
         "velocity": sprint.velocity,
     })
}

/// Get issues not in any sprint (backlog).
/// Plane-equivalent: GET /api/v1/sprints/{id}/backlog
pub fn backlog_issues<'a>(
    issues: impl Iterator<Item = &'a Issue>,
    project_id: Uuid,
) -> Vec<&'a Issue> {
    // TODO: implement backlog filtering
    unimplemented!("backlog_issues not yet implemented")
}

/// Return only issues that are assigned to the given sprint.
/// Plane-equivalent: GET /api/v1/issues?sprint_id=<id>
pub fn get_issues_in_sprint<'a>(issues: &'a [&Issue], sprint_id: Uuid) -> Vec<&'a Issue> {
    // TODO: implement sprint-based filtering
    unimplemented!("get_issues_in_sprint not yet implemented")
}

/// Burndown data for a sprint: estimated, completed, remaining story points.
/// Plane-equivalent endpoint: GET /api/v1/sprints/{id}/burndown
pub fn sprint_burndown_data(sprint: &Sprint, issues: &[&Issue]) -> SprintBurndownData {
    // TODO: implement burndown calculation
    unimplemented!("sprint_burndown_data not yet implemented")
}

/// Sprint status / summary — Plane-equivalent: GET /api/v1/sprints/{id}/status
pub fn get_sprint_status(sprint: &Sprint, issues: &[&Issue]) -> SprintStatus {
    // TODO: implement sprint status computation
    unimplemented!("get_sprint_status not yet implemented")
}

/// Burndown summary returned by `sprint_burndown_data`.
#[derive(Debug, Clone)]
pub struct SprintBurndownData {
    pub estimated_effort: f64,
    pub completed_effort: f64,
    pub remaining_effort: f64,
}

/// Sprint status returned by `get_sprint_status`.
#[derive(Debug, Clone)]
pub struct SprintStatus {
    pub total: usize,
    pub done: usize,
    pub in_progress: usize,
    pub backlog: usize,
    pub completed_points: f64,
    pub status: String,
    pub velocity: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_sprint(state: SprintState) -> Sprint {
        Sprint {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            board_id: Uuid::new_v4(),
            name: "Sprint 1".to_string(),
            goal: None,
            state,
            start_date: None,
            end_date: None,
            completed_at: None,
            velocity: None,
            created_at: Utc::now(),
         }
     }

    fn make_issue(
        issue_id: Uuid,
        project_id: Uuid,
        project_key: &str,
        sprint_id: Option<Uuid>,
        story_points: Option<f64>,
        resolution: Option<String>,
        status: &str,
    ) -> Issue {
        Issue {
            id: issue_id,
            key: format!("TEST-{}", issue_id.hyphenated().to_string()[0..4].len() + 1),
            project_id,
            project_key: project_key.to_string(),
            issue_type: crate::IssueType::Task,
            summary: format!("Test issue {}", issue_id),
            description: None,
            status: status.to_string(),
            priority: crate::Priority::Medium,
            assignee: None,
            reporter: "admin".to_string(),
            labels: vec![],
            components: vec![],
            fix_versions: vec![],
            affects_versions: vec![],
            epic_id: None,
            parent_id: None,
            sprint_id,
            story_points,
            time_estimate_seconds: None,
            time_spent_seconds: 0,
            custom_fields: HashMap::new(),
            watchers: vec![],
            votes: 0,
            rank: 0,
            resolution,
            due_date: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            resolved_at: None,
         }
     }

     // --- Existing tests (preserved) ---

     #[test]
    fn test_start_sprint() {
        let mut sprint = make_sprint(SprintState::Future);
        assert!(start_sprint(&mut sprint).is_ok());
        assert_eq!(sprint.state, SprintState::Active);
        assert!(sprint.start_date.is_some());
     }

     #[test]
    fn test_start_active_sprint_fails() {
        let mut sprint = make_sprint(SprintState::Active);
        assert!(start_sprint(&mut sprint).is_err());
     }

     #[test]
    fn test_complete_sprint() {
        let mut sprint = make_sprint(SprintState::Active);
        let result = complete_sprint(&mut sprint, &[]);
        assert!(result.is_ok());
        assert_eq!(sprint.state, SprintState::Closed);
     }

     // --- Tests for NEW Plane-equivalent functions ---

     /// Verify backlog_issues returns only issues without sprint assignment.
     #[test]
    fn test_backlog_issues_filters_by_project() {
        let pid = Uuid::new_v4();
        let issue_no_sprint = make_issue(
            Uuid::new_v4(),
            pid,
             "TEST",
            None,
            None,
            None,
             "To Do",
         );
        let issue_with_sprint = make_issue(
            Uuid::new_v4(),
            pid,
             "TEST",
            Some(Uuid::new_v4()),
            None,
            None,
             "To Do",
         );
        let other_project = make_issue(
            Uuid::new_v4(),
            Uuid::new_v4(),
             "OTHER",
            None,
            None,
            None,
             "To Do",
         );
        let issues: Vec<&Issue> =
            vec![&issue_no_sprint, &issue_with_sprint, &other_project];
        let result = backlog_issues(issues.iter().copied(), pid);
        // Should return exactly 1: the one with no sprint and matching project
        assert_eq!(result.len(), 1);
     }

     /// Verify backlog_issues returns empty when all issues are sprinted.
     #[test]
    fn test_backlog_issues_returns_empty_for_no_backlog() {
        let pid = Uuid::new_v4();
        let sprint_id = Uuid::new_v4();
        let issue = make_issue(
            Uuid::new_v4(),
            pid,
             "TEST",
            Some(sprint_id),
            None,
            None,
             "To Do",
         );
        let result = backlog_issues(vec![&issue].iter().copied(), pid);
        assert!(result.is_empty());
     }

     /// Verify get_issues_in_sprint returns only issues assigned to the given sprint.
     #[test]
    fn test_get_issues_in_sprint_filters_correctly() {
        let sprint_id = Uuid::new_v4();
        let other_sprint = Uuid::new_v4();
        let issues = vec![
            make_issue(
                Uuid::new_v4(),
                sprint_id,
                 "TEST",
                Some(sprint_id),
                Some(3.0),
                None,
                 "To Do",
             ),
            make_issue(
                Uuid::new_v4(),
                sprint_id,
                 "TEST",
                None,
                None,
                None,
                 "To Do",
             ),
            make_issue(
                Uuid::new_v4(),
                sprint_id,
                 "TEST",
                Some(sprint_id),
                Some(5.0),
                Some("Fixed".to_string()),
                 "Done",
             ),
            make_issue(
                Uuid::new_v4(),
                sprint_id,
                 "TEST",
                Some(other_sprint),
                Some(2.0),
                None,
                 "In Progress",
             ),
         ];
        let ref_issues: Vec<&Issue> = issues.iter().collect();
        let result = get_issues_in_sprint(&ref_issues, sprint_id);
        assert_eq!(result.len(), 2);
        for issue in &result {
            assert_eq!(issue.sprint_id, Some(sprint_id));
         }
     }

     /// Verify get_issues_in_sprint returns empty when no issues match.
     #[test]
    fn test_get_issues_in_sprint_empty_result() {
        let sprint_id = Uuid::new_v4();
        let issued = make_issue(
            Uuid::new_v4(),
            sprint_id,
             "TEST",
            Some(Uuid::new_v4()),
            None,
            None,
             "To Do",
         );
        let ref_issues: Vec<&Issue> = vec![&issued];
        let result = get_issues_in_sprint(&ref_issues, sprint_id);
        assert!(result.is_empty());
     }

     /// Verify sprint_burndown_data returns correct estimated/remaining points.
     #[test]
    fn test_sprint_burndown_data() {
        let sprint = make_sprint(SprintState::Active);
        let done = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(8.0),
            Some("Fixed".to_string()),
             "Done",
         );
        let progress = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(5.0),
            None,
             "In Progress",
         );
        let backlog = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(3.0),
            None,
             "To Do",
         );
        let issues: Vec<&Issue> = vec![&done, &progress, &backlog];
        let data = sprint_burndown_data(&sprint, &issues);
        assert_eq!(data.estimated_effort, 16.0);
        assert_eq!(data.completed_effort, 8.0);
        assert_eq!(data.remaining_effort, 8.0);
     }

     /// Verify burndown returns zero values when no issues assigned to sprint.
     #[test]
    fn test_sprint_burndown_unassigned_issues() {
        let sprint = make_sprint(SprintState::Active);
        let issue = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            None,
            Some(10.0),
            None,
             "To Do",
         );
        let issues: Vec<&Issue> = vec![&issue];
        let data = sprint_burndown_data(&sprint, &issues);
        assert_eq!(data.estimated_effort, 0.0);
        assert_eq!(data.completed_effort, 0.0);
        assert_eq!(data.remaining_effort, 0.0);
     }

     /// Verify get_sprint_status returns correct full info.
     #[test]
    fn test_get_sprint_status_full_info() {
        let sprint = make_sprint(SprintState::Active);
        let done = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(8.0),
            Some("Fixed".to_string()),
             "Done",
         );
        let progress = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(5.0),
            None,
             "In Progress",
         );
        let todo = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(2.0),
            None,
             "To Do",
         );
        let issues: Vec<&Issue> = vec![&done, &progress, &todo];
        let status = get_sprint_status(&sprint, &issues);
        assert_eq!(status.total, 3);
        assert_eq!(status.done, 1);
        assert_eq!(status.in_progress, 1);
        assert_eq!(status.backlog, 1);
        assert_eq!(status.completed_points, 8.0);
     }

     /// Verify get_sprint_status counts correctly for a closed sprint.
     #[test]
    fn test_get_sprint_status_closed_sprint() {
        let mut sprint = make_sprint(SprintState::Active);
        sprint.state = SprintState::Closed;
        sprint.velocity = Some(12.0);
        let done = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(10.0),
            Some("Fixed".to_string()),
             "Done",
         );
        let undone = make_issue(
            Uuid::new_v4(),
            sprint.project_id,
             "TEST",
            Some(sprint.id),
            Some(6.0),
            None,
             "In Progress",
         );
        let issues: Vec<&Issue> = vec![&done, &undone];
        let status = get_sprint_status(&sprint, &issues);
        assert_eq!(status.total, 2);
        assert_eq!(status.status, "Closed");
        assert_eq!(status.completed_points, 10.0);
        assert_eq!(status.velocity, Some(12.0));
     }
}
