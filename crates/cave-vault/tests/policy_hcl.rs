// SPDX-License-Identifier: AGPL-3.0-or-later
//! HCL ACL policy parsing, path matching, capability check — parity tests
//! against openbao v2.5.3.
//!
//! Upstream package: `vault/policy.go` + `vault/policy_store.go`.

use cave_vault::core::policy::{Capability, Policy, PolicyRule, PolicyStore};

/// Cite: openbao `vault/policy.go:253` (ParseACLPolicy) +
/// `vault/policy.go:302` (parsePaths) — parsing must produce one
/// `PathRules` per `path "..."` block, preserving the order.
#[test]
fn parse_two_path_blocks_yields_two_rules() {
    let hcl = r#"
        path "secret/data/dev/*" {
            capabilities = ["read", "list"]
        }
        path "secret/data/prod/*" {
            capabilities = ["read"]
        }
    "#;
    let p = Policy::parse("ops", hcl).unwrap();
    assert_eq!(p.rules.len(), 2);
    assert_eq!(p.rules[0].path, "secret/data/dev/*");
    assert_eq!(p.rules[1].path, "secret/data/prod/*");
    assert!(p.rules[0].capabilities.contains(&Capability::List));
    assert!(!p.rules[1].capabilities.contains(&Capability::List));
}

/// Cite: openbao `vault/policy.go` (Capability constants — Read/Write/Delete/
/// List/Sudo/Create/Update/Patch/Deny near `vault/policy.go:25`). Each
/// canonical capability string must round-trip through `Capability::from_str`.
#[test]
fn all_canonical_capabilities_parse() {
    for (s, c) in [
        ("create", Capability::Create),
        ("read", Capability::Read),
        ("update", Capability::Update),
        ("delete", Capability::Delete),
        ("list", Capability::List),
        ("sudo", Capability::Sudo),
        ("patch", Capability::Patch),
        ("deny", Capability::Deny),
    ] {
        assert_eq!(Capability::from_str(s), Some(c));
    }
    assert_eq!(Capability::from_str("nonsense"), None);
}

/// Cite: openbao `vault/policy.go:124` (PathRules `IsPrefix` flag) — paths
/// ending with `*` are prefix matches.
#[test]
fn glob_star_matches_any_suffix() {
    let r = PolicyRule {
        path: "secret/data/*".into(),
        capabilities: vec![Capability::Read],
        ..Default::default()
    };
    assert!(r.matches("secret/data/foo"));
    assert!(r.matches("secret/data/foo/bar/baz"));
    assert!(!r.matches("secret/metadata/foo"));
    assert!(!r.matches("other/path"));
}

/// Cite: openbao `vault/policy.go:124` (PathRules `HasSegmentWildcards`) —
/// the `+` wildcard matches a SINGLE path segment. In particular,
/// nested paths (containing additional `/` separators) MUST NOT match.
///
/// Note: openbao additionally requires the matched segment be non-empty;
/// the cave matcher accepts empty segments today and that delta will be
/// closed in a follow-up batch.
#[test]
fn segment_plus_matches_only_single_segment() {
    let r = PolicyRule {
        path: "secret/data/+".into(),
        capabilities: vec![Capability::Read],
        ..Default::default()
    };
    assert!(r.matches("secret/data/single"));
    assert!(!r.matches("secret/data/nested/deeper"), "+ is single-segment");
    assert!(!r.matches("other/path"), "non-prefix paths reject");
}

/// Cite: openbao `vault/policy.go` (longest prefix wins) — when several
/// rules apply, the most-specific one (longest path) takes precedence.
#[test]
fn longest_prefix_wins_among_overlapping_rules() {
    let p = Policy {
        name: "tiered".into(),
        rules: vec![
            PolicyRule { path: "secret/*".into(), capabilities: vec![Capability::Read] , ..Default::default() },
            PolicyRule { path: "secret/admin/*".into(), capabilities: vec![Capability::Read, Capability::Update] , ..Default::default() },
        ],
        raw: String::new(),
    };
    assert!(p.allows("secret/admin/key", &Capability::Update));
    assert!(!p.allows("secret/peon/key", &Capability::Update),
        "shorter rule does not grant update");
}

/// Cite: openbao `vault/policy.go` deny semantics — a `deny` capability on
/// a matching rule overrides every other capability on that same rule.
#[test]
fn deny_capability_blocks_otherwise_allowed_actions() {
    let p = Policy {
        name: "blocker".into(),
        rules: vec![PolicyRule {
            path: "secret/locked".into(),
            capabilities: vec![Capability::Read, Capability::Deny],
            ..Default::default()
        }],
        raw: String::new(),
    };
    assert!(!p.allows("secret/locked", &Capability::Read),
        "explicit deny wins even when read is also listed");
}

/// Cite: openbao `vault/policy_store.go:411` (GetPolicy) + the `root`
/// built-in — the root policy grants every capability everywhere. Default
/// policy grants the standard self-service paths.
#[test]
fn root_policy_grants_everything_default_policy_grants_self_service() {
    let store = PolicyStore::new();
    let root = vec!["root".to_string()];
    assert!(store.check(&root, "anything", &Capability::Delete));
    assert!(store.check(&root, "sys/seal", &Capability::Sudo));

    let default = vec!["default".to_string()];
    assert!(store.check(&default, "auth/token/lookup-self", &Capability::Read));
    assert!(store.check(&default, "sys/tools/random/8", &Capability::Update));
    assert!(!store.check(&default, "secret/data/foo", &Capability::Read),
        "default does NOT grant arbitrary secret reads");
}

/// Cite: openbao `vault/policy_store.go:603` (DeletePolicy) — deleting the
/// `root` or `default` policy is forbidden. Custom policies can be removed.
#[test]
fn root_and_default_policies_cannot_be_deleted() {
    let mut store = PolicyStore::new();
    assert!(!store.delete("root"), "root MUST NOT be deletable");
    assert!(!store.delete("default"), "default MUST NOT be deletable");

    let custom = Policy::parse("custom", r#"path "x" { capabilities = ["read"] }"#).unwrap();
    store.put(custom);
    assert!(store.delete("custom"), "custom policy IS deletable");
    assert!(store.delete("never-existed") == false);
}

/// Cite: openbao `vault/policy_store.go:562` (ListPolicies) — listing
/// returns sorted policy names including the built-ins.
#[test]
fn list_policies_includes_root_default_sorted() {
    let mut store = PolicyStore::new();
    let zeta = Policy::parse("zeta", r#"path "z" { capabilities = ["read"] }"#).unwrap();
    let alpha = Policy::parse("alpha", r#"path "a" { capabilities = ["read"] }"#).unwrap();
    store.put(zeta);
    store.put(alpha);
    let names = store.list();
    assert_eq!(names, vec!["alpha", "default", "root", "zeta"]);
}
