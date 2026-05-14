// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! deeper-001: Identity store — entity + alias + group + group_aliases +
//! external-group member matching by mount_accessor + effective policy
//! union. Pinned to openbao v2.5.3.

use cave_vault::engines::identity::{GroupType, IdentityStore};

const TENANT: &str = "tenant-acme-prod";

fn store() -> IdentityStore { IdentityStore::default() }

/// Cite: openbao `vault/identity_store_entities.go:312`
/// (handleEntityUpdateCommon) — entity name is unique per namespace.
/// Re-upserting the same name returns the same entity ID and updates
/// policies / metadata in place.
#[test]
fn upsert_entity_is_idempotent_on_name() {
    let mut s = store();
    let id1 = s.upsert_entity(
        format!("{}-svc", TENANT),
        vec!["default".into()],
        [("tenant_id".to_string(), TENANT.to_string())].into(),
    );
    let id2 = s.upsert_entity(
        format!("{}-svc", TENANT),
        vec!["default".into(), "ops".into()],
        [("tenant_id".to_string(), TENANT.to_string())].into(),
    );
    assert_eq!(id1, id2, "same name ⇒ same entity ID");
    let e = s.entities.get(&id1).unwrap();
    assert_eq!(e.policies, vec!["default".to_string(), "ops".into()],
        "policies updated in place");
}

/// Cite: openbao `vault/identity_store_aliases.go:270`
/// (handleAliasCreate) — `(mount_accessor, alias_name)` is the
/// uniqueness constraint; trying to attach a second alias with the
/// same key returns an error.
#[test]
fn duplicate_alias_on_same_mount_accessor_is_rejected() {
    let mut s = store();
    let entity_id = s.upsert_entity("alice", vec!["default".into()], Default::default());
    let mount = format!("auth_userpass_{}", &TENANT[..6]);
    let alias_id = s.attach_entity_alias(&entity_id, &mount, "userpass", "alice").unwrap();
    assert!(!alias_id.is_empty());

    // Second attach on the SAME (mount, name) ⇒ error.
    let err = s.attach_entity_alias(&entity_id, &mount, "userpass", "alice").unwrap_err();
    assert!(err.contains("already exists"));
}

/// Cite: openbao `vault/identity_store.go::entityFromAlias` — the auth
/// pipeline turns a successful login into an entity by looking up
/// `(mount_accessor, alias_name)` ⇒ canonical_id ⇒ entity.
#[test]
fn entity_by_alias_resolves_back_to_canonical_entity() {
    let mut s = store();
    let entity_id = s.upsert_entity("bob", vec!["default".into()], Default::default());
    s.attach_entity_alias(&entity_id, "auth_oidc_abc", "oidc", "bob@example.com").unwrap();

    let resolved = s.entity_by_alias("auth_oidc_abc", "bob@example.com").expect("found");
    assert_eq!(resolved.id, entity_id);
    assert_eq!(resolved.name, "bob");
    // Wrong mount_accessor ⇒ no resolution (cross-mount alias not leaked).
    assert!(s.entity_by_alias("auth_oidc_xyz", "bob@example.com").is_none());
}

/// Cite: openbao `vault/identity_store_entities.go:535`
/// (pathEntityIDDelete) — deleting an entity drops its name index, every
/// alias it owned, AND removes it from any group memberships.
#[test]
fn delete_entity_cascades_to_aliases_and_group_memberships() {
    let mut s = store();
    let alice = s.upsert_entity("alice", vec!["default".into()], Default::default());
    let _ = s.attach_entity_alias(&alice, "auth_userpass_x", "userpass", "alice").unwrap();
    let _ = s.attach_entity_alias(&alice, "auth_oidc_x", "oidc", "alice@example.com").unwrap();
    let team_id = s.upsert_group("team-platform", GroupType::Internal,
        vec!["platform-policy".into()]);
    s.add_entity_to_group(&team_id, &alice).unwrap();

    assert!(s.entity_is_member(&team_id, &alice));
    let alias_ids_before: Vec<String> = s.entity_aliases.keys().cloned().collect();
    assert_eq!(alias_ids_before.len(), 2);

    assert!(s.delete_entity(&alice));
    assert!(s.entity_names.get("alice").is_none());
    assert!(s.entities.get(&alice).is_none());
    assert!(s.entity_aliases.is_empty(), "all aliases purged");
    assert!(!s.entity_is_member(&team_id, &alice),
        "membership purged from group");
}

/// Cite: openbao `vault/identity_store_groups.go:247`
/// (handleGroupUpdateCommon) — internal groups carry an explicit
/// `member_entity_ids` slice; adding the same entity twice is a no-op.
/// External groups reject direct membership writes.
#[test]
fn internal_group_supports_direct_membership_external_does_not() {
    let mut s = store();
    let alice = s.upsert_entity("alice", vec!["default".into()], Default::default());
    let internal = s.upsert_group("ops", GroupType::Internal,
        vec!["ops-policy".into()]);
    s.add_entity_to_group(&internal, &alice).unwrap();
    s.add_entity_to_group(&internal, &alice).unwrap();  // dedupe
    let g = s.groups.get(&internal).unwrap();
    assert_eq!(g.member_entity_ids, vec![alice.clone()]);

    // External groups reject direct membership.
    let external = s.upsert_group("oidc-admins", GroupType::External,
        vec!["admin-policy".into()]);
    let err = s.add_entity_to_group(&external, &alice).unwrap_err();
    assert!(err.contains("external"));
}

/// Cite: openbao `vault/identity_store_groups.go::isGroupMemberMatching`
/// — for external groups, an entity is a member iff one of its aliases
/// matches one of the group's group_aliases on `(mount_accessor, name)`.
#[test]
fn external_group_matches_members_via_alias_mount_accessor() {
    let mut s = store();
    // Alice has an OIDC alias whose name matches a future group_alias.
    let alice = s.upsert_entity("alice", vec!["default".into()], Default::default());
    s.attach_entity_alias(&alice, "auth_oidc_corp", "oidc", "platform-admins").unwrap();
    let bob = s.upsert_entity("bob", vec!["default".into()], Default::default());
    s.attach_entity_alias(&bob, "auth_oidc_corp", "oidc", "viewers").unwrap();

    let admins = s.upsert_group("oidc-admins", GroupType::External,
        vec!["admin-policy".into()]);
    s.attach_group_alias(&admins, "auth_oidc_corp", "oidc", "platform-admins").unwrap();

    assert!(s.entity_is_member(&admins, &alice));
    assert!(!s.entity_is_member(&admins, &bob),
        "bob's alias is in a different OIDC group");

    // Wrong mount_accessor (different OIDC mount) ⇒ no membership.
    let other_admins = s.upsert_group("other-admins", GroupType::External, vec![]);
    s.attach_group_alias(&other_admins, "auth_oidc_other", "oidc", "platform-admins").unwrap();
    assert!(!s.entity_is_member(&other_admins, &alice),
        "mount_accessor isolation enforced");
}

/// Cite: openbao `vault/identity_store_groups.go::collectPoliciesByEntityID`
/// — the effective policy set for an entity is the union of: the
/// entity's own policies + every group it belongs to (internal direct +
/// external alias-matched). Sorted, deduped.
#[test]
fn effective_policies_unions_entity_and_group_memberships() {
    let mut s = store();
    let alice = s.upsert_entity("alice",
        vec!["default".into(), "personal".into()], Default::default());
    s.attach_entity_alias(&alice, "auth_oidc_corp", "oidc", "engineers").unwrap();

    let ops = s.upsert_group("ops", GroupType::Internal,
        vec!["ops-policy".into()]);
    s.add_entity_to_group(&ops, &alice).unwrap();

    let oidc_eng = s.upsert_group("oidc-engineers", GroupType::External,
        vec!["engineering-policy".into()]);
    s.attach_group_alias(&oidc_eng, "auth_oidc_corp", "oidc", "engineers").unwrap();

    // Group bob belongs to NEITHER membership avenue; his policies stay personal.
    let bob = s.upsert_entity("bob",
        vec!["default".into()], Default::default());

    let alice_p = s.effective_policies(&alice);
    assert_eq!(alice_p, vec![
        "default".to_string(), "engineering-policy".into(),
        "ops-policy".into(), "personal".into(),
    ]);

    let bob_p = s.effective_policies(&bob);
    assert_eq!(bob_p, vec!["default".to_string()]);
}
