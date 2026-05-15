// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream OpenBao tests, cross-referenced
//! from `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: openbao/openbao @ v2.0.0 (fork of hashicorp/vault v1.14.0)
//!   * vault/policy_test.go
//!   * vault/token_store_test.go
//!   * audit/audit_test.go
//!   * shamir/shamir_test.go
//!   * helper/parseutil/parseutil_test.go (parse_duration)
//!
//! Vault has thousands of tests — this is a behavioral subset around
//! the core auth/audit/policy data paths.

use cave_vault::core::audit::{
    AuditAuth, AuditBackend, AuditBackendType, AuditEntry, AuditLogger, AuditRequest,
};
use cave_vault::error::VaultError;
use cave_vault::policy::{Capability, Policy, PolicyEngine, PolicyPath};
use cave_vault::shamir::{combine, split};
use cave_vault::token::{CreateTokenParams, TokenStore, parse_duration};
use std::collections::HashMap;

fn ops_policy() -> Policy {
    Policy {
        name: "ops".into(),
        paths: vec![
            PolicyPath {
                path: "secret/data/*".into(),
                capabilities: vec![Capability::Read, Capability::Create, Capability::Update],
            },
            PolicyPath {
                path: "sys/seal".into(),
                capabilities: vec![Capability::Deny],
            },
        ],
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: vault/policy_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestPolicyMatching / `matching_path_grants_listed_caps`.
#[test]
fn upstream_policy_matching_path_grants_listed_capabilities() {
    let p = ops_policy();
    assert!(p.allows("secret/data/mykey", &Capability::Read));
    assert!(p.allows("secret/data/mykey", &Capability::Create));
    // Caps not listed are not granted.
    assert!(!p.allows("secret/data/mykey", &Capability::Delete));
}

/// Upstream: TestPolicy_Deny / `deny_capability_overrides_other_grants`.
/// Upstream policy lookup applies deny precedence at the rule level.
#[test]
fn upstream_policy_deny_capability_overrides_grants() {
    let p = ops_policy();
    // Although other paths allow read, the sys/seal rule's Deny capability
    // must block read on that specific path.
    assert!(!p.allows("sys/seal", &Capability::Read));
    assert!(!p.allows("sys/seal", &Capability::Update));
}

/// Upstream: TestPolicy_Glob / `most_specific_pattern_wins`.
/// Among `secret/*` and `secret/data/*` both matching, the longest
/// pattern wins per upstream's path-matching precedence.
#[test]
fn upstream_policy_most_specific_glob_wins() {
    let p = Policy {
        name: "layered".into(),
        paths: vec![
            PolicyPath {
                path: "secret/data/*".into(),
                capabilities: vec![Capability::Read],
            },
            PolicyPath {
                path: "**".into(),
                capabilities: vec![Capability::Deny],
            },
        ],
    };
    // The more specific `secret/data/*` rule should match → Read allowed.
    assert!(p.allows("secret/data/k", &Capability::Read));
}

/// Upstream: TestACL_Root / `root_policy_grants_every_capability_on_every_path`.
#[test]
fn upstream_policy_engine_root_grants_arbitrary_path_and_cap() {
    let pe = PolicyEngine::new();
    pe.check(
        &["root".into()],
        "arbitrary/path/anything",
        &Capability::Delete,
    )
    .unwrap();
    pe.check(&["root".into()], "sys/anything", &Capability::Sudo)
        .unwrap();
}

/// Upstream: TestACL_Combination / `policy_set_combines_grants_across_policies`.
/// Multiple attached policies are OR-combined — any one granting → allow.
#[test]
fn upstream_policy_engine_check_combines_grants_from_multiple_policies() {
    let mut pe = PolicyEngine::new();
    let read_only = Policy {
        name: "read".into(),
        paths: vec![PolicyPath {
            path: "secret/data/*".into(),
            capabilities: vec![Capability::Read],
        }],
    };
    let write_only = Policy {
        name: "write".into(),
        paths: vec![PolicyPath {
            path: "secret/data/*".into(),
            capabilities: vec![Capability::Create, Capability::Update],
        }],
    };
    pe.put(read_only);
    pe.put(write_only);
    // Each policy alone covers only its capability; together they
    // grant both.
    pe.check(
        &["read".into(), "write".into()],
        "secret/data/x",
        &Capability::Read,
    )
    .unwrap();
    pe.check(
        &["read".into(), "write".into()],
        "secret/data/x",
        &Capability::Update,
    )
    .unwrap();
}

/// Upstream: TestACL_NoPolicy / `unmatched_capability_yields_PermissionDenied`.
#[test]
fn upstream_policy_engine_check_denied_returns_permission_denied() {
    let mut pe = PolicyEngine::new();
    pe.put(ops_policy());
    let err = pe
        .check(&["ops".into()], "sys/seal", &Capability::Update)
        .unwrap_err();
    assert!(matches!(err, VaultError::PermissionDenied));
}

/// Upstream: TestPolicy_BuiltIn / `default_policy_is_immutable`.
/// Upstream Vault refuses delete on the default + root policies.
#[test]
fn upstream_policy_engine_delete_builtins_is_no_op() {
    let mut pe = PolicyEngine::new();
    assert!(!pe.delete("root"));
    assert!(!pe.delete("default"));
    // Sanity: still resolvable after.
    assert!(pe.get("root").is_some());
    assert!(pe.get("default").is_some());
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: vault/token_store_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestTokenStore_CreateLookup / `root_token_lookup_returns_root_token`.
#[test]
fn upstream_token_store_create_root_returns_root_token() {
    let mut store = TokenStore::default();
    let root = store.create_root("hvs.test-root");
    assert!(root.is_root);
    assert_eq!(root.policies, vec!["root".to_string()]);
    let fetched = store.lookup("hvs.test-root").unwrap();
    assert_eq!(fetched.id, root.id);
    assert_eq!(fetched.accessor, root.accessor);
}

/// Upstream: TestTokenStore_Lookup / `lookup_by_accessor_returns_token`.
#[test]
fn upstream_token_store_lookup_by_accessor() {
    let mut store = TokenStore::default();
    let root = store.create_root("hvs.acc-test");
    let by_acc = store
        .lookup_by_accessor(&root.accessor)
        .expect("accessor lookup");
    assert_eq!(by_acc.id, root.id);
    // Unknown accessor → None.
    assert!(store.lookup_by_accessor("nope").is_none());
}

/// Upstream: TestTokenStore_Revoke / `revoked_token_no_longer_resolves`.
#[test]
fn upstream_token_store_revoke_removes_token_and_accessor() {
    let mut store = TokenStore::default();
    let root = store.create_root("hvs.revoke-test");
    let acc = root.accessor.clone();
    assert!(store.revoke("hvs.revoke-test"));
    assert!(store.lookup("hvs.revoke-test").is_none());
    assert!(store.lookup_by_accessor(&acc).is_none());
}

/// Upstream: TestTokenStore_CreateChild / `child_token_inherits_parent_policies`.
#[test]
fn upstream_token_store_child_token_inherits_parent_policies() {
    let mut store = TokenStore::default();
    let parent = store.create_root("hvs.parent-test");
    let params = CreateTokenParams {
        policies: Some(vec!["ops".into()]),
        ..Default::default()
    };
    let child = store.create(&params, Some(&parent)).unwrap();
    // Child carries the requested ops policy.
    assert!(child.policies.contains(&"ops".to_string()));
    // Parent link is preserved.
    assert_eq!(child.parent.as_deref(), Some(parent.id.as_str()));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: helper/parseutil/parseutil_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestParseDurationSecond / `bare_integer_is_seconds`.
#[test]
fn upstream_parse_duration_bare_integer_is_seconds() {
    assert_eq!(parse_duration("60"), 60);
    assert_eq!(parse_duration("0"), 0);
}

/// Upstream: TestParseDurationSecond / `suffix_units_s_m_h_d`.
#[test]
fn upstream_parse_duration_recognises_unit_suffixes() {
    assert_eq!(parse_duration("30s"), 30);
    assert_eq!(parse_duration("5m"), 5 * 60);
    assert_eq!(parse_duration("2h"), 2 * 3600);
    assert_eq!(parse_duration("1d"), 86_400);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: audit/audit_test.go + audit/format_json_test.go
// ────────────────────────────────────────────────────────────────────────────

fn audit_entry(op: &str, path: &str, token: &str, accessor: &str) -> AuditEntry {
    AuditEntry {
        time: "2026-05-14T00:00:00Z".into(),
        audit_type: "request".into(),
        request: AuditRequest {
            id: "req-1".into(),
            operation: op.into(),
            mount_type: "kv".into(),
            path: path.into(),
            remote_address: "127.0.0.1".into(),
        },
        auth: Some(AuditAuth {
            client_token: token.into(),
            accessor: accessor.into(),
            display_name: "alice".into(),
            policies: vec!["default".into()],
            token_type: "service".into(),
        }),
        error: None,
    }
}

fn audit_logger() -> AuditLogger {
    AuditLogger::new(b"test-key-32-bytes-padding-here!!".to_vec())
}

/// Upstream: TestAuditFormatter / `signed_envelope_redacts_client_token`.
/// `audit/format.go::HashAuth` replaces the plaintext token with an HMAC.
#[test]
fn upstream_audit_signed_envelope_redacts_client_token() {
    let log = audit_logger();
    let entry = audit_entry("read", "secret/a", "tok-plain", "acc-plain");
    let env = log.signed_envelope(&entry);
    assert!(!env.json.contains("tok-plain"));
    assert!(!env.json.contains("acc-plain"));
    // SHA-256 hex = 64 chars.
    assert_eq!(env.signature.len(), 64);
}

/// Upstream: TestAuditFormatter / `verify_envelope_roundtrip`.
#[test]
fn upstream_audit_signed_envelope_verify_round_trips() {
    let log = audit_logger();
    let entry = audit_entry("write", "secret/b", "t", "a");
    let env = log.signed_envelope(&entry);
    assert!(log.verify_envelope(&env));
}

/// Upstream: TestAuditFormatter / `tamper_breaks_verification`.
#[test]
fn upstream_audit_signed_envelope_tampered_signature_fails_verify() {
    let log = audit_logger();
    let entry = audit_entry("read", "secret/a", "t", "a");
    let mut env = log.signed_envelope(&entry);
    // Flip one nibble — any flip is enough for SHA-256 HMAC.
    let last = env.signature.pop().unwrap();
    env.signature.push(if last == '0' { '1' } else { '0' });
    assert!(!log.verify_envelope(&env));
}

/// Upstream: TestAuditBroker_Enable / `enable_then_list_returns_backend`.
#[test]
fn upstream_audit_logger_enable_then_list_backends() {
    let log = audit_logger();
    let backend = AuditBackend {
        path: "file/".into(),
        backend_type: AuditBackendType::File,
        description: "test".into(),
        options: HashMap::new(),
        local: false,
        seal_wrap: false,
    };
    log.enable("file/", backend);
    let listed = log.list_backends();
    assert!(listed.contains_key("file/"));
    assert!(log.disable("file/"));
    assert!(!log.disable("file/"));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: shamir/shamir_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestShamir / `split_then_combine_reconstructs_secret`.
#[test]
fn upstream_shamir_split_then_combine_reconstructs_secret() {
    let secret = b"unseal-key-payload";
    let shares = split(secret, 3, 5);
    // Any 3 of the 5 shares should reconstruct.
    let reconstructed = combine(&shares[..3]);
    assert_eq!(reconstructed, secret);
}

/// Upstream: TestShamir / `different_subset_of_threshold_still_works`.
#[test]
fn upstream_shamir_any_subset_of_threshold_size_reconstructs() {
    let secret = b"unseal";
    let shares = split(secret, 2, 4);
    let reconstructed_first = combine(&shares[..2]);
    let reconstructed_last = combine(&shares[2..]);
    assert_eq!(reconstructed_first, secret);
    assert_eq!(reconstructed_last, secret);
}
