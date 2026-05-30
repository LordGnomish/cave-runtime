// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD coverage for the Casbin enforce decision + batch enforce.
//!
//! Upstream (Apache-2.0, line-port permitted): casbin v3.10.0
//!   enforcer.go — `Enforce` (single decision)
//!   enforcer_interface.go — `BatchEnforce` (vectorised decision)
//!
//! Implements the canonical RBAC-with-resource-roles model:
//!   `m = g(r.sub, p.sub) && keyMatch(r.obj, p.obj) && r.act == p.act`
//!   `e = some(where (p.eft == allow))`
//! Role inheritance (`g`) flows through the [`RoleManager`]; object matching
//! uses the Casbin `keyMatch` operator already ported in `matchers.rs`.
//! (Closes manifest skip: enforcer_interface.go BatchEnforce.)

use cave_permission::enforcer::Enforcer;

fn rbac_enforcer() -> Enforcer {
    let mut e = Enforcer::new();
    // p, admin, /data/*, read   — admins may read anything under /data
    e.add_policy("admin", "/data/*", "read");
    // p, alice, /reports, write — direct grant to alice
    e.add_policy("alice", "/reports", "write");
    // g, alice, admin           — alice inherits the admin role
    e.add_grouping_policy("alice", "admin");
    e
}

#[test]
fn direct_policy_allows() {
    let e = rbac_enforcer();
    // alice has a direct write grant on /reports.
    assert!(e.enforce("alice", "/reports", "write"));
}

#[test]
fn role_inheritance_allows_via_g() {
    let e = rbac_enforcer();
    // alice inherits admin, admin can read /data/* — so alice can read /data/x.
    assert!(e.enforce("alice", "/data/secret", "read"));
    // admin directly can too.
    assert!(e.enforce("admin", "/data/secret", "read"));
}

#[test]
fn mismatched_action_or_object_denies() {
    let e = rbac_enforcer();
    // Right object, wrong action.
    assert!(!e.enforce("alice", "/data/secret", "write"));
    // Object outside the keyMatch wildcard.
    assert!(!e.enforce("admin", "/other/thing", "read"));
    // Unknown subject with no policy and no roles.
    assert!(!e.enforce("mallory", "/reports", "write"));
}

#[test]
fn empty_enforcer_denies_by_default() {
    let e = Enforcer::new();
    assert!(!e.enforce("anyone", "anything", "read"));
}

#[test]
fn batch_enforce_matches_per_request_decisions() {
    let e = rbac_enforcer();
    let reqs = vec![
        ("alice", "/reports", "write"),  // allow (direct)
        ("alice", "/data/secret", "read"), // allow (via admin role)
        ("alice", "/data/secret", "write"), // deny (wrong act)
        ("mallory", "/reports", "write"), // deny (no grant)
    ];
    let decisions = e.batch_enforce(&reqs);
    assert_eq!(decisions, vec![true, true, false, false]);
}
