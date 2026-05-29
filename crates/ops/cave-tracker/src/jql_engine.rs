// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Enhanced JQL-like query engine for cave-tracker.
//!
//! Supports:
//! - `field = value` (equality)
//! - `field IN ("a", "b")` (membership)
//! - `field IS EMPTY` / `field IS NOT EMPTY`
//! - `expr AND expr` / `expr OR expr`
//! - `ORDER BY field ASC|DESC`
//!
//! This module exposes the same types as the orphan `jql.rs` but adapted
//! to the current `models::Issue` (string-based status/assignee, Priority enum).

use crate::models::{Issue, Priority};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum JqlValue {
    Str(String),
    CurrentUser,
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

pub struct JqlEngine;

impl JqlEngine {
    /// Parse a JQL query string.
    pub fn parse(query: &str) -> Result<ParsedJql, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(ParsedJql { condition: None, order_by: vec![] });
        }
        let (where_part, order_part) = split_order_by(query);
        let order_by = parse_order_by(order_part)?;
        let condition = if where_part.trim().is_empty() {
            None
        } else {
            Some(parse_condition(where_part.trim())?)
        };
        Ok(ParsedJql { condition, order_by })
    }

    /// Evaluate a parsed JQL against a slice of issues, returning matching clones.
    pub fn evaluate(issues: &[Issue], parsed: &ParsedJql) -> Vec<Issue> {
        let mut result: Vec<Issue> = issues
            .iter()
            .filter(|issue| {
                if let Some(cond) = &parsed.condition {
                    eval_condition(issue, cond)
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        for order in parsed.order_by.iter().rev() {
            let asc = order.ascending;
            let field = order.field.to_lowercase();
            result.sort_by(|a, b| {
                let ord = compare_by_field(a, b, &field);
                if asc { ord } else { ord.reverse() }
            });
        }

        result
    }
}

// ── Parser internals ──────────────────────────────────────────────────────────

fn split_order_by(query: &str) -> (&str, &str) {
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
        if part.is_empty() { continue; }
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() { continue; }
        let field = tokens[0].to_string();
        let ascending = if tokens.len() >= 2 {
            match tokens[1].to_uppercase().as_str() {
                "ASC" => true,
                "DESC" => false,
                other => return Err(format!("Unknown sort direction: {}", other)),
            }
        } else { true };
        result.push(OrderBy { field, ascending });
    }
    Ok(result)
}

fn parse_condition(s: &str) -> Result<JqlCondition, String> {
    parse_or(s)
}

fn parse_or(s: &str) -> Result<JqlCondition, String> {
    let parts = split_top_level(s, " OR ");
    if parts.len() == 1 { return parse_and(parts[0].trim()); }
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
    if parts.len() == 1 { return parse_atom(parts[0].trim()); }
    let mut iter = parts.into_iter();
    let mut left = parse_atom(iter.next().unwrap().trim())?;
    for part in iter {
        let right = parse_atom(part.trim())?;
        left = JqlCondition::And(Box::new(left), Box::new(right));
    }
    Ok(left)
}

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
        if ch == '"' || ch == '\'' { in_quote = !in_quote; }
        else if !in_quote {
            if ch == '(' { depth += 1; }
            else if ch == ')' { depth = depth.saturating_sub(1); }
            else if depth == 0
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

fn strip_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        let mut depth = 0i32;
        let mut ok = true;
        for ch in inner.chars() {
            match ch {
                '(' => depth += 1,
                ')' => { if depth == 0 { ok = false; break; } depth -= 1; }
                _ => {}
            }
        }
        if ok && depth == 0 { return inner.trim(); }
    }
    s
}

fn find_in_keyword(s: &str) -> Option<usize> {
    // Match ` IN (` only — the IN keyword must be followed by `(` (with optional whitespace)
    // to avoid matching `In Progress` or similar values.
    let upper = s.to_uppercase();
    let mut i = 0;
    while i + 4 <= upper.len() {
        if upper[i..].starts_with(" IN ") || upper[i..].starts_with(" IN\t") {
            // Verify the rest (after " IN ") starts with '(' possibly with whitespace.
            let rest = upper[i + 4..].trim_start();
            if rest.starts_with('(') {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn parse_value_list(s: &str) -> Result<Vec<JqlValue>, String> {
    let s = if s.trim().starts_with('(') && s.trim().ends_with(')') {
        let t = s.trim();
        &t[1..t.len() - 1]
    } else { s.trim() };
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = ' ';
    for ch in s.chars() {
        match ch {
            '"' | '\'' if !in_quote => { in_quote = true; quote_char = ch; }
            c if in_quote && c == quote_char => { in_quote = false; }
            ',' if !in_quote => {
                let v = current.trim().to_string();
                if !v.is_empty() { values.push(parse_single_value(&v)?); }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    let v = current.trim().to_string();
    if !v.is_empty() { values.push(parse_single_value(&v)?); }
    Ok(values)
}

fn parse_single_value(s: &str) -> Result<JqlValue, String> {
    let s = s.trim();
    if s.to_lowercase() == "currentuser()" { return Ok(JqlValue::CurrentUser); }
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return Ok(JqlValue::Str(s[1..s.len() - 1].to_string()));
    }
    Ok(JqlValue::Str(s.to_string()))
}

fn parse_atom(s: &str) -> Result<JqlCondition, String> {
    let s = strip_outer_parens(s);
    let upper = s.to_uppercase();

    // IS NOT EMPTY
    if let Some(rest) = upper.strip_suffix(" IS NOT EMPTY") {
        return Ok(JqlCondition::IsNotEmpty { field: rest.trim().to_lowercase() });
    }
    // IS EMPTY
    if let Some(rest) = upper.strip_suffix(" IS EMPTY") {
        return Ok(JqlCondition::IsEmpty { field: rest.trim().to_lowercase() });
    }
    // IN (...)
    if let Some(in_pos) = find_in_keyword(s) {
        let field = s[..in_pos].trim().to_lowercase();
        let rest = s[in_pos + 4..].trim();
        let values = parse_value_list(rest)?;
        return Ok(JqlCondition::In { field, values });
    }
    // = operator
    if let Some(eq_pos) = s.find('=') {
        let field = s[..eq_pos].trim().to_lowercase();
        let value_str = s[eq_pos + 1..].trim();
        let value = parse_single_value(value_str)?;
        return Ok(JqlCondition::Equals { field, value });
    }
    Err(format!("Cannot parse JQL atom: {}", s))
}

// ── Evaluator ──────────────────────────────────────────────────────────────────

fn eval_condition(issue: &Issue, cond: &JqlCondition) -> bool {
    match cond {
        JqlCondition::Equals { field, value } => eval_equals(issue, field, value),
        JqlCondition::In { field, values } => values.iter().any(|v| eval_equals(issue, field, v)),
        JqlCondition::IsEmpty { field } => eval_is_empty(issue, field),
        JqlCondition::IsNotEmpty { field } => !eval_is_empty(issue, field),
        JqlCondition::And(l, r) => eval_condition(issue, l) && eval_condition(issue, r),
        JqlCondition::Or(l, r) => eval_condition(issue, l) || eval_condition(issue, r),
    }
}

fn jql_value_str(v: &JqlValue) -> String {
    match v {
        JqlValue::Str(s) => s.clone(),
        JqlValue::CurrentUser => String::new(),
    }
}

fn eval_equals(issue: &Issue, field: &str, value: &JqlValue) -> bool {
    let vs = jql_value_str(value);
    match field {
        "project" | "project_key" => issue.project_key.to_uppercase() == vs.to_uppercase(),
        "status" => issue.status.to_lowercase() == vs.to_lowercase(),
        "assignee" => issue.assignee.as_deref().unwrap_or("").to_lowercase() == vs.to_lowercase(),
        "reporter" => issue.reporter.to_lowercase() == vs.to_lowercase(),
        "priority" => {
            let ps = format!("{:?}", issue.priority).to_lowercase();
            ps == vs.to_lowercase()
        }
        "type" | "issuetype" | "issue_type" => {
            let ts = issue.issue_type.to_string().to_lowercase();
            ts == vs.to_lowercase()
        }
        "label" | "labels" => issue.labels.iter().any(|l| l.to_lowercase() == vs.to_lowercase()),
        "sprint" | "sprint_id" => issue.sprint_id.map(|id| id.to_string()).as_deref().unwrap_or("") == vs,
        "epic" | "epic_id" => issue.epic_id.map(|id| id.to_string()).as_deref().unwrap_or("") == vs,
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

fn compare_by_field(a: &Issue, b: &Issue, field: &str) -> std::cmp::Ordering {
    match field {
        "priority" => {
            fn pri_ord(p: &Priority) -> u8 {
                match p {
                    Priority::Critical => 0,
                    Priority::High => 1,
                    Priority::Medium => 2,
                    Priority::Low => 3,
                    Priority::Trivial => 4,
                }
            }
            pri_ord(&a.priority).cmp(&pri_ord(&b.priority))
        }
        "created_at" => a.created_at.cmp(&b.created_at),
        "updated_at" => a.updated_at.cmp(&b.updated_at),
        "summary" => a.summary.cmp(&b.summary),
        "rank" => a.rank.cmp(&b.rank),
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
    use crate::models::{Issue, IssueType, Priority};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make(key: &str, project: &str, status: &str) -> Issue {
        Issue {
            id: Uuid::new_v4(),
            key: key.to_string(),
            project_id: Uuid::new_v4(),
            project_key: project.to_string(),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
            resolved_at: None,
        }
    }

    #[test]
    fn parse_empty_query() {
        let p = JqlEngine::parse("").unwrap();
        assert!(p.condition.is_none());
    }

    #[test]
    fn parse_project_equals() {
        let p = JqlEngine::parse("project = CAVE").unwrap();
        assert!(p.condition.is_some());
    }

    #[test]
    fn evaluate_project_filter() {
        let issues = vec![make("CAVE-1", "CAVE", "To Do"), make("OTHER-1", "OTHER", "To Do")];
        let p = JqlEngine::parse("project = CAVE").unwrap();
        let res = JqlEngine::evaluate(&issues, &p);
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn evaluate_or_condition() {
        let issues = vec![make("CAVE-1", "CAVE", "To Do"), make("CAVE-2", "CAVE", "Done"), make("CAVE-3", "CAVE", "In Progress")];
        let p = JqlEngine::parse("status = Done OR status = In Progress").unwrap();
        let res = JqlEngine::evaluate(&issues, &p);
        assert_eq!(res.len(), 2);
    }
}
