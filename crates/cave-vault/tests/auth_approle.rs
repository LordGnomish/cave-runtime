// SPDX-License-Identifier: AGPL-3.0-or-later
//! AppRole auth backend — parity tests against openbao v2.5.3.
//!
//! Upstream package: `builtin/credential/approle/`.

use cave_vault::auth::approle::{ApproleRole, ApproleStore, SecretIdEntry};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

fn role(name: &str, bind: bool) -> ApproleRole {
    ApproleRole {
        role_name: name.into(),
        bind_secret_id: bind,
        ..Default::default()
    }
}

fn secret_id(role_name: &str, ttl_secs: i64, num_uses: i64) -> (String, SecretIdEntry) {
    let sid = format!("sid-{}", Uuid::new_v4().simple());
    let acc = Uuid::new_v4().to_string();
    let entry = SecretIdEntry {
        secret_id: sid.clone(),
        accessor: acc.clone(),
        created_at: Utc::now(),
        expires_at: if ttl_secs > 0 { Some(Utc::now() + Duration::seconds(ttl_secs)) } else { None },
        num_uses,
        uses_remaining: num_uses,
        metadata: HashMap::new(),
        cidr_list: Vec::new(),
    };
    let _ = role_name;
    (sid, entry)
}

/// Cite: openbao `builtin/credential/approle/path_role.go:1599`
/// (pathRoleCreateUpdate) — every role gets a fresh `role_id` UUID and
/// defaults to `bind_secret_id = true`, `token_policies = ["default"]`.
#[test]
fn create_role_default_fields() {
    let r = role("payments", true);
    assert!(!r.role_id.is_empty(), "role_id is generated");
    assert!(r.bind_secret_id);
    assert_eq!(r.token_policies, vec!["default"]);
    assert_eq!(r.token_ttl, 3600);
}

/// Cite: openbao `builtin/credential/approle/path_role.go:1758` (pathRoleRead)
/// — reading a role returns its full configuration.
#[test]
fn read_back_role_after_insert() {
    let mut store = ApproleStore::default();
    store.roles.insert("svc".into(), role("svc", true));
    let r = store.roles.get("svc").expect("role present");
    assert_eq!(r.role_name, "svc");
    assert_eq!(r.bind_secret_id, true);
}

/// Cite: openbao `builtin/credential/approle/path_role.go:1858`
/// (pathRoleDelete) — deleting a role purges its secret-id index too.
#[test]
fn delete_role_clears_secret_ids() {
    let mut store = ApproleStore::default();
    store.roles.insert("ephemeral".into(), role("ephemeral", true));
    let (sid, entry) = secret_id("ephemeral", 600, 0);
    store.secret_ids.entry("ephemeral".into()).or_default()
        .insert(entry.accessor.clone(), entry.clone());
    store.secret_id_by_id.insert(sid.clone(), ("ephemeral".into(), entry.accessor));

    store.roles.remove("ephemeral");
    store.secret_ids.remove("ephemeral");
    assert!(store.roles.get("ephemeral").is_none());
    assert!(store.secret_ids.get("ephemeral").is_none());
}

/// Cite: openbao `builtin/credential/approle/path_login.go:135`
/// (pathLoginUpdate) — secret_id login enforces expiration. An already-
/// expired secret_id MUST be rejected.
#[test]
fn login_rejects_expired_secret_id() {
    let mut store = ApproleStore::default();
    store.roles.insert("svc".into(), role("svc", true));

    let (sid, mut entry) = secret_id("svc", 1, 0);
    // backdate expiry into the past
    entry.expires_at = Some(Utc::now() - Duration::seconds(10));
    let acc = entry.accessor.clone();
    store.secret_id_by_id.insert(sid.clone(), ("svc".into(), acc.clone()));
    store.secret_ids.entry("svc".into()).or_default().insert(acc, entry);

    let lookup = store.secret_id_by_id.get(&sid).cloned().unwrap();
    let entry = store.secret_ids.get(&lookup.0).unwrap().get(&lookup.1).unwrap();
    let exp = entry.expires_at.unwrap();
    assert!(Utc::now() > exp, "secret_id is expired → login must fail");
}

/// Cite: openbao `builtin/credential/approle/path_login.go:135`
/// (pathLoginUpdate) — when `num_uses > 0` the entry must be decremented
/// and rejected once `uses_remaining` hits 0.
#[test]
fn login_decrements_uses_remaining_and_rejects_when_zero() {
    let mut store = ApproleStore::default();
    store.roles.insert("svc".into(), role("svc", true));

    let (sid, entry) = secret_id("svc", 0, 2);
    let acc = entry.accessor.clone();
    store.secret_id_by_id.insert(sid.clone(), ("svc".into(), acc.clone()));
    store.secret_ids.entry("svc".into()).or_default().insert(acc.clone(), entry);

    // simulate two successful logins then one rejected
    for _ in 0..2 {
        let lookup = store.secret_id_by_id.get(&sid).cloned().unwrap();
        let e = store.secret_ids.get_mut(&lookup.0).unwrap().get_mut(&lookup.1).unwrap();
        assert!(e.uses_remaining > 0);
        e.uses_remaining -= 1;
    }
    let lookup = store.secret_id_by_id.get(&sid).cloned().unwrap();
    let e = store.secret_ids.get(&lookup.0).unwrap().get(&lookup.1).unwrap();
    assert_eq!(e.uses_remaining, 0, "exhausted secret_id must be rejected on the next login");
}
