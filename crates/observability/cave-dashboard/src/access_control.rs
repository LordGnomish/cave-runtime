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
