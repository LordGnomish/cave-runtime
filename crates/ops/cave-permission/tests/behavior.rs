// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD portable-coverage fills for the Casbin core embedded in
//! `cave-permission`.
//!
//! Upstream (Apache-2.0, line-port permitted): casbin v3.10.0
//!   - util/builtin_operators.go — `KeyMatch` / `RegexMatch`
//!   - rbac/default-role-manager/role_manager.go — `RoleManagerImpl.GetImplicitRoles`
//!   - enforcer.go — `Enforce` (object `keyMatch` leg, end-to-end)
//!
//! These target public cave fns that are already implemented but whose
//! casbin-portable branch was previously unexercised by any behavioral test:
//!   * `matchers::key_match` — the short-key `else` branch (key1.len() <= `*` idx).
//!   * `matchers::regex_match` — malformed-pattern hardening (cave returns
//!     `false` where upstream panics).
//!   * `rbac::RoleManager::get_implicit_roles` — `max_hierarchy_level` truncation.
//!   * `enforcer::Enforcer::enforce` — object wildcard (`keyMatch`) through the
//!     real authorizer, mirroring casbin `TestKeyMatchModelInMemory`.

use cave_permission::enforcer::Enforcer;
use cave_permission::matchers::{key_match, regex_match};
use cave_permission::rbac::RoleManager;

// ─── matchers::key_match — short-key `else` branch (src/matchers.rs:31-33) ───
//
// When `key2` contains `*` at index `i` and `key1.len() <= i`, the impl takes
// the else branch `key1 == &key2[..i]` (NOT the byte-prefix compare). These
// cases exercise exactly that leg, mirroring casbin `TestKeyMatch`.

#[test]
fn key_match_short_key_equals_wildcard_prefix() {
    // `*` is at byte index 5 in "/foo/*". key1.len() == 4 (<= 5) => else branch:
    // "/foo" == "/foo/*"[..5] would be "/foo" == "/foo/" => false.
    assert!(!key_match("/foo", "/foo/*"));
    // But the wildcard position itself: "*" at index 4 in "/foo*"; key1.len()==4
    // is NOT > 4, so else branch: "/foo" == "/foo*"[..4] => "/foo" == "/foo" => true.
    assert!(key_match("/foo", "/foo*"));
}

#[test]
fn key_match_key_shorter_than_wildcard_position_denies() {
    // key1 "/fo" (len 3) is shorter than the `*` at index 5 in "/foo/*".
    // 3 > 5 is false => else: "/fo" == "/foo/*"[..5] => "/fo" == "/foo/" => false.
    assert!(!key_match("/fo", "/foo/*"));
    // A prefix that does line up to the wildcard index matches:
    // "/foo/" len 5, `*` at 5 in "/foo/*"; 5 > 5 false => "/foo/" == "/foo/" => true.
    assert!(key_match("/foo/", "/foo/*"));
}

#[test]
fn key_match_empty_key_edges() {
    // Empty key against a bare "*" (idx 0): 0 > 0 false => "" == ""[..0] => "" == "" => true.
    assert!(key_match("", "*"));
    // Empty key against "/foo/*" (`*` at idx 5): 0 > 5 false => "" == "/foo/" => false.
    assert!(!key_match("", "/foo/*"));
    // Empty key against a pattern with no wildcard => plain equality.
    assert!(key_match("", ""));
    assert!(!key_match("", "/x"));
}

// ─── matchers::regex_match — malformed-pattern hardening (src/matchers.rs:62-67)
//
// Upstream panics on an invalid regex; cave returns `false`. `enforce` relies on
// this so a bad policy line can never crash the authorizer.

#[test]
fn regex_match_malformed_pattern_returns_false_without_panic() {
    // "[" is an unterminated character class — Regex::new errors => Err arm => false.
    assert!(!regex_match("x", "["));
    // "(" is an unterminated group — also Err => false.
    assert!(!regex_match("anything", "("));
    // A well-formed pattern still matches normally (control case).
    assert!(regex_match("topic/1", "topic/[0-9]+"));
}

// ─── rbac::RoleManager::get_implicit_roles — max_hierarchy_level cap ──────────
//
// The closure walk runs `while level < max_hierarchy_level`, advancing one BFS
// frontier per level. With a cap of 2 over the chain a->b->c->d, only b (level 0)
// and c (level 1) are collected; d (which is reached on level 2) is truncated.
// Mirrors casbin `TestMaxHierarchyLevel` for the implicit-roles path.

#[test]
fn get_implicit_roles_truncates_at_hierarchy_cap() {
    let mut rm = RoleManager::new(2);
    rm.add_link("a", "b");
    rm.add_link("b", "c");
    rm.add_link("c", "d"); // beyond the level-2 cap
    let mut implicit = rm.get_implicit_roles("a");
    implicit.sort();
    assert_eq!(implicit, vec!["b".to_string(), "c".to_string()]);
}

#[test]
fn get_implicit_roles_full_chain_under_generous_cap() {
    // Same chain, but cap 10 collects the entire transitive closure.
    let mut rm = RoleManager::new(10);
    rm.add_link("a", "b");
    rm.add_link("b", "c");
    rm.add_link("c", "d");
    let mut implicit = rm.get_implicit_roles("a");
    implicit.sort();
    assert_eq!(
        implicit,
        vec!["b".to_string(), "c".to_string(), "d".to_string()]
    );
}

#[test]
fn get_implicit_roles_cap_one_returns_only_direct_roles() {
    // max_hierarchy_level = 1 => only the first BFS frontier (direct roles).
    let mut rm = RoleManager::new(1);
    rm.add_link("alice", "admin");
    rm.add_link("admin", "root");
    let implicit = rm.get_implicit_roles("alice");
    assert_eq!(implicit, vec!["admin".to_string()]);
}

// ─── enforcer::Enforcer::enforce — object keyMatch wildcard, end-to-end ──────
//
// The enforce matcher is `g(r.sub, p.sub) && keyMatch(r.obj, p.obj) && r.act == p.act`.
// A policy with a `*` object exercises the `key_match(obj, p_obj)` leg through
// the real authorizer (previously only tested in isolation). Mirrors casbin
// `TestKeyMatchModelInMemory`.

#[test]
fn enforce_object_wildcard_matches_under_prefix_only() {
    let mut e = Enforcer::new();
    assert!(e.add_policy("alice", "/data/*", "read"));
    // Inside the wildcard prefix => allowed.
    assert!(e.enforce("alice", "/data/x", "read"));
    assert!(e.enforce("alice", "/data/deep/nested", "read"));
    // Outside the wildcard prefix => denied.
    assert!(!e.enforce("alice", "/other/x", "read"));
    // Right object, wrong action => denied (act == p_act leg).
    assert!(!e.enforce("alice", "/data/x", "write"));
    // Right object/action, wrong subject (no g link, no policy) => denied.
    assert!(!e.enforce("bob", "/data/x", "read"));
}
