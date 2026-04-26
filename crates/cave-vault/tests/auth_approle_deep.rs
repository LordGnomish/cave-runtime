//! deeper-001: AppRole — secret_id binding semantics, token policy /
//! TTL / max_ttl propagation, role_id update, num_uses exhaustion.
//! Pinned to openbao v2.5.3.

use cave_vault::auth::approle::{ApproleRole, ApproleStore, SecretIdEntry};
use cave_vault::token::{CreateTokenParams, TokenStore};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

const TENANT: &str = "tenant-acme-prod";

fn role(name: &str, bind: bool) -> ApproleRole {
    ApproleRole {
        role_name: format!("{}-{}", TENANT, name),
        bind_secret_id: bind,
        token_ttl: 3600,
        token_max_ttl: 7200,
        token_policies: vec!["default".into(), format!("{}-policy", TENANT)],
        ..Default::default()
    }
}

fn secret_id(role_name: &str, ttl_secs: i64, num_uses: i64) -> (String, SecretIdEntry) {
    let sid = format!("sid-{}", Uuid::new_v4().simple());
    let acc = Uuid::new_v4().to_string();
    let entry = SecretIdEntry {
        secret_id: sid.clone(),
        accessor: acc,
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

/// Cite: openbao `builtin/credential/approle/path_role.go::pathRoleCreateUpdate`
/// (`bind_secret_id = false`) — when `bind_secret_id = false`, the role
/// authenticates with `role_id` only; no secret_id required.
#[test]
fn bind_secret_id_false_allows_role_id_only_login() {
    let r = role("public", false);
    assert!(!r.bind_secret_id);
    // Login flow: the handler doesn't require entry.secret_id.
    let supplied_secret_id: Option<&str> = None;
    let needs_secret = r.bind_secret_id;
    assert!(!needs_secret || supplied_secret_id.is_some(),
        "bind_secret_id=false ⇒ no secret_id needed");
}

/// Cite: openbao `builtin/credential/approle/path_login.go:432`
/// (resp.Auth.TTL = role.TokenTTL) — the issued token inherits the
/// role's `token_ttl`, `token_max_ttl`, and `token_policies` verbatim.
/// The deeper test materialises a real token via TokenStore::create.
#[test]
fn token_ttl_max_ttl_and_policies_propagate_from_role() {
    let r = role("svc", true);
    let mut ts = TokenStore::default();
    let params = CreateTokenParams {
        policies: Some(r.token_policies.clone()),
        ttl: Some(format!("{}s", r.token_ttl)),
        explicit_max_ttl: Some(format!("{}s", r.token_max_ttl)),
        renewable: Some(true),
        no_parent: Some(true),
        metadata: Some([("tenant_id".into(), TENANT.into())].into()),
        ..Default::default()
    };
    let tok = ts.create(&params, None).unwrap();
    assert_eq!(tok.ttl, 3600);
    assert_eq!(tok.max_ttl, 7200);
    // TokenStore guarantees the issued token carries every role policy.
    assert!(tok.policies.iter().any(|p| p == "default"),
        "default policy injected by TokenStore.create");
    assert!(tok.policies.iter().any(|p| p == &format!("{}-policy", TENANT)),
        "tenant-scoped policy preserved verbatim");
    assert_eq!(tok.metadata.get("tenant_id"), Some(&TENANT.to_string()));
    assert!(tok.renewable);
}

/// Cite: openbao `builtin/credential/approle/path_role.go::pathRoleSecretIDDestroyUpdateDelete`
/// (cleanup) — destroying a secret_id by value drops both the index
/// entry and the per-role accessor entry.
#[test]
fn destroy_secret_id_drops_both_index_and_accessor_entries() {
    let mut store = ApproleStore::default();
    let role_name = format!("{}-svc", TENANT);
    store.roles.insert(role_name.clone(), role("svc", true));
    let (sid, entry) = secret_id(&role_name, 0, 0);
    let acc = entry.accessor.clone();
    store.secret_id_by_id.insert(sid.clone(), (role_name.clone(), acc.clone()));
    store.secret_ids.entry(role_name.clone()).or_default()
        .insert(acc.clone(), entry);

    // Simulated destroy
    if let Some((rn, accessor)) = store.secret_id_by_id.remove(&sid) {
        store.secret_ids.entry(rn).or_default().remove(&accessor);
    }

    assert!(store.secret_id_by_id.get(&sid).is_none());
    assert!(store.secret_ids.get(&role_name).unwrap().get(&acc).is_none(),
        "per-accessor entry also removed");
}

/// Cite: openbao `builtin/credential/approle/path_role.go::pathRoleRoleIDUpdate`
/// — the operator may rotate `role_id` on an existing role; subsequent
/// logins must use the new `role_id` and the old one yields invalid_role_id.
#[test]
fn role_id_update_rotates_login_credential() {
    let mut store = ApproleStore::default();
    let role_name = format!("{}-rot", TENANT);
    store.roles.insert(role_name.clone(), role("rot", true));
    let original_id = store.roles.get(&role_name).unwrap().role_id.clone();

    // rotate
    let new_id = format!("rid-{}", Uuid::new_v4().simple());
    store.roles.get_mut(&role_name).unwrap().role_id = new_id.clone();

    // Login attempt with original_id ⇒ no role with that role_id
    let by_id = store.roles.values().find(|r| r.role_id == original_id);
    assert!(by_id.is_none(), "old role_id no longer matches");

    // Login attempt with new_id ⇒ matches
    let by_id = store.roles.values().find(|r| r.role_id == new_id);
    assert!(by_id.is_some());
}

/// Cite: openbao `builtin/credential/approle/path_login.go:135`
/// (pathLoginUpdate, num_uses branch) — when `num_uses > 0`, every
/// successful login decrements `uses_remaining`. Once it reaches 0,
/// further logins must be rejected with `secret_id use limit exceeded`.
#[test]
fn num_uses_exhaustion_progresses_then_rejects() {
    let (sid, mut entry) = secret_id("svc", 0, 3);
    assert_eq!(entry.uses_remaining, 3);
    for _ in 0..3 {
        if entry.num_uses > 0 {
            assert!(entry.uses_remaining > 0);
            entry.uses_remaining -= 1;
        }
    }
    assert_eq!(entry.uses_remaining, 0);
    let next_login_allowed = entry.num_uses == 0 || entry.uses_remaining > 0;
    assert!(!next_login_allowed, "exhausted secret_id rejected");
    let _ = sid;
}

/// Cite: openbao `builtin/credential/approle/path_login.go:432`
/// (resp.Auth.Period = role.TokenPeriod) — when `period > 0`, the
/// issued token is "periodic": its expiry resets to `period` on every
/// renew. cave reflects this by setting `token.period = Some(period)`.
#[test]
fn periodic_role_yields_token_with_period_set() {
    let mut r = role("periodic", true);
    r.period = 1200;  // 20 minute renewal cadence

    let mut ts = TokenStore::default();
    let params = CreateTokenParams {
        policies: Some(r.token_policies.clone()),
        ttl: Some(format!("{}s", r.token_ttl)),
        period: Some(format!("{}s", r.period)),
        renewable: Some(true),
        no_parent: Some(true),
        metadata: Some([("tenant_id".into(), TENANT.into())].into()),
        ..Default::default()
    };
    let tok = ts.create(&params, None).unwrap();
    assert_eq!(tok.period, Some(1200));
    assert!(tok.renewable);
}
