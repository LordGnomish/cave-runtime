// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Real behavior tests for cave-vault policy engine (Mode B-prime spike).
//! Exercises Vault-style path-glob ACL + Deny-overrides-cap invariant.
//! Generated 2026-05-04 via local Ollama (qwen3.6:35b-a3b-coding-mxfp8).
#![allow(unused_imports, unused_variables, unused_mut, dead_code)]

#[cfg(test)]
mod tests {
    use cave_vault::policy::{Capability, Policy, PolicyEngine, PolicyPath};

    #[test]
    fn root_policy_grants_all_caps_on_any_path() {
        let policy = cave_vault::policy::root_policy();
        
        // Root policy has "**" path with all caps including Sudo.
        assert!(policy.allows("anything/at/all", &Capability::Create));
        assert!(policy.allows("auth/token/create", &Capability::Sudo));
        assert!(policy.allows("sys/mounts", &Capability::List));
    }

    #[test]
    fn default_policy_allows_lookup_self_read_only() {
        let policy = cave_vault::policy::default_policy();
        
        // Default policy allows Read on auth/token/lookup-self
        assert!(policy.allows("auth/token/lookup-self", &Capability::Read));
        
        // Default policy does NOT allow Update on auth/token/lookup-self
        assert!(!policy.allows("auth/token/lookup-self", &Capability::Update));
    }

    #[test]
    fn policy_deny_overrides_cap() {
        let policy = Policy {
            name: "test".to_string(),
            paths: vec![
                PolicyPath {
                    path: "secret/sensitive".to_string(),
                    capabilities: vec![Capability::Read, Capability::Deny],
                }
            ],
        };

        // Even though Read is listed, Deny overrides it.
        assert!(!policy.allows("secret/sensitive", &Capability::Read));
        
        // Deny itself should return true for allows? 
        // The spec says: "allows() returns true iff cap is in caps and Deny is NOT in caps"
        // So allows("secret/sensitive", &Capability::Deny) should be false because Deny IS in caps.
        assert!(!policy.allows("secret/sensitive", &Capability::Deny));
    }

    #[test]
    fn policy_glob_specificity_picks_longest() {
        let policy = Policy {
            name: "specificity-test".to_string(),
            paths: vec![
                PolicyPath {
                    path: "secret/**".to_string(),
                    capabilities: vec![Capability::Read],
                },
                PolicyPath {
                    path: "secret/data/special".to_string(),
                    capabilities: vec![Capability::Sudo],
                },
            ],
        };

        // "secret/data/special" matches both "secret/**" and "secret/data/special".
        // The more specific one ("secret/data/special") should win.
        let caps = policy.capabilities_for("secret/data/special");
        assert!(caps.contains(&&Capability::Sudo));
        assert!(!caps.contains(&&Capability::Read)); // Should not have Read from the glob

        // "secret/data/general" only matches "secret/**".
        let caps_general = policy.capabilities_for("secret/data/general");
        assert!(caps_general.contains(&&Capability::Read));
        assert!(!caps_general.contains(&&Capability::Sudo));
    }

    #[test]
    fn engine_protects_root_and_default_from_delete() {
        let mut engine = PolicyEngine::new();

        // Root and default are immutable built-ins.
        assert!(!engine.delete("root"));
        assert!(!engine.delete("default"));

        // Add a custom policy.
        let custom_policy = Policy {
            name: "custom".to_string(),
            paths: vec![
                PolicyPath {
                    path: "custom/path".to_string(),
                    capabilities: vec![Capability::Read],
                }
            ],
        };
        engine.put(custom_policy);

        // Verify it exists.
        assert!(engine.get("custom").is_some());

        // Delete the custom policy.
        assert!(engine.delete("custom"));

        // Verify it's gone.
        assert!(engine.get("custom").is_none());

        // List should still contain root and default, sorted alphabetically.
        let list = engine.list();
        assert_eq!(list, vec!["default".to_string(), "root".to_string()]);
    }
}
