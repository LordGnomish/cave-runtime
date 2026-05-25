// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JQL-like query language for filtering issues.
use crate::models::{Issue, IssueType, Priority};

#[derive(Debug, Clone)]
pub struct IssueFilter {
    pub project_key: Option<String>,
    pub issue_type: Option<IssueType>,
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub priority: Option<Priority>,
    pub label: Option<String>,
    pub sprint_id: Option<uuid::Uuid>,
    pub epic_id: Option<uuid::Uuid>,
    pub text_search: Option<String>, // searches summary and description
    pub unresolved: Option<bool>,
    pub order_by: Option<OrderBy>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl Default for IssueFilter {
    fn default() -> Self {
        Self {
            project_key: None,
            issue_type: None,
            status: None,
            assignee: None,
            reporter: None,
            priority: None,
            label: None,
            sprint_id: None,
            epic_id: None,
            text_search: None,
            unresolved: None,
            order_by: None,
            limit: None,
            offset: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum OrderBy {
    CreatedAsc,
    CreatedDesc,
    UpdatedDesc,
    Priority,
    Rank,
    StoryPoints,
}

pub fn apply_filter<'a>(
    issues: impl Iterator<Item = &'a Issue>,
    filter: &IssueFilter,
) -> Vec<&'a Issue> {
    let mut results: Vec<&Issue> = issues
        .filter(|issue| {
            if let Some(ref pk) = filter.project_key {
                if &issue.project_key != pk {
                    return false;
                }
            }
            if let Some(ref it) = filter.issue_type {
                if &issue.issue_type != it {
                    return false;
                }
            }
            if let Some(ref st) = filter.status {
                if &issue.status != st {
                    return false;
                }
            }
            if let Some(ref a) = filter.assignee {
                if issue.assignee.as_deref() != Some(a.as_str()) {
                    return false;
                }
            }
            if let Some(ref r) = filter.reporter {
                if &issue.reporter != r {
                    return false;
                }
            }
            if let Some(ref p) = filter.priority {
                if &issue.priority != p {
                    return false;
                }
            }
            if let Some(ref l) = filter.label {
                if !issue.labels.contains(l) {
                    return false;
                }
            }
            if let Some(sid) = filter.sprint_id {
                if issue.sprint_id != Some(sid) {
                    return false;
                }
            }
            if let Some(eid) = filter.epic_id {
                if issue.epic_id != Some(eid) {
                    return false;
                }
            }
            if let Some(unresolved) = filter.unresolved {
                if unresolved && issue.resolution.is_some() {
                    return false;
                }
                if !unresolved && issue.resolution.is_none() {
                    return false;
                }
            }
            if let Some(ref text) = filter.text_search {
                let text_lower = text.to_lowercase();
                let in_summary = issue.summary.to_lowercase().contains(&text_lower);
                let in_desc = issue
                    .description
                    .as_deref()
                    .map(|d| d.to_lowercase().contains(&text_lower))
                    .unwrap_or(false);
                if !in_summary && !in_desc {
                    return false;
                }
            }
            true
        })
        .collect();

    // Sort
    match filter.order_by.as_ref().unwrap_or(&OrderBy::CreatedDesc) {
        OrderBy::CreatedAsc => results.sort_by(|a, b| a.created_at.cmp(&b.created_at)),
        OrderBy::CreatedDesc => results.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        OrderBy::UpdatedDesc => results.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        OrderBy::Priority => results.sort_by(|a, b| a.priority.cmp(&b.priority)),
        OrderBy::Rank => results.sort_by(|a, b| a.rank.cmp(&b.rank)),
        OrderBy::StoryPoints => results.sort_by(|a, b| {
            b.story_points
                .partial_cmp(&a.story_points)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }

    // Paginate
    let offset = filter.offset.unwrap_or(0);
    let limit = filter.limit.unwrap_or(50);
    results.into_iter().skip(offset).take(limit).collect()
}

/// Parse a simple JQL-like query string into an IssueFilter.
/// Supports: project=X, status=Y, assignee=Z, type=T, priority=P, label=L, text~"search"
pub fn parse_jql(jql: &str) -> IssueFilter {
    let mut filter = IssueFilter::default();
    for part in jql.split(" AND ") {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().trim_matches('"').to_string();
            match key.as_str() {
                "project" => filter.project_key = Some(value),
                "status" => filter.status = Some(value),
                "assignee" => filter.assignee = Some(value),
                "reporter" => filter.reporter = Some(value),
                "priority" => {
                    filter.priority = match value.to_lowercase().as_str() {
                        "critical" => Some(Priority::Critical),
                        "high" => Some(Priority::High),
                        "medium" => Some(Priority::Medium),
                        "low" => Some(Priority::Low),
                        "trivial" => Some(Priority::Trivial),
                        _ => None,
                    }
                }
                "type" | "issuetype" => {
                    filter.issue_type = match value.to_lowercase().as_str() {
                        "epic" => Some(IssueType::Epic),
                        "story" => Some(IssueType::Story),
                        "task" => Some(IssueType::Task),
                        "bug" => Some(IssueType::Bug),
                        "subtask" => Some(IssueType::Subtask),
                        _ => None,
                    }
                }
                "label" => filter.label = Some(value),
                "unresolved" => filter.unresolved = Some(value == "true"),
                _ => {}
            }
        } else if let Some(rest) = part.strip_prefix("text~") {
            filter.text_search = Some(rest.trim_matches('"').to_string());
        }
    }
    filter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_jql_project() {
        let f = parse_jql("project=CAVE AND status=Done");
        assert_eq!(f.project_key, Some("CAVE".to_string()));
        assert_eq!(f.status, Some("Done".to_string()));
    }

    #[test]
    fn test_parse_jql_priority() {
        let f = parse_jql("priority=high");
        assert_eq!(f.priority, Some(Priority::High));
    }

    #[test]
    fn test_apply_filter_by_status() {
        let issues = vec![
            make_issue("CAVE-1", "In Progress"),
            make_issue("CAVE-2", "Done"),
        ];
        let filter = IssueFilter {
            status: Some("Done".to_string()),
            ..Default::default()
        };
        let results = apply_filter(issues.iter(), &filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "CAVE-2");
    }

    #[test]
    fn test_text_search() {
        let issues = vec![make_issue("CAVE-1", "To Do"), make_issue("CAVE-2", "To Do")];
        let filter = IssueFilter {
            text_search: Some("CAVE-1".to_string()),
            ..Default::default()
        };
        // text search looks at summary which has the key embedded
        let results = apply_filter(issues.iter(), &filter);
        assert_eq!(results.len(), 1);
    }

    fn make_issue(key: &str, status: &str) -> Issue {
        use std::collections::HashMap;
        Issue {
            id: uuid::Uuid::new_v4(),
            key: key.to_string(),
            project_id: uuid::Uuid::new_v4(),
            project_key: "CAVE".to_string(),
            issue_type: IssueType::Task,
            summary: format!("Issue {}", key),
            description: None,
            status: status.to_string(),
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
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            resolved_at: None,
        }
    }
}
