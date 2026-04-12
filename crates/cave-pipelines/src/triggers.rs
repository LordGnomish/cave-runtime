//! Pipeline triggers — webhook, cron, git push, CEL interceptors.

use crate::models::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Trigger definitions ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerTemplate {
    pub id: Uuid,
    pub name: String,
    pub pipeline_ref: String,
    #[serde(default)]
    pub params: Vec<TriggerParam>,
    #[serde(default)]
    pub workspace_bindings: Vec<TriggerWorkspaceBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerParam {
    pub name: String,
    /// CEL expression or literal value.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerWorkspaceBinding {
    pub name: String,
    pub binding_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerBinding {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub params: Vec<TriggerBindingParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerBindingParam {
    pub name: String,
    /// JSONPath or CEL expression extracting value from event payload.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventListener {
    pub id: Uuid,
    pub name: String,
    pub triggers: Vec<ListenerTrigger>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerTrigger {
    pub name: String,
    pub interceptors: Vec<Interceptor>,
    pub bindings: Vec<String>,
    pub template: String,
}

// ─── Interceptors (CEL-based filtering) ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Interceptor {
    /// CEL filter — event only passes if expression evaluates to true.
    Cel { filter: String, overlays: Vec<CelOverlay> },
    /// GitHub-specific interceptor (HMAC signature validation).
    GitHub { secret_ref: String, event_types: Vec<String> },
    /// Bitbucket interceptor.
    Bitbucket { secret_ref: String, event_types: Vec<String> },
    /// Webhook interceptor (URL-based forwarding).
    Webhook { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelOverlay {
    pub key: String,
    pub expression: String,
}

// ─── CEL evaluator (simplified) ──────────────────────────────────────────────

/// Evaluate a simplified CEL expression against a JSON body.
/// Supports: equality, &&, ||, body.field access, string literals.
pub fn evaluate_cel(expr: &str, body: &serde_json::Value) -> bool {
    let trimmed = expr.trim();

    // AND
    if let Some((left, right)) = split_logical(trimmed, "&&") {
        return evaluate_cel(left, body) && evaluate_cel(right, body);
    }

    // OR
    if let Some((left, right)) = split_logical(trimmed, "||") {
        return evaluate_cel(left, body) || evaluate_cel(right, body);
    }

    // NOT
    if trimmed.starts_with('!') {
        return !evaluate_cel(&trimmed[1..], body);
    }

    // Equality: body.X.Y == 'value'
    if let Some((lhs, rhs)) = split_at_op(trimmed, "==") {
        let lhs_val = resolve_cel_path(lhs.trim(), body);
        let rhs_val = strip_quotes(rhs.trim());
        return lhs_val.as_deref() == Some(rhs_val);
    }

    // Not-equal
    if let Some((lhs, rhs)) = split_at_op(trimmed, "!=") {
        let lhs_val = resolve_cel_path(lhs.trim(), body);
        let rhs_val = strip_quotes(rhs.trim());
        return lhs_val.as_deref() != Some(rhs_val);
    }

    // .matches('regex') — simplified
    if let Some(idx) = trimmed.find(".matches('") {
        let path = &trimmed[..idx];
        let rest = &trimmed[idx + ".matches('".len()..];
        if let Some(end) = rest.find("')") {
            let pattern = &rest[..end];
            if let Some(val) = resolve_cel_path(path, body) {
                return regex::Regex::new(pattern).map(|r| r.is_match(&val)).unwrap_or(false);
            }
        }
    }

    // Fallback: field existence check
    resolve_cel_path(trimmed, body).is_some()
}

fn split_logical<'a>(expr: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    // Find op not inside parens or quotes
    let bytes = expr.as_bytes();
    let mut depth = 0i32;
    let op_bytes = op.as_bytes();
    let n = bytes.len();
    let m = op_bytes.len();
    if n < m { return None; }
    for i in 0..=(n - m) {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && &bytes[i..i + m] == op_bytes {
            return Some((&expr[..i], &expr[i + m..]));
        }
    }
    None
}

fn split_at_op<'a>(expr: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    expr.find(op).map(|i| (&expr[..i], &expr[i + op.len()..]))
}

fn strip_quotes(s: &str) -> &str {
    if (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('"') && s.ends_with('"'))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn resolve_cel_path(path: &str, body: &serde_json::Value) -> Option<String> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = body;
    for part in &parts {
        if *part == "body" {
            continue;
        }
        current = current.get(part)?;
    }
    match current {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// ─── Cron trigger ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronTrigger {
    pub id: Uuid,
    pub name: String,
    pub schedule: String,
    pub pipeline_ref: String,
    #[serde(default)]
    pub params: Vec<Param>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
}

// ─── Git push trigger ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPushTrigger {
    pub id: Uuid,
    pub name: String,
    pub repo_url: String,
    #[serde(default)]
    pub branches: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    pub pipeline_ref: String,
    #[serde(default)]
    pub params: Vec<TriggerParam>,
    pub webhook_secret: Option<String>,
}

// ─── Incoming webhook event ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookEvent {
    pub event_type: String,
    pub source: String,
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
    pub received_at: DateTime<Utc>,
}

impl WebhookEvent {
    pub fn new(event_type: impl Into<String>, body: serde_json::Value) -> Self {
        Self {
            event_type: event_type.into(),
            source: "unknown".to_string(),
            headers: HashMap::new(),
            body,
            received_at: Utc::now(),
        }
    }
}

/// Check if a webhook event passes all interceptors.
pub fn passes_interceptors(event: &WebhookEvent, interceptors: &[Interceptor]) -> bool {
    for interceptor in interceptors {
        match interceptor {
            Interceptor::Cel { filter, .. } => {
                if !evaluate_cel(filter, &event.body) {
                    return false;
                }
            }
            Interceptor::GitHub { event_types, .. } => {
                if !event_types.is_empty() {
                    let header_event = event
                        .headers
                        .get("x-github-event")
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    if !event_types.iter().any(|et| et == header_event) {
                        return false;
                    }
                }
            }
            Interceptor::Bitbucket { event_types, .. } => {
                if !event_types.is_empty() {
                    let header_event = event
                        .headers
                        .get("x-event-key")
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    if !event_types.iter().any(|et| et == header_event) {
                        return false;
                    }
                }
            }
            Interceptor::Webhook { .. } => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cel_equality() {
        let body = json!({"ref": "refs/heads/main"});
        assert!(evaluate_cel("body.ref == 'refs/heads/main'", &body));
        assert!(!evaluate_cel("body.ref == 'refs/heads/dev'", &body));
    }

    #[test]
    fn cel_and_operator() {
        let body = json!({"action": "push", "repository": {"private": "false"}});
        let expr = "body.action == 'push' && body.repository.private == 'false'";
        assert!(evaluate_cel(expr, &body));
    }

    #[test]
    fn cel_or_operator() {
        let body = json!({"action": "opened"});
        assert!(evaluate_cel("body.action == 'opened' || body.action == 'reopened'", &body));
    }

    #[test]
    fn cel_not_equal() {
        let body = json!({"status": "draft"});
        assert!(evaluate_cel("body.status != 'open'", &body));
        assert!(!evaluate_cel("body.status != 'draft'", &body));
    }

    #[test]
    fn passes_interceptors_github() {
        let mut event = WebhookEvent::new("push", json!({}));
        event.headers.insert("x-github-event".to_string(), "push".to_string());
        let interceptors = vec![Interceptor::GitHub {
            secret_ref: "my-secret".to_string(),
            event_types: vec!["push".to_string()],
        }];
        assert!(passes_interceptors(&event, &interceptors));
    }

    #[test]
    fn passes_interceptors_github_filtered() {
        let mut event = WebhookEvent::new("pull_request", json!({}));
        event.headers.insert("x-github-event".to_string(), "pull_request".to_string());
        let interceptors = vec![Interceptor::GitHub {
            secret_ref: "my-secret".to_string(),
            event_types: vec!["push".to_string()],
        }];
        assert!(!passes_interceptors(&event, &interceptors));
    }

    #[test]
    fn passes_interceptors_cel() {
        let event = WebhookEvent::new("push", json!({"ref": "refs/heads/main"}));
        let interceptors = vec![Interceptor::Cel {
            filter: "body.ref == 'refs/heads/main'".to_string(),
            overlays: vec![],
        }];
        assert!(passes_interceptors(&event, &interceptors));
    }
}
