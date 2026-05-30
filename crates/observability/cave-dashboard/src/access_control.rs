// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Grafana access-control RBAC evaluator — line-port of
//! grafana/grafana `pkg/services/accesscontrol` (evaluator.go + scope.go +
//! checker.go + ValidateScope).
//!
//! This is Grafana's own action/scope permission engine: permissions are a
//! map of `action -> [scopes]`, and an [`Evaluator`] (permission / all / any)
//! decides whether a user holding those permissions is authorised. Scope
//! matching supports the `<resource>:<attribute>:<id>` shape plus trailing-`*`
//! wildcards. This is the Grafana-shaped RBAC surface the auth module only
//! covered shallowly — distinct from cave-auth's identity-provider RBAC.

use std::collections::HashMap;

const MAX_PREFIX_PARTS: usize = 2;

// ── scope.go ────────────────────────────────────────────────────────────────

/// `SplitScope` — returns (kind, attribute, identifier).
pub fn split_scope(scope: &str) -> (String, String, String) {
    if scope.is_empty() {
        return (String::new(), String::new(), String::new());
    }
    let fragments: Vec<&str> = scope.split(':').collect();
    match fragments.len() {
        1 => (
            fragments[0].to_string(),
            fragments[0].to_string(),
            fragments[0].to_string(),
        ),
        2 => (
            fragments[0].to_string(),
            fragments[1].to_string(),
            fragments[1].to_string(),
        ),
        _ => (
            fragments[0].to_string(),
            fragments[1].to_string(),
            fragments[2..].join(":"),
        ),
    }
}

/// `Scope` — build a scope from parts joined by ':'.
pub fn scope(parts: &[&str]) -> String {
    parts.join(":")
}

/// `GetResourceScope` — "<resource>:id:<id>".
pub fn get_resource_scope(resource: &str, resource_id: &str) -> String {
    scope(&[resource, "id", resource_id])
}

/// `GetResourceScopeUID` — "<resource>:uid:<id>".
pub fn get_resource_scope_uid(resource: &str, resource_id: &str) -> String {
    scope(&[resource, "uid", resource_id])
}

/// `GetResourceScopeName` — "<resource>:name:<id>".
pub fn get_resource_scope_name(resource: &str, resource_id: &str) -> String {
    scope(&[resource, "name", resource_id])
}

/// `GetResourceAllScope` — "<resource>:*".
pub fn get_resource_all_scope(resource: &str) -> String {
    scope(&[resource, "*"])
}

/// `GetResourceAllIDScope` — "<resource>:id:*".
pub fn get_resource_all_id_scope(resource: &str) -> String {
    scope(&[resource, "id", "*"])
}

/// `ScopePrefix` — the "<resource>:<attribute>:" prefix (≤ maxPrefixParts).
pub fn scope_prefix(scope: &str) -> String {
    let mut parts: Vec<String> = scope.split(':').map(str::to_string).collect();
    if parts.len() > MAX_PREFIX_PARTS {
        parts.truncate(MAX_PREFIX_PARTS);
        parts.push(String::new());
    }
    parts.join(":")
}

/// `WildcardsFromPrefixes` — generates valid wildcards from prefixes,
/// e.g. "datasource:uid:" → ["*", "datasource:*", "datasource:uid:*"].
pub fn wildcards_from_prefixes(prefixes: &[&str]) -> Vec<String> {
    let mut wildcards = vec!["*".to_string()];
    for prefix in prefixes {
        let mut acc = String::new();
        for p in prefix.split(':') {
            if p.is_empty() {
                continue;
            }
            acc.push_str(p);
            acc.push(':');
            wildcards.push(format!("{acc}*"));
        }
    }
    wildcards
}

// ── ValidateScope (accesscontrol.go) ────────────────────────────────────────

/// `ValidateScope` — scopes may only contain `*`/`?` in the last position,
/// and a trailing `*` must follow ':' or '/'.
pub fn validate_scope(scope: &str) -> bool {
    if scope.is_empty() {
        return false;
    }
    let bytes = scope.as_bytes();
    let last = bytes[bytes.len() - 1];
    let prefix = &scope[..scope.len() - 1];
    if !prefix.is_empty() && last == b'*' {
        let last_char = prefix.as_bytes()[prefix.len() - 1];
        if last_char != b':' && last_char != b'/' {
            return false;
        }
    }
    !prefix.contains(['*', '?'])
}

// ── match() (evaluator.go) ──────────────────────────────────────────────────

/// `match(scope, target)` — exact or trailing-`*` prefix match, gated on
/// [`validate_scope`].
pub fn scope_match(scope: &str, target: &str) -> bool {
    if scope.is_empty() {
        return false;
    }
    if !validate_scope(scope) {
        return false;
    }
    let prefix = &scope[..scope.len() - 1];
    let last = scope.as_bytes()[scope.len() - 1];
    if last == b'*' && target.starts_with(prefix) {
        return true;
    }
    scope == target
}

// ── Evaluator (evaluator.go) ────────────────────────────────────────────────

/// A permission evaluator. Mirrors `permissionEvaluator` / `allEvaluator` /
/// `anyEvaluator`.
#[derive(Debug, Clone, PartialEq)]
pub enum Evaluator {
    Permission { action: String, scopes: Vec<String> },
    All(Vec<Evaluator>),
    Any(Vec<Evaluator>),
}

/// `EvalPermission` — requires at least one of `scopes` to match the user's
/// scopes for `action` (or just the action if `scopes` is empty).
pub fn eval_permission(action: &str, scopes: &[&str]) -> Evaluator {
    Evaluator::Permission {
        action: action.to_string(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
    }
}

/// `EvalAll` — every sub-evaluator must pass.
pub fn eval_all(all_of: Vec<Evaluator>) -> Evaluator {
    Evaluator::All(all_of)
}

/// `EvalAny` — at least one sub-evaluator must pass.
pub fn eval_any(any_of: Vec<Evaluator>) -> Evaluator {
    Evaluator::Any(any_of)
}

impl Evaluator {
    /// `Evaluate` — decide against a map of `action -> [scopes]`.
    pub fn evaluate(&self, permissions: &HashMap<String, Vec<String>>) -> bool {
        match self {
            Evaluator::Permission { action, scopes } => {
                let Some(user_scopes) = permissions.get(action) else {
                    return false;
                };
                if scopes.is_empty() {
                    return true;
                }
                for target in scopes {
                    for scope in user_scopes {
                        if scope_match(scope, target) {
                            return true;
                        }
                    }
                }
                false
            }
            Evaluator::All(all_of) => all_of.iter().all(|e| e.evaluate(permissions)),
            Evaluator::Any(any_of) => any_of.iter().any(|e| e.evaluate(permissions)),
        }
    }
}

impl std::fmt::Display for Evaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Evaluator::Permission { action, .. } => write!(f, "{action}"),
            Evaluator::All(all_of) => {
                let parts: Vec<String> = all_of.iter().map(|e| e.to_string()).collect();
                write!(f, "all of {}", parts.join(", "))
            }
            Evaluator::Any(any_of) => {
                let parts: Vec<String> = any_of.iter().map(|e| e.to_string()).collect();
                write!(f, "any of {}", parts.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ── scope.go ────────────────────────────────────────────────────────────

    #[test]
    fn test_split_scope() {
        assert_eq!(split_scope("*"), ("*".into(), "*".into(), "*".into()));
        assert_eq!(
            split_scope("dashboards:*"),
            ("dashboards".into(), "*".into(), "*".into())
        );
        assert_eq!(
            split_scope("dashboards:uid:my_dash"),
            ("dashboards".into(), "uid".into(), "my_dash".into())
        );
        assert_eq!(
            split_scope("a:b:c:d"),
            ("a".into(), "b".into(), "c:d".into())
        );
        assert_eq!(split_scope(""), ("".into(), "".into(), "".into()));
    }

    #[test]
    fn test_scope_builders() {
        assert_eq!(get_resource_scope("dashboards", "1"), "dashboards:id:1");
        assert_eq!(get_resource_scope_uid("dashboards", "x"), "dashboards:uid:x");
        assert_eq!(get_resource_all_scope("dashboards"), "dashboards:*");
        assert_eq!(get_resource_all_id_scope("dashboards"), "dashboards:id:*");
    }

    #[test]
    fn test_scope_prefix() {
        assert_eq!(scope_prefix("datasources:name:test"), "datasources:name:");
        assert_eq!(scope_prefix("dashboards:uid:1"), "dashboards:uid:");
        assert_eq!(scope_prefix("dashboards:*"), "dashboards:*");
    }

    #[test]
    fn test_wildcards_from_prefixes() {
        assert_eq!(
            wildcards_from_prefixes(&["datasource:uid:"]),
            vec!["*", "datasource:*", "datasource:uid:*"]
        );
    }

    // ── ValidateScope (accesscontrol.go) ────────────────────────────────────

    #[test]
    fn test_validate_scope() {
        assert!(validate_scope("dashboards:uid:*"));
        assert!(validate_scope("dashboards:*"));
        assert!(validate_scope("*"));
        assert!(validate_scope("dashboards:uid:1"));
        assert!(validate_scope("reports:/path/*")); // trailing slash before *
        // '*' not in last position is invalid
        assert!(!validate_scope("dash*board"));
        // trailing '*' must follow ':' or '/'
        assert!(!validate_scope("dashboards:uid:foo*"));
    }

    // ── match() (evaluator.go) ──────────────────────────────────────────────

    #[test]
    fn test_scope_match() {
        assert!(scope_match("dashboards:*", "dashboards:uid:1"));
        assert!(scope_match("dashboards:uid:1", "dashboards:uid:1"));
        assert!(!scope_match("dashboards:uid:1", "dashboards:uid:2"));
        // "*" matches anything
        assert!(scope_match("*", "anything:goes:here"));
        // empty scope never matches
        assert!(!scope_match("", "dashboards:uid:1"));
        // invalid scope never matches
        assert!(!scope_match("dash*board", "dashboards:uid:1"));
    }

    // ── permissionEvaluator.Evaluate ────────────────────────────────────────

    fn perms(action: &str, scopes: &[&str]) -> HashMap<String, Vec<String>> {
        let mut m = HashMap::new();
        m.insert(action.to_string(), scopes.iter().map(|s| s.to_string()).collect());
        m
    }

    #[test]
    fn test_eval_permission_exact_scope() {
        let p = perms("dashboards:read", &["dashboards:uid:1"]);
        assert!(eval_permission("dashboards:read", &["dashboards:uid:1"]).evaluate(&p));
        assert!(!eval_permission("dashboards:read", &["dashboards:uid:2"]).evaluate(&p));
    }

    #[test]
    fn test_eval_permission_wildcard_scope() {
        let p = perms("dashboards:read", &["dashboards:*"]);
        assert!(eval_permission("dashboards:read", &["dashboards:uid:1"]).evaluate(&p));
    }

    #[test]
    fn test_eval_permission_missing_action_is_false() {
        let p = perms("dashboards:read", &["dashboards:*"]);
        assert!(!eval_permission("dashboards:write", &["dashboards:uid:1"]).evaluate(&p));
    }

    #[test]
    fn test_eval_permission_no_scopes_required() {
        let p = perms("dashboards:read", &["whatever"]);
        // action present, no required scopes → allowed
        assert!(eval_permission("dashboards:read", &[]).evaluate(&p));
    }

    // ── EvalAll / EvalAny ───────────────────────────────────────────────────

    #[test]
    fn test_eval_all() {
        let mut p = perms("dashboards:read", &["dashboards:*"]);
        p.insert("folders:read".into(), vec!["folders:*".into()]);
        let e = eval_all(vec![
            eval_permission("dashboards:read", &["dashboards:uid:1"]),
            eval_permission("folders:read", &["folders:uid:f1"]),
        ]);
        assert!(e.evaluate(&p));
        // remove one → fails
        let e2 = eval_all(vec![
            eval_permission("dashboards:read", &["dashboards:uid:1"]),
            eval_permission("alerts:read", &["alerts:uid:a1"]),
        ]);
        assert!(!e2.evaluate(&p));
    }

    #[test]
    fn test_eval_any() {
        let p = perms("dashboards:read", &["dashboards:*"]);
        let e = eval_any(vec![
            eval_permission("alerts:read", &["alerts:uid:a1"]),
            eval_permission("dashboards:read", &["dashboards:uid:1"]),
        ]);
        assert!(e.evaluate(&p));
        let e2 = eval_any(vec![
            eval_permission("alerts:read", &["alerts:uid:a1"]),
            eval_permission("folders:read", &["folders:uid:f1"]),
        ]);
        assert!(!e2.evaluate(&p));
    }

    #[test]
    fn test_evaluator_string() {
        assert_eq!(eval_permission("dashboards:read", &["x"]).to_string(), "dashboards:read");
        let s = eval_all(vec![
            eval_permission("a", &[]),
            eval_permission("b", &[]),
        ])
        .to_string();
        assert!(s.contains("all of"));
    }
}
