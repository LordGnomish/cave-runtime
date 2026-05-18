// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{Issue, Sprint, SprintState};
use uuid::Uuid;

pub fn start_sprint(sprint: &mut Sprint) -> Result<(), String> {
    if sprint.state != SprintState::Future {
        return Err(format!("Sprint '{}' is not in Future state (current: {:?})", sprint.name, sprint.state));
    }
    sprint.state = SprintState::Active;
    sprint.start_date = Some(chrono::Utc::now());
    Ok(())
}

pub fn complete_sprint(sprint: &mut Sprint, issues: &[&Issue]) -> Result<(), String> {
    if sprint.state != SprintState::Active {
        return Err(format!("Sprint '{}' is not Active", sprint.name));
    }
    let velocity: f64 = issues.iter()
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
    let in_progress = issues.iter().filter(|i| i.resolution.is_none() && i.status != "To Do" && i.status != "Backlog").count();
    let todo = total - done - in_progress;
    let total_points: f64 = issues.iter().filter_map(|i| i.story_points).sum();
    let done_points: f64 = issues.iter().filter(|i| i.resolution.is_some()).filter_map(|i| i.story_points).sum();
    serde_json::json!({
        "sprint_id": sprint.id,
        "sprint_name": sprint.name,
        "state": sprint.state,
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
pub fn backlog_issues<'a>(issues: impl Iterator<Item = &'a Issue>, project_id: Uuid) -> Vec<&'a Issue> {
    issues.filter(|i| i.project_id == project_id && i.sprint_id.is_none()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_sprint(state: SprintState) -> Sprint {
        Sprint {
            id: Uuid::new_v4(), project_id: Uuid::new_v4(), board_id: Uuid::new_v4(),
            name: "Sprint 1".to_string(), goal: None, state,
            start_date: None, end_date: None, completed_at: None, velocity: None,
            created_at: Utc::now(),
        }
    }

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
}
