// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{Board, BoardColumn, BoardType, Issue};
use uuid::Uuid;

pub fn default_scrum_board(project_id: Uuid, project_name: &str) -> Board {
    Board {
        id: Uuid::new_v4(),
        project_id,
        name: format!("{} Scrum Board", project_name),
        board_type: BoardType::Scrum,
        columns: vec![
            BoardColumn { name: "To Do".to_string(), statuses: vec!["To Do".to_string()], wip_limit: None },
            BoardColumn { name: "In Progress".to_string(), statuses: vec!["In Progress".to_string()], wip_limit: Some(5) },
            BoardColumn { name: "In Review".to_string(), statuses: vec!["In Review".to_string()], wip_limit: Some(3) },
            BoardColumn { name: "Done".to_string(), statuses: vec!["Done".to_string()], wip_limit: None },
        ],
        backlog_enabled: true,
        current_sprint_id: None,
        created_at: chrono::Utc::now(),
    }
}

pub fn default_kanban_board(project_id: Uuid, project_name: &str) -> Board {
    Board {
        id: Uuid::new_v4(),
        project_id,
        name: format!("{} Kanban Board", project_name),
        board_type: BoardType::Kanban,
        columns: vec![
            BoardColumn { name: "To Do".to_string(), statuses: vec!["To Do".to_string()], wip_limit: None },
            BoardColumn { name: "In Progress".to_string(), statuses: vec!["In Progress".to_string()], wip_limit: Some(10) },
            BoardColumn { name: "Done".to_string(), statuses: vec!["Done".to_string()], wip_limit: None },
        ],
        backlog_enabled: false,
        current_sprint_id: None,
        created_at: chrono::Utc::now(),
    }
}

/// Get board view: each column with its issues.
pub fn board_view<'a>(board: &Board, issues: &[&'a Issue]) -> Vec<(String, Vec<&'a Issue>)> {
    board.columns.iter().map(|col| {
        let col_issues: Vec<&Issue> = issues.iter()
            .filter(|i| col.statuses.contains(&i.status))
            .copied()
            .collect();
        (col.name.clone(), col_issues)
    }).collect()
}

/// Check WIP limit violations for a column.
pub fn check_wip_violations(board: &Board, issues: &[&Issue]) -> Vec<String> {
    let mut violations = Vec::new();
    for col in &board.columns {
        if let Some(limit) = col.wip_limit {
            let count = issues.iter().filter(|i| col.statuses.contains(&i.status)).count();
            if count > limit as usize {
                violations.push(format!("Column '{}': {} issues exceed WIP limit of {}", col.name, count, limit));
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_scrum_board_has_4_columns() {
        let board = default_scrum_board(Uuid::new_v4(), "Test");
        assert_eq!(board.columns.len(), 4);
    }

    #[test]
    fn test_kanban_board_no_backlog() {
        let board = default_kanban_board(Uuid::new_v4(), "Test");
        assert!(!board.backlog_enabled);
    }

    #[test]
    fn test_wip_check_no_violations() {
        let board = default_scrum_board(Uuid::new_v4(), "Test");
        let violations = check_wip_violations(&board, &[]);
        assert!(violations.is_empty());
    }
}
