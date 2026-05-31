// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader role granting `get` on `pods`, bound to `alice`.
    fn reader_policy() -> Policy {
        Policy::new()
            .with_role(Role::new("reader").with_permission("get", "pods"))
            .with_binding("alice", "reader")
    }

    #[test]
    fn test_exact_allow() {
        let policy = reader_policy();
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
    }

    #[test]
    fn test_wildcard_verb() {
        let policy = Policy::new()
            .with_role(Role::new("pod-admin").with_permission("*", "pods"))
            .with_binding("alice", "pod-admin");
        // Any verb on pods is allowed...
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Allow);
        // ...but the wildcard is scoped to the resource.
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Deny);
    }

    #[test]
    fn test_wildcard_resource() {
        let policy = Policy::new()
            .with_role(Role::new("getter").with_permission("get", "*"))
            .with_binding("alice", "getter");
        // `get` on any resource is allowed...
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Allow);
        // ...but the wildcard is scoped to the verb.
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Deny);
    }

    #[test]
    fn test_double_wildcard_allows_all() {
        let policy = Policy::new()
            .with_role(Role::new("super").with_permission("*", "*"))
            .with_binding("root", "super");
        assert_eq!(policy.evaluate("root", "delete", "secrets"), Decision::Allow);
        assert_eq!(policy.evaluate("root", "anything", "everything"), Decision::Allow);
    }

    #[test]
    fn test_deny_by_default_unmatched_verb() {
        let policy = reader_policy();
        // `reader` only grants `get`; `delete` must be denied.
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Deny);
    }

    #[test]
    fn test_deny_by_default_unmatched_resource() {
        let policy = reader_policy();
        // `reader` only grants on `pods`; `secrets` must be denied.
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Deny);
    }

    #[test]
    fn test_multi_role_union() {
        // alice holds two roles; the effective grant is their union.
        let policy = Policy::new()
            .with_role(Role::new("pod-reader").with_permission("get", "pods"))
            .with_role(Role::new("secret-writer").with_permission("create", "secrets"))
            .with_binding("alice", "pod-reader")
            .with_binding("alice", "secret-writer");

        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(policy.evaluate("alice", "create", "secrets"), Decision::Allow);
        // Neither role grants this combination.
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Deny);
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Deny);
    }

    #[test]
    fn test_unknown_subject_denied() {
        let policy = reader_policy();
        // `mallory` has no binding at all.
        assert_eq!(policy.evaluate("mallory", "get", "pods"), Decision::Deny);
    }

    #[test]
    fn test_permission_matches_exact() {
        let p = Permission::new("get", "pods");
        assert!(p.matches("get", "pods"));
        assert!(!p.matches("get", "secrets"));
        assert!(!p.matches("delete", "pods"));
    }

    #[test]
    fn test_permission_matches_wildcards() {
        assert!(Permission::new("*", "pods").matches("delete", "pods"));
        assert!(Permission::new("get", "*").matches("get", "configmaps"));
        assert!(Permission::allow_all().matches("whatever", "whichever"));
        // A concrete verb does not match a different verb even with wildcard resource.
        assert!(!Permission::new("get", "*").matches("list", "pods"));
    }

    #[test]
    fn test_decision_is_allowed() {
        assert!(Decision::Allow.is_allowed());
        assert!(!Decision::Deny.is_allowed());
    }

    #[test]
    fn test_binding_to_unknown_role_is_ignored() {
        // The binding references "ghost", which is never defined as a role.
        let policy = Policy::new()
            .with_role(Role::new("reader").with_permission("get", "pods"))
            .with_binding("alice", "ghost");
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Deny);
        // The convenience boolean form agrees.
        assert!(!policy.is_allowed("alice", "get", "pods"));
    }

    #[test]
    fn test_empty_policy_denies() {
        let policy = Policy::new();
        assert_eq!(policy.evaluate("anyone", "get", "anything"), Decision::Deny);
    }

    #[test]
    fn test_builder_roundtrip_serde() {
        let policy = reader_policy();
        let json = serde_json::to_string(&policy).expect("serialize");
        let restored: Policy = serde_json::from_str(&json).expect("deserialize");
        // Behavior survives a serde round-trip.
        assert_eq!(restored.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(restored.evaluate("alice", "delete", "pods"), Decision::Deny);
        // Decision serializes lowercase.
        assert_eq!(
            serde_json::to_string(&Decision::Allow).unwrap(),
            "\"allow\""
        );
    }
}
