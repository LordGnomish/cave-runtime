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
    issues
        .filter(|i| i.project_id == project_id && i.sprint_id.is_none())
        .collect()
}

/// Return only issues that are assigned to the given sprint.
/// Plane-equivalent: GET /api/v1/issues?sprint_id=<id>
pub fn get_issues_in_sprint<'a>(issues: &'a [&Issue], sprint_id: Uuid) -> Vec<&'a Issue> {
    issues.iter().copied().filter(|i| i.sprint_id == Some(sprint_id)).collect()
}

/// Burndown data for a sprint: estimated, completed, remaining story points.
/// Plane-equivalent endpoint: GET /api/v1/sprints/{id}/burndown
pub fn sprint_burndown_data(sprint: &Sprint, issues: &[&Issue]) -> SprintBurndownData {
    let sprint_issues: Vec<&Issue> = issues
        .iter()
        .copied()
        .filter(|i| i.sprint_id == Some(sprint.id))
        .collect();
    let estimated: f64 = sprint_issues.iter().filter_map(|i| i.story_points).sum();
    let completed: f64 = sprint_issues
        .iter()
        .filter(|i| i.resolution.is_some())
        .filter_map(|i| i.story_points)
        .sum();
    let remaining = estimated - completed;
    SprintBurndownData {
        estimated_effort: estimated,
        completed_effort: completed,
        remaining_effort: remaining,
    }
}

/// Sprint status / summary — Plane-equivalent: GET /api/v1/sprints/{id}/status
pub fn get_sprint_status(sprint: &Sprint, issues: &[&Issue]) -> SprintStatus {
    let sprint_issues: Vec<&Issue> = issues
        .iter()
        .copied()
        .filter(|i| i.sprint_id == Some(sprint.id))
        .collect();
    let total = sprint_issues.len();
    let done = sprint_issues.iter().filter(|i| i.resolution.is_some()).count();
    let in_progress = sprint_issues
         .iter()
         .filter(|i| i.resolution.is_none() && i.status != "Backlog" && i.status != "To Do")
         .count();
    let backlog = total - done - in_progress;
    SprintStatus {
        total,
        done,
        in_progress,
        backlog,
        completed_points: sprint_issues
            .iter()
            .filter(|i| i.resolution.is_some())
            .filter_map(|i| i.story_points)
            .sum(),
        status: sprint_state_to_str(&sprint.state).to_string(),
        velocity: sprint.velocity,
    }
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

    // --- Additional edge-case tests added in GREEN commit ---

    /// Test sprint_stats with zero issues.
    #[test]
    fn test_sprint_stats_empty_issues() {
        let sprint = make_sprint(SprintState::Future);
        let stats = sprint_stats(&sprint, &[]);
        assert_eq!(stats["total_issues"].as_u64().unwrap(), 0);
        assert_eq!(stats["done"].as_u64().unwrap(), 0);
        assert_eq!(stats["in_progress"].as_u64().unwrap(), 0);
        assert_eq!(stats["todo"].as_u64().unwrap(), 0);
        assert_eq!(stats["total_story_points"].as_f64().unwrap(), 0.0);
        assert_eq!(stats["completed_story_points"].as_f64().unwrap(), 0.0);
    }

    /// Test backlog_issues returns all when none have sprint ids.
    #[test]
    fn test_backlog_issues_all_backlog() {
        let pid = Uuid::new_v4();
        let issue1 = make_issue(Uuid::new_v4(), pid, "A", None, None, None, "Backlog");
        let issue2 = make_issue(Uuid::new_v4(), pid, "B", None, Some(5.0), None, "To Do");
        let issues: Vec<&Issue> = vec![&issue1, &issue2];
        let result = backlog_issues(issues.iter().copied(), pid);
        assert_eq!(result.len(), 2);
    }

    /// Test backlog_issues across multiple projects.
    #[test]
    fn test_backlog_issues_multi_project() {
        let pid_a = Uuid::new_v4();
        let pid_b = Uuid::new_v4();
        let issue_a = make_issue(Uuid::new_v4(), pid_a, "A", None, None, None, "To Do");
        let issue_b = make_issue(Uuid::new_v4(), pid_b, "B", None, None, None, "To Do");
        let sprinted = make_issue(Uuid::new_v4(), pid_a, "C", Some(Uuid::new_v4()), None, None, "To Do");
        let issues: Vec<&Issue> = vec![&issue_a, &issue_b, &sprinted];
        let result = backlog_issues(issues.iter().copied(), pid_a);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, issue_a.key);
    }

    /// Test get_issues_in_sprint with multiple sprints.
    #[test]
    fn test_get_issues_in_sprint_multi_sprint() {
        let sprint_a = Uuid::new_v4();
        let sprint_b = Uuid::new_v4();
        let issues = vec![
            make_issue(Uuid::new_v4(), sprint_a, "A1", Some(sprint_a), None, None, "To Do"),
            make_issue(Uuid::new_v4(), sprint_a, "A2", Some(sprint_a), None, None, "Done"),
            make_issue(Uuid::new_v4(), sprint_b, "B1", Some(sprint_b), None, None, "To Do"),
            make_issue(Uuid::new_v4(), sprint_b, "B2", Some(sprint_b), None, None, "Done"),
        ];
        let ref_issues: Vec<&Issue> = issues.iter().collect();
        let result_a = get_issues_in_sprint(&ref_issues, sprint_a);
        let result_b = get_issues_in_sprint(&ref_issues, sprint_b);
        assert_eq!(result_a.len(), 2);
        assert_eq!(result_b.len(), 2);
    }

    /// Test sprint_burndown_data with no story points.
    #[test]
    fn test_sprint_burndown_zero_story_points() {
        let sprint = make_sprint(SprintState::Active);
        let issue1 = make_issue(Uuid::new_v4(), sprint.project_id, "A", Some(sprint.id), None, Some("Fixed".to_string()), "Done");
        let issue2 = make_issue(Uuid::new_v4(), sprint.project_id, "B", Some(sprint.id), None, None, "To Do");
        let issues: Vec<&Issue> = vec![&issue1, &issue2];
        let data = sprint_burndown_data(&sprint, &issues);
        assert_eq!(data.estimated_effort, 0.0);
        assert_eq!(data.completed_effort, 0.0);
        assert_eq!(data.remaining_effort, 0.0);
    }

    /// Test sprint_burndown_data all issues resolved.
    #[test]
    fn test_sprint_burndown_all_completed() {
        let sprint = make_sprint(SprintState::Active);
        let issue1 = make_issue(Uuid::new_v4(), sprint.project_id, "A", Some(sprint.id), Some(3.0), Some("Fixed".to_string()), "Done");
        let issue2 = make_issue(Uuid::new_v4(), sprint.project_id, "B", Some(sprint.id), Some(7.0), Some("Fixed".to_string()), "Done");
        let issues: Vec<&Issue> = vec![&issue1, &issue2];
        let data = sprint_burndown_data(&sprint, &issues);
        assert_eq!(data.estimated_effort, 10.0);
        assert_eq!(data.completed_effort, 10.0);
        assert_eq!(data.remaining_effort, 0.0);
    }

    /// Test get_sprint_status with Future sprint state.
    #[test]
    fn test_get_sprint_status_future_state() {
        let mut sprint = make_sprint(SprintState::Future);
        let issue = make_issue(Uuid::new_v4(), sprint.project_id, "A", Some(sprint.id), Some(5.0), None, "Backlog");
        let issues: Vec<&Issue> = vec![&issue];
        let status = get_sprint_status(&sprint, &issues);
        assert_eq!(status.total, 1);
        assert_eq!(status.status, "Future");
        assert_eq!(status.done, 0);
        assert_eq!(status.in_progress, 0);
        assert_eq!(status.backlog, 1);
    }

    /// Test get_sprint_status with Closed sprint and zero completed points.
    #[test]
    fn test_get_sprint_status_closed_zero_velocity() {
        let mut sprint = make_sprint(SprintState::Active);
        sprint.state = SprintState::Closed;
        sprint.velocity = None;
        let issue = make_issue(Uuid::new_v4(), sprint.project_id, "A", Some(sprint.id), None, None, "To Do");
        let issues: Vec<&Issue> = vec![&issue];
        let status = get_sprint_status(&sprint, &issues);
        assert_eq!(status.total, 1);
        assert_eq!(status.velocity, None);
        assert_eq!(status.completed_points, 0.0);
    }

    /// Test sprint lifecycle: Future -> Active -> Closed via stats.
    #[test]
    fn test_sprint_lifecycle_stats() {
        let mut sprint = make_sprint(SprintState::Future);
        let issue_done = make_issue(Uuid::new_v4(), sprint.project_id, "D", Some(sprint.id), Some(5.0), Some("Fixed".to_string()), "Done");
        let issue_open = make_issue(Uuid::new_v4(), sprint.project_id, "O", Some(sprint.id), Some(3.0), None, "In Progress");
        let done_points = issue_done.story_points.unwrap_or(0.0);
        // Before start: stats show 0 total since no issues passed to stats
        let stats_future = sprint_stats(&sprint, &[]);
        assert_eq!(stats_future["total_issues"].as_u64().unwrap(), 0);
        // After start
        start_sprint(&mut sprint).unwrap();
        assert_eq!(sprint.state, SprintState::Active);
        // After close
        complete_sprint(&mut sprint, &[&issue_done, &issue_open]).unwrap();
        assert_eq!(sprint.state, SprintState::Closed);
        let stats_closed = sprint_stats(&sprint, &[&issue_done, &issue_open]);
        assert_eq!(stats_closed["velocity"].as_f64().unwrap(), done_points);
    }

    /// Test backlog_issues with mixed resolution statuses.
    #[test]
    fn test_backlog_issues_mixed_resolution() {
        let pid = Uuid::new_v4();
        let resolved = make_issue(Uuid::new_v4(), pid, "A", None, Some(5.0), Some("Fixed".to_string()), "Done");
        let unresolved = make_issue(Uuid::new_v4(), pid, "B", None, None, None, "To Do");
        let sprinted = make_issue(Uuid::new_v4(), pid, "C", Some(Uuid::new_v4()), Some(3.0), None, "In Progress");
        let issues: Vec<&Issue> = vec![&resolved, &unresolved, &sprinted];
        let result = backlog_issues(issues.iter().copied(), pid);
        assert_eq!(result.len(), 2);
        let keys: Vec<String> = result.iter().map(|i| i.key.clone()).collect();
        assert!(keys.contains(&resolved.key));
        assert!(keys.contains(&unresolved.key));
    }

    /// Test complete_sprint on a Future sprint should fail.
    #[test]
    fn test_complete_future_sprint_fails() {
        let mut sprint = make_sprint(SprintState::Future);
        let result = complete_sprint(&mut sprint, &[]);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Active"));
    }

    /// Test complete_sprint sets velocity even with zero story points.
    #[test]
    fn test_complete_sprint_zero_velocity() {
        let mut sprint = make_sprint(SprintState::Active);
        let issue = make_issue(Uuid::new_v4(), sprint.project_id, "A", Some(sprint.id), None, Some("Fixed".to_string()), "Done");
        complete_sprint(&mut sprint, &[&issue]).unwrap();
        assert_eq!(sprint.velocity, Some(0.0));
        assert!(sprint.completed_at.is_some());
    }

    /// Test sprint_burndown_data with mixed sprint assignments.
    #[test]
    fn test_sprint_burndown_mixed_sprint_assignments() {
        let sprint = make_sprint(SprintState::Active);
        let sprint2 = Uuid::new_v4();
        let in_sprint = make_issue(Uuid::new_v4(), sprint.project_id, "A", Some(sprint.id), Some(5.0), None, "To Do");
        let not_in_sprint = make_issue(Uuid::new_v4(), sprint.project_id, "B", None, Some(10.0), None, "To Do");
        let in_other_sprint = make_issue(Uuid::new_v4(), sprint2, "C", Some(sprint2), Some(3.0), None, "To Do");
        let issues: Vec<&Issue> = vec![&in_sprint, &not_in_sprint, &in_other_sprint];
        let data = sprint_burndown_data(&sprint, &issues);
        assert_eq!(data.estimated_effort, 5.0);
        assert_eq!(data.completed_effort, 0.0);
        assert_eq!(data.remaining_effort, 5.0);
    }

    /// Test get_sprint_status with Backlog status excluded from in_progress.
    #[test]
    fn test_get_sprint_status_backlog_not_in_progress() {
        let mut sprint = make_sprint(SprintState::Active);
        let backlog_issue = make_issue(Uuid::new_v4(), sprint.project_id, "BACK", Some(sprint.id), None, None, "Backlog");
        let open_issue = make_issue(Uuid::new_v4(), sprint.project_id, "OPEN", Some(sprint.id), None, None, "To Do");
        let progress_issue = make_issue(Uuid::new_v4(), sprint.project_id, "PROG", Some(sprint.id), Some(2.0), None, "In Progress");
        let issues: Vec<&Issue> = vec![&backlog_issue, &open_issue, &progress_issue];
        let status = get_sprint_status(&sprint, &issues);
        assert_eq!(status.total, 3);
        assert_eq!(status.in_progress, 1); // only In Progress, not Backlog or To Do
        assert_eq!(status.backlog, 2);      // Backlog + To Do
    }

    /// Test start_sprint sets start_date correctly.
    #[test]
    fn test_start_sprint_sets_date() {
        let mut sprint = make_sprint(SprintState::Future);
        let before = chrono::Utc::now();
        start_sprint(&mut sprint).unwrap();
        let after = chrono::Utc::now();
        assert!(sprint.start_date.is_some());
        let start_date = sprint.start_date.unwrap();
        assert!(start_date >= before && start_date <= after);
    }

    /// Verify sprint_stats velocity is None before sprint completion.
    #[test]
    fn test_sprint_stats_velocity_before_closure() {
        let mut sprint = make_sprint(SprintState::Active);
        let binding = make_issue(Uuid::new_v4(), sprint.project_id, "A", Some(sprint.id), Some(3.0), None, "To Do");
        let issues = vec![&binding];
        let before = sprint_stats(&sprint, &issues);
        assert_eq!(before["velocity"].as_f64(), None);
        complete_sprint(&mut sprint, &issues).unwrap();
        let after = sprint_stats(&sprint, &issues);
        assert_eq!(after["velocity"].as_f64(), Some(0.0)); // no issues resolved
    }
}
