// SPDX-License-Identifier: AGPL-3.0-or-later
//! Simple JQL-like query language for cave-tracker.
//!
//! Supported:
//! - `project = "KEY"` / `project = KEY`
//! - `status = "In Progress"` / `status IN ("In Progress", "Done")`
//! - `assignee = currentUser()` / `assignee = "uuid"`
//! - `priority = P1`
//! - `type = Bug` / `issueType = Story`
//! - `label = "frontend"` / `labels IN ("frontend", "backend")`
//! - `sprint = "Sprint 1"` / `sprint IS EMPTY` / `sprint IS NOT EMPTY`
//! - `AND`, `OR` operators
//! - `ORDER BY field ASC/DESC`

use crate::models::{Issue, Priority};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum JqlValue {
    String(String),
    CurrentUser,
    Number(f64),
}

#[derive(Debug, Clone)]
pub enum JqlCondition {
    Equals { field: String, value: JqlValue },
    In { field: String, values: Vec<JqlValue> },
    IsEmpty { field: String },
    IsNotEmpty { field: String },
    And(Box<JqlCondition>, Box<JqlCondition>),
    Or(Box<JqlCondition>, Box<JqlCondition>),
}

#[derive(Debug)]
pub struct OrderBy {
    pub field: String,
    pub ascending: bool,
}

#[derive(Debug)]
pub struct ParsedJql {
    pub condition: Option<JqlCondition>,
    pub order_by: Vec<OrderBy>,
}

pub struct JqlParser;

impl JqlParser {
    /// Parse a JQL query string into a `ParsedJql`.
    pub fn parse(query: &str) -> Result<ParsedJql, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(ParsedJql {
                condition: None,
                order_by: vec![],
            });
        }

        // Split off ORDER BY clause first
        let (where_part, order_part) = split_order_by(query);
        let order_by = parse_order_by(order_part)?;
        let condition = if where_part.trim().is_empty() {
            None
        } else {
            Some(parse_condition(where_part.trim())?)
        };

        Ok(ParsedJql { condition, order_by })
    }
}

/// Split query into `(where_clause, order_by_clause)`.
fn split_order_by(query: &str) -> (&str, &str) {
    // Case-insensitive search for "ORDER BY"
    let upper = query.to_uppercase();
    if let Some(pos) = upper.find("ORDER BY") {
        (&query[..pos], &query[pos + 8..])
    } else {
        (query, "")
    }
}

fn parse_order_by(s: &str) -> Result<Vec<OrderBy>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec![]);
    }

    let mut result = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let field = tokens[0].to_string();
        let ascending = if tokens.len() >= 2 {
            match tokens[1].to_uppercase().as_str() {
                "ASC" => true,
                "DESC" => false,
                other => return Err(format!("Unknown sort direction: {other}")),
            }
        } else {
            true // default ASC
        };
        result.push(OrderBy { field, ascending });
    }
    Ok(result)
}

/// Parse a condition string, handling AND / OR at the top level.
fn parse_condition(s: &str) -> Result<JqlCondition, String> {
    // We process OR first (lowest precedence), then AND.
    parse_or(s)
}

fn parse_or(s: &str) -> Result<JqlCondition, String> {
    let parts = split_top_level(s, " OR ");
    if parts.len() == 1 {
        return parse_and(parts[0].trim());
    }
    let mut iter = parts.into_iter();
    let mut left = parse_and(iter.next().unwrap().trim())?;
    for part in iter {
        let right = parse_and(part.trim())?;
        left = JqlCondition::Or(Box::new(left), Box::new(right));
    }
    Ok(left)
}

fn parse_and(s: &str) -> Result<JqlCondition, String> {
    let parts = split_top_level(s, " AND ");
    if parts.len() == 1 {
        return parse_atom(parts[0].trim());
    }
    let mut iter = parts.into_iter();
    let mut left = parse_atom(iter.next().unwrap().trim())?;
    for part in iter {
        let right = parse_atom(part.trim())?;
        left = JqlCondition::And(Box::new(left), Box::new(right));
    }
    Ok(left)
}

/// Split `s` by `sep` only at the top level (not inside parentheses or quotes).
fn split_top_level<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let sep_upper = sep.to_uppercase();
    let s_upper = s.to_uppercase();
    let mut results = Vec::new();
    let mut depth = 0usize;
    let mut in_quote = false;
    let mut last = 0usize;
    let bytes = s.as_bytes();
    let sep_len = sep.len();
    let mut i = 0;

    while i < s.len() {
        let ch = bytes[i] as char;
        if ch == '"' || ch == '\'' {
            in_quote = !in_quote;
        } else if !in_quote {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth = depth.saturating_sub(1);
            } else if depth == 0
                && i + sep_len <= s.len()
                && s_upper[i..i + sep_len] == sep_upper[..]
            {
                results.push(&s[last..i]);
                last = i + sep_len;
                i += sep_len;
                continue;
            }
        }
        i += 1;
    }
    results.push(&s[last..]);
    results
}

fn parse_atom(s: &str) -> Result<JqlCondition, String> {
    // Strip surrounding parens
    let s = strip_outer_parens(s);

    // Handle IS EMPTY / IS NOT EMPTY
    let upper = s.to_uppercase();
    if let Some(rest) = upper.strip_suffix(" IS NOT EMPTY") {
        let field = rest.trim().to_lowercase();
        return Ok(JqlCondition::IsNotEmpty { field });
    }
    if let Some(rest) = upper.strip_suffix(" IS EMPTY") {
        let field = rest.trim().to_lowercase();
        return Ok(JqlCondition::IsEmpty { field });
    }

    // Handle IN (...)
    if let Some(in_pos) = find_keyword_in(s) {
        let field = s[..in_pos].trim().to_lowercase();
        // in_pos points to the leading space before "IN"; skip " IN " (4 chars)
        let rest = s[in_pos + 4..].trim();
        let values = parse_value_list(rest)?;
        return Ok(JqlCondition::In { field, values });
    }

    // Handle = operator
    if let Some(eq_pos) = s.find('=') {
        let field = s[..eq_pos].trim().to_lowercase();
        let value_str = s[eq_pos + 1..].trim();
        let value = parse_single_value(value_str)?;
        return Ok(JqlCondition::Equals { field, value });
    }

    Err(format!("Cannot parse JQL atom: {s}"))
}

/// Find position of ` IN ` keyword at the top level.
fn find_keyword_in(s: &str) -> Option<usize> {
    let upper = s.to_uppercase();
    // Look for whitespace + "IN" + whitespace
    let mut i = 0;
    while i + 4 <= upper.len() {
        if upper[i..].starts_with(" IN ") || upper[i..].starts_with(" IN\t") {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn strip_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if s.starts_with('(') && s.ends_with(')') {
        // verify it's balanced
        let inner = &s[1..s.len() - 1];
        let mut depth = 0i32;
        let mut balanced = true;
        for ch in inner.chars() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        balanced = false;
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if balanced && depth == 0 {
            return inner.trim();
        }
    }
    s
}

fn parse_value_list(s: &str) -> Result<Vec<JqlValue>, String> {
    let s = s.trim();
    let s = if s.starts_with('(') && s.ends_with(')') {
        &s[1..s.len() - 1]
    } else {
        s
    };

    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = ' ';

    for ch in s.chars() {
        match ch {
            '"' | '\'' if !in_quote => {
                in_quote = true;
                quote_char = ch;
            }
            c if in_quote && c == quote_char => {
                in_quote = false;
            }
            ',' if !in_quote => {
                let v = current.trim().to_string();
                if !v.is_empty() {
                    values.push(parse_single_value(&v)?);
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    let v = current.trim().to_string();
    if !v.is_empty() {
        values.push(parse_single_value(&v)?);
    }

    Ok(values)
}

fn parse_single_value(s: &str) -> Result<JqlValue, String> {
    let s = s.trim();
    // currentUser()
    if s.to_lowercase() == "currentuser()" {
        return Ok(JqlValue::CurrentUser);
    }
    // Quoted string
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        let inner = &s[1..s.len() - 1];
        return Ok(JqlValue::String(inner.to_string()));
    }
    // Number
    if let Ok(n) = s.parse::<f64>() {
        return Ok(JqlValue::Number(n));
    }
    // Bare word (e.g. P1, Bug, KEY)
    Ok(JqlValue::String(s.to_string()))
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

pub struct JqlEvaluator;

impl JqlEvaluator {
    /// Filter and sort `issues` according to `parsed`, substituting `current_user` for
    /// `currentUser()` references.
    pub fn evaluate(issues: &[Issue], parsed: &ParsedJql, current_user: Option<Uuid>) -> Vec<Issue> {
        let mut result: Vec<Issue> = issues
            .iter()
            .filter(|issue| {
                if let Some(cond) = &parsed.condition {
                    eval_condition(issue, cond, current_user)
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        // Apply ORDER BY clauses (last one has lowest priority → apply in reverse)
        for order in parsed.order_by.iter().rev() {
            let asc = order.ascending;
            let field = order.field.to_lowercase();
            result.sort_by(|a, b| {
                let ord = compare_issues_by_field(a, b, &field);
                if asc { ord } else { ord.reverse() }
            });
        }

        result
    }
}

fn eval_condition(issue: &Issue, cond: &JqlCondition, current_user: Option<Uuid>) -> bool {
    match cond {
        JqlCondition::Equals { field, value } => {
            eval_equals(issue, field, value, current_user)
        }
        JqlCondition::In { field, values } => {
            values.iter().any(|v| eval_equals(issue, field, v, current_user))
        }
        JqlCondition::IsEmpty { field } => eval_is_empty(issue, field),
        JqlCondition::IsNotEmpty { field } => !eval_is_empty(issue, field),
        JqlCondition::And(l, r) => {
            eval_condition(issue, l, current_user) && eval_condition(issue, r, current_user)
        }
        JqlCondition::Or(l, r) => {
            eval_condition(issue, l, current_user) || eval_condition(issue, r, current_user)
        }
    }
}

fn normalize_status(s: &str) -> String {
    s.to_lowercase()
        .replace(' ', "_")
        .replace('-', "_")
}

fn eval_equals(issue: &Issue, field: &str, value: &JqlValue, current_user: Option<Uuid>) -> bool {
    let value_str = match value {
        JqlValue::String(s) => s.clone(),
        JqlValue::CurrentUser => current_user.map(|u| u.to_string()).unwrap_or_default(),
        JqlValue::Number(n) => n.to_string(),
    };

    match field {
        "project" | "project_key" => {
            issue.project_key.to_uppercase() == value_str.to_uppercase()
        }
        "status" => {
            let issue_status = format!("{:?}", issue.status).to_lowercase();
            let normalized_value = normalize_status(&value_str);
            // Also compare against serde representation
            let serde_status = serde_json::to_string(&issue.status)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            issue_status == normalized_value
                || serde_status == normalized_value
                || normalize_status(&format!("{:?}", issue.status)) == normalized_value
        }
        "assignee" => match &issue.assignee {
            Some(id) => id.to_string() == value_str,
            None => false,
        },
        "reporter" => issue.reporter.to_string() == value_str,
        "priority" => {
            let serde_priority = serde_json::to_string(&issue.priority)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            serde_priority == value_str.to_lowercase()
                || format!("{:?}", issue.priority).to_lowercase() == value_str.to_lowercase()
        }
        "type" | "issuetype" | "issue_type" => {
            let serde_type = serde_json::to_string(&issue.issue_type)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            serde_type == value_str.to_lowercase()
                || format!("{:?}", issue.issue_type).to_lowercase() == value_str.to_lowercase()
        }
        "label" | "labels" => issue
            .labels
            .iter()
            .any(|l| l.to_lowercase() == value_str.to_lowercase()),
        "sprint" => match &issue.sprint_id {
            Some(id) => id.to_string() == value_str,
            None => false,
        },
        "epic" | "epic_id" => match &issue.epic_id {
            Some(id) => id.to_string() == value_str,
            None => false,
        },
        _ => false,
    }
}

fn eval_is_empty(issue: &Issue, field: &str) -> bool {
    match field {
        "assignee" => issue.assignee.is_none(),
        "sprint" | "sprint_id" => issue.sprint_id.is_none(),
        "epic" | "epic_id" => issue.epic_id.is_none(),
        "labels" => issue.labels.is_empty(),
        "description" => issue.description.is_none(),
        "story_points" => issue.story_points.is_none(),
        _ => false,
    }
}

fn compare_issues_by_field(a: &Issue, b: &Issue, field: &str) -> std::cmp::Ordering {
    match field {
        "priority" => {
            let pri_ord = |p: &Priority| match p {
                Priority::P1 => 1u8,
                Priority::P2 => 2,
                Priority::P3 => 3,
                Priority::P4 => 4,
                Priority::P5 => 5,
            };
            pri_ord(&a.priority).cmp(&pri_ord(&b.priority))
        }
        "created_at" => a.created_at.cmp(&b.created_at),
        "updated_at" => a.updated_at.cmp(&b.updated_at),
        "summary" => a.summary.cmp(&b.summary),
        "issue_number" => a.issue_number.cmp(&b.issue_number),
        "story_points" => {
            let ap = a.story_points.unwrap_or(0.0);
            let bp = b.story_points.unwrap_or(0.0);
            ap.partial_cmp(&bp).unwrap_or(std::cmp::Ordering::Equal)
        }
        _ => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IssueStatus, IssueType, Priority};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_issue(
        project: &str,
        status: IssueStatus,
        priority: Priority,
        issue_type: IssueType,
        labels: Vec<String>,
        assignee: Option<Uuid>,
        sprint_id: Option<Uuid>,
    ) -> Issue {
        Issue {
            id: Uuid::new_v4(),
            project_key: project.to_string(),
            issue_number: 1,
            issue_type,
            summary: "Test".to_string(),
            description: None,
            assignee,
            reporter: Uuid::new_v4(),
            priority,
            status,
            labels,
            components: vec![],
            sprint_id,
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
    fn test_parse_project_equals() {
        let parsed = JqlParser::parse("project = CAVE").unwrap();
        assert!(parsed.condition.is_some());
    }

    #[test]
    fn test_parse_status_in() {
        let parsed = JqlParser::parse(r#"status IN ("in_progress", "done")"#).unwrap();
        assert!(parsed.condition.is_some());
        if let Some(JqlCondition::In { field, values }) = parsed.condition {
            assert_eq!(field, "status");
            assert_eq!(values.len(), 2);
        } else {
            panic!("Expected In condition");
        }
    }

    #[test]
    fn test_parse_assignee_current_user() {
        let parsed = JqlParser::parse("assignee = currentUser()").unwrap();
        if let Some(JqlCondition::Equals { field, value }) = parsed.condition {
            assert_eq!(field, "assignee");
            assert!(matches!(value, JqlValue::CurrentUser));
        } else {
            panic!("Expected Equals condition");
        }
    }

    #[test]
    fn test_parse_and_condition() {
        let parsed = JqlParser::parse("project = CAVE AND status = in_progress").unwrap();
        assert!(matches!(parsed.condition, Some(JqlCondition::And(_, _))));
    }

    #[test]
    fn test_parse_order_by() {
        let parsed = JqlParser::parse("project = CAVE ORDER BY priority DESC").unwrap();
        assert_eq!(parsed.order_by.len(), 1);
        assert_eq!(parsed.order_by[0].field, "priority");
        assert!(!parsed.order_by[0].ascending);
    }

    #[test]
    fn test_parse_sprint_is_empty() {
        let parsed = JqlParser::parse("sprint IS EMPTY").unwrap();
        if let Some(JqlCondition::IsEmpty { field }) = parsed.condition {
            assert_eq!(field, "sprint");
        } else {
            panic!("Expected IsEmpty condition");
        }
    }

    #[test]
    fn test_evaluate_project_filter() {
        let issues = vec![
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], None, None),
            make_issue("OTHER", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], None, None),
        ];
        let parsed = JqlParser::parse("project = CAVE").unwrap();
        let result = JqlEvaluator::evaluate(&issues, &parsed, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].project_key, "CAVE");
    }

    #[test]
    fn test_evaluate_status_in() {
        let issues = vec![
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], None, None),
            make_issue("CAVE", IssueStatus::InProgress, Priority::P3, IssueType::Task, vec![], None, None),
            make_issue("CAVE", IssueStatus::Done, Priority::P3, IssueType::Task, vec![], None, None),
        ];
        let parsed = JqlParser::parse(r#"status IN ("in_progress", "done")"#).unwrap();
        let result = JqlEvaluator::evaluate(&issues, &parsed, None);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_evaluate_assignee_current_user() {
        let user_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();
        let issues = vec![
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], Some(user_id), None),
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], Some(other_id), None),
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], None, None),
        ];
        let parsed = JqlParser::parse("assignee = currentUser()").unwrap();
        let result = JqlEvaluator::evaluate(&issues, &parsed, Some(user_id));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_evaluate_label_filter() {
        let issues = vec![
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec!["frontend".to_string()], None, None),
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec!["backend".to_string()], None, None),
        ];
        let parsed = JqlParser::parse(r#"label = "frontend""#).unwrap();
        let result = JqlEvaluator::evaluate(&issues, &parsed, None);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_evaluate_sprint_is_empty() {
        let sprint_id = Uuid::new_v4();
        let issues = vec![
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], None, None),
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], None, Some(sprint_id)),
        ];
        let parsed = JqlParser::parse("sprint IS EMPTY").unwrap();
        let result = JqlEvaluator::evaluate(&issues, &parsed, None);
        assert_eq!(result.len(), 1);
        assert!(result[0].sprint_id.is_none());
    }

    #[test]
    fn test_evaluate_empty_query_returns_all() {
        let issues = vec![
            make_issue("CAVE", IssueStatus::ToDo, Priority::P3, IssueType::Task, vec![], None, None),
            make_issue("OTHER", IssueStatus::Done, Priority::P1, IssueType::Bug, vec![], None, None),
        ];
        let parsed = JqlParser::parse("").unwrap();
        let result = JqlEvaluator::evaluate(&issues, &parsed, None);
        assert_eq!(result.len(), 2);
    }
}
