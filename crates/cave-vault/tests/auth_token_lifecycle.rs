// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Token lifecycle — parity tests against openbao v2.5.3.
//!
//! Upstream package: `vault/token_store.go` (and helpers in `vault/policy_store.go`).
//! Each test cites its upstream anchor inline.

use cave_vault::token::{parse_duration, CreateTokenParams, TokenStore, TokenType};

/// Cite: openbao `vault/token_store.go:1246` (TokenStore.create) — every
/// new service token gets a generated ID prefixed `hvs.` and a separate
/// 12-byte accessor. Both must be retrievable.
#[test]
fn create_service_token_returns_hvs_prefixed_id_and_accessor() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        policies: Some(vec!["default".into()]),
        ttl: Some("1h".into()),
        ..Default::default()
    };
    let token = store.create(&params, None).unwrap();
    assert!(token.id.starts_with("hvs."), "token id must use hvs. prefix");
    assert!(!token.accessor.is_empty(), "accessor must be generated");
    assert_eq!(token.token_type, TokenType::Service);
    assert_eq!(token.ttl, 3600);
}

/// Cite: openbao `vault/token_store.go:1678` (TokenStore.Lookup) — lookup
/// by ID returns the entry; an unknown id yields nothing.
#[test]
fn lookup_by_id_and_by_accessor() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        ttl: Some("30m".into()),
        ..Default::default()
    };
    let token = store.create(&params, None).unwrap();

    let by_id = store.lookup(&token.id).expect("by id");
    assert_eq!(by_id.id, token.id);

    let by_acc = store.lookup_by_accessor(&token.accessor).expect("by accessor");
    assert_eq!(by_acc.id, token.id);

    assert!(store.lookup("hvs.does-not-exist").is_none());
}

/// Cite: openbao `vault/token_store.go` Renew flow (token TTL extension via
/// the renewer goroutine; see also `vault/token_store.go:1708` lookupTainted
/// for the renew read path). Renewing must respect `max_ttl` clamping.
#[test]
fn renew_extends_expiry_but_clamps_to_max_ttl() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        ttl: Some("1h".into()),
        explicit_max_ttl: Some("2h".into()),
        renewable: Some(true),
        ..Default::default()
    };
    let token = store.create(&params, None).unwrap();

    let renewed = store.renew(&token.id, 7200).unwrap();
    // requested 2h; max_ttl is 7200 so extension == 7200
    assert!(renewed.remaining_ttl() >= 7100);
    assert!(renewed.remaining_ttl() <= 7200);

    // requesting beyond max_ttl gets clamped
    let renewed = store.renew(&token.id, 999_999).unwrap();
    assert!(renewed.remaining_ttl() <= 7200, "must clamp to max_ttl");
}

/// Cite: openbao `vault/token_store.go:1708` (lookupTainted) +
/// `vault/token_store.go` revocation path — revoking a token removes both
/// id and accessor.
#[test]
fn revoke_removes_token_and_accessor_lookup() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams { ttl: Some("1h".into()), ..Default::default() };
    let token = store.create(&params, None).unwrap();
    let id = token.id.clone();
    let acc = token.accessor.clone();

    assert!(store.revoke(&id));
    assert!(store.lookup(&id).is_none());
    assert!(store.lookup_by_accessor(&acc).is_none());
    assert!(!store.revoke(&id), "double-revoke is idempotent (returns false)");
}

/// Cite: openbao `vault/token_store.go:1135` (rootToken). Root tokens have
/// no parent, no expiry, the `root` policy, and `is_root = true`.
#[test]
fn root_token_has_no_expiry_and_root_policy() {
    let mut store = TokenStore::default();
    let root = store.create_root("hvs.root-test-001");
    assert!(root.is_root);
    assert!(root.orphan);
    assert_eq!(root.policies, vec!["root".to_string()]);
    assert!(root.expires_at.is_none());
    assert_eq!(root.remaining_ttl(), 0, "no expiry → no remaining TTL");
}

/// Cite: openbao `vault/token_store.go:1246` (TokenStore.create) — a child
/// token inherits the parent reference; revoking the parent recursively
/// revokes the child via `revoke_tree`.
#[test]
fn revoke_tree_cascades_to_children() {
    let mut store = TokenStore::default();
    let parent = store.create(&CreateTokenParams { ttl: Some("1h".into()), ..Default::default() }, None).unwrap();
    let parent_clone = parent.clone();
    let child = store.create(
        &CreateTokenParams { ttl: Some("30m".into()), ..Default::default() },
        Some(&parent_clone),
    ).unwrap();

    assert_eq!(child.parent.as_deref(), Some(parent.id.as_str()));
    assert!(!child.orphan);

    store.revoke_tree(&parent.id);
    assert!(store.lookup(&parent.id).is_none());
    assert!(store.lookup(&child.id).is_none(), "child must be revoked with parent");
}

/// Cite: openbao supports human duration strings (`1h`, `30m`, `7d`) at
/// every TTL boundary — see `vault/token_store.go` and the SDK
/// `sdk/helper/parseutil`. cave-vault's `parse_duration` mirrors the
/// supported suffixes.
#[test]
fn parse_duration_handles_h_m_s_d_and_bare_seconds() {
    assert_eq!(parse_duration("1h"), 3600);
    assert_eq!(parse_duration("90m"), 5400);
    assert_eq!(parse_duration("7d"), 604_800);
    assert_eq!(parse_duration("45s"), 45);
    assert_eq!(parse_duration("3600"), 3600, "bare integer = seconds");
    assert_eq!(parse_duration(""), 0);
    assert_eq!(parse_duration("garbage"), 0);
}
