// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD coverage for the Casbin management API — the in-memory policy
//! store the enforcer evaluates against.
//!
//! Upstream (Apache-2.0, line-port permitted): casbin v3.10.0
//!   management_api.go — AddPolicy / RemovePolicy / HasPolicy / GetPolicy /
//!                       AddGroupingPolicy / HasGroupingPolicy
//!
//! The crate had no policy store at all (only `AllowAllPermissionPolicy`), so
//! there was nothing to add/remove/query. These are pure in-memory runtime
//! operations the enforcer needs — not the portal mutation HTTP surface.
//! (Closes manifest skip: management_api.go, was portal-api-owned.)

use cave_permission::enforcer::Enforcer;

#[test]
fn add_policy_is_idempotent_and_queryable() {
    let mut e = Enforcer::new();
    // First add succeeds (true = added).
    assert!(e.add_policy("alice", "data1", "read"));
    // Duplicate add is a no-op (false = already present), upstream semantics.
    assert!(!e.add_policy("alice", "data1", "read"));
    assert!(e.has_policy("alice", "data1", "read"));
    assert!(!e.has_policy("alice", "data1", "write"));
}

#[test]
fn remove_policy_deletes_rule() {
    let mut e = Enforcer::new();
    e.add_policy("alice", "data1", "read");
    e.add_policy("bob", "data2", "write");
    // Remove existing => true.
    assert!(e.remove_policy("alice", "data1", "read"));
    assert!(!e.has_policy("alice", "data1", "read"));
    // Remove non-existent => false.
    assert!(!e.remove_policy("alice", "data1", "read"));
    // Unrelated rule survives.
    assert!(e.has_policy("bob", "data2", "write"));
}

#[test]
fn get_policy_returns_all_rules_sorted() {
    let mut e = Enforcer::new();
    e.add_policy("bob", "data2", "write");
    e.add_policy("alice", "data1", "read");
    let policies = e.get_policy();
    assert_eq!(policies.len(), 2);
    // Deterministic ordering for stable diffs / API responses.
    assert_eq!(
        policies[0],
        vec![
            "alice".to_string(),
            "data1".to_string(),
            "read".to_string()
        ]
    );
    assert_eq!(
        policies[1],
        vec!["bob".to_string(), "data2".to_string(), "write".to_string()]
    );
}

#[test]
fn grouping_policy_feeds_role_manager() {
    let mut e = Enforcer::new();
    // g, alice, admin
    assert!(e.add_grouping_policy("alice", "admin"));
    assert!(!e.add_grouping_policy("alice", "admin")); // idempotent
    assert!(e.has_grouping_policy("alice", "admin"));
    // The link is reflected in the role manager used by enforce().
    assert!(e.role_manager().has_link("alice", "admin"));
}
