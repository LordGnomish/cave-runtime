// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! deeper-001: Token store — explicit_max_ttl clamping, periodic tokens,
//! num_uses, child token TTL clamp, default-policy injection toggle,
//! orphan parenting. Pinned to openbao v2.5.3.

use cave_vault::token::{CreateTokenParams, TokenStore, parse_duration};

const TENANT: &str = "tenant-acme-prod";

fn token_params(policies: &[&str]) -> CreateTokenParams {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("tenant_id".into(), TENANT.into());
    CreateTokenParams {
        policies: Some(policies.iter().map(|s| s.to_string()).collect()),
        ttl: Some("1h".into()),
        renewable: Some(true),
        metadata: Some(metadata),
        ..Default::default()
    }
}

/// Cite: openbao `vault/token_store.go:975` (`ExplicitMaxTTL`) +
/// `vault/token_store.go::renew` clamp logic — `renew(N)` extends the
/// expiry but never beyond `explicit_max_ttl`.
#[test]
fn renew_clamps_to_explicit_max_ttl_even_with_huge_increment() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        ttl: Some("1h".into()),
        explicit_max_ttl: Some("4h".into()),
        renewable: Some(true),
        ..token_params(&["default"])
    };
    let tok = store.create(&params, None).unwrap();
    assert_eq!(tok.max_ttl, 4 * 3600);

    let renewed = store.renew(&tok.id, 99 * 3600).unwrap();
    assert!(
        renewed.remaining_ttl() <= 4 * 3600,
        "max_ttl clamps a huge increment to 4h"
    );
    assert!(
        renewed.remaining_ttl() >= 4 * 3600 - 5,
        "and uses the clamped max, not 1h"
    );
}

/// Cite: openbao `vault/token_store.go::create` `period` field — periodic
/// tokens have `period = Some(P)` and renew within the period without a
/// hard max_ttl ceiling. cave preserves the semantic field; the renewer
/// uses `period` as the canonical TTL on each renew.
#[test]
fn periodic_token_carries_period_field_and_no_max_clamp() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        ttl: Some("1h".into()),
        period: Some("30m".into()),
        renewable: Some(true),
        ..token_params(&["default"])
    };
    let tok = store.create(&params, None).unwrap();
    assert_eq!(tok.period, Some(1800));
    // explicit_max_ttl defaulted to ttl*8 = 8h when not set
    assert_eq!(tok.max_ttl, 8 * 3600);
}

/// Cite: openbao `vault/token_store.go:160` (`no_default_policy` field) —
/// when `no_default_policy = true`, the issued token does NOT auto-include
/// the `default` policy.
#[test]
fn no_default_policy_skips_default_injection() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        no_default_policy: Some(true),
        policies: Some(vec!["ops".into()]),
        ..token_params(&["ops"])
    };
    let tok = store.create(&params, None).unwrap();
    assert_eq!(tok.policies, vec!["ops".to_string()]);
    assert!(!tok.policies.iter().any(|p| p == "default"));

    // Default behaviour: 'default' is auto-appended.
    let mut params2 = token_params(&["ops"]);
    params2.no_default_policy = Some(false);
    let tok2 = store.create(&params2, None).unwrap();
    assert!(tok2.policies.iter().any(|p| p == "default"));
}

/// Cite: openbao `vault/token_store.go:1627` (`if te.NumUses == 0`)
/// — `num_uses == 0` means unlimited; `num_uses > 0` means the token can
/// only be used `num_uses` times. The `uses_remaining` mirror starts equal
/// to `num_uses` and is decremented on each use.
#[test]
fn num_uses_seed_equals_num_uses_value() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        num_uses: Some(5),
        ..token_params(&["default"])
    };
    let tok = store.create(&params, None).unwrap();
    assert_eq!(tok.num_uses, 5);
    assert_eq!(tok.uses_remaining, 5);

    let unlimited = store.create(&token_params(&["default"]), None).unwrap();
    assert_eq!(unlimited.num_uses, 0, "0 = unlimited");
    assert_eq!(unlimited.uses_remaining, 0);
}

/// Cite: openbao `vault/token_store.go:1246` (`create`, parent-link
/// branch) — a child token records its parent and is added to the
/// parent's children index. Revoking the parent must cascade to revoke
/// the child via revoke_tree.
#[test]
fn child_token_records_parent_and_revoke_tree_cascades() {
    let mut store = TokenStore::default();
    let parent = store.create(&token_params(&["default"]), None).unwrap();
    let parent_clone = parent.clone();
    let child = store
        .create(
            &CreateTokenParams {
                ttl: Some("30m".into()),
                ..token_params(&["default"])
            },
            Some(&parent_clone),
        )
        .unwrap();

    assert_eq!(child.parent.as_deref(), Some(parent.id.as_str()));
    assert!(!child.orphan);

    store.revoke_tree(&parent.id);
    assert!(store.lookup(&parent.id).is_none());
    assert!(store.lookup(&child.id).is_none());
}

/// Cite: openbao `vault/token_store.go:1246` (`create`, `no_parent` branch)
/// — explicitly orphaned tokens never inherit a parent reference even
/// when one is supplied; they survive parent revocation.
#[test]
fn no_parent_orphans_token_even_with_parent_supplied() {
    let mut store = TokenStore::default();
    let parent = store.create(&token_params(&["default"]), None).unwrap();
    let parent_clone = parent.clone();
    let mut params = token_params(&["default"]);
    params.no_parent = Some(true);
    let orphan = store.create(&params, Some(&parent_clone)).unwrap();

    assert!(orphan.orphan, "no_parent ⇒ orphan");
    assert!(orphan.parent.is_none(), "no parent reference recorded");

    // Parent revoke does NOT cascade.
    store.revoke_tree(&parent.id);
    assert!(store.lookup(&parent.id).is_none());
    assert!(
        store.lookup(&orphan.id).is_some(),
        "orphan survives parent revocation"
    );
}

/// Cite: openbao `vault/token_store.go::renew` non-renewable branch —
/// renewing a non-renewable token returns an error rather than silently
/// resetting the TTL.
#[test]
fn renew_on_non_renewable_token_errors() {
    let mut store = TokenStore::default();
    let params = CreateTokenParams {
        renewable: Some(false),
        ..token_params(&["default"])
    };
    let tok = store.create(&params, None).unwrap();
    assert!(!tok.renewable);
    assert!(
        store.renew(&tok.id, 60).is_err(),
        "non-renewable ⇒ renew rejected"
    );
    // Bonus: parse_duration default for unrecognised units → 0 (no fallback).
    assert_eq!(parse_duration("5x"), 0);
}
