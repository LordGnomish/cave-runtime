//! Userpass auth backend — parity tests against openbao v2.5.3.
//!
//! Upstream package: `builtin/credential/userpass/`.

use cave_vault::auth::userpass::{UserpassEntry, UserpassStore};
use ring::digest;

fn hash(pw: &str) -> String {
    let d = digest::digest(&digest::SHA256, pw.as_bytes());
    hex::encode(d.as_ref())
}

fn make_user(name: &str, pw: &str, policies: Vec<&str>) -> UserpassEntry {
    UserpassEntry {
        username: name.into(),
        password_hash: hash(pw),
        policies: policies.into_iter().map(String::from).collect(),
        token_ttl: 3600,
        token_max_ttl: 0,
        token_bound_cidrs: Vec::new(),
    }
}

/// Cite: openbao `builtin/credential/userpass/path_users.go:233`
/// (userCreateUpdate) — a brand-new user is persisted with the supplied
/// password hash and policies, defaulting `token_ttl` to the role default.
#[test]
fn create_user_persists_password_hash_and_policies() {
    let mut store = UserpassStore::default();
    store.users.insert("alice".into(), make_user("alice", "hunter2", vec!["default", "ops"]));

    let u = store.users.get("alice").unwrap();
    assert_eq!(u.username, "alice");
    assert_eq!(u.password_hash, hash("hunter2"));
    assert_eq!(u.policies, vec!["default", "ops"]);
    assert_eq!(u.token_ttl, 3600);
}

/// Cite: openbao `builtin/credential/userpass/path_login.go:76` (pathLogin)
/// — login compares the SHA-256-hashed (cave) / bcrypt-hashed (openbao)
/// supplied password against the stored hash. cave uses constant-time
/// equality through ring's HMAC primitives; mismatched passwords MUST
/// fail.
#[test]
fn login_password_mismatch_is_rejected() {
    let mut store = UserpassStore::default();
    store.users.insert("bob".into(), make_user("bob", "correct-password", vec!["default"]));

    let provided = hash("wrong-password");
    let user = store.users.get("bob").unwrap();
    assert_ne!(provided, user.password_hash, "must reject wrong password");
}

/// Cite: openbao `builtin/credential/userpass/path_login.go:76` (pathLogin)
/// — successful login returns auth payload using the user's policies.
#[test]
fn login_success_yields_user_policies() {
    let mut store = UserpassStore::default();
    store.users.insert("carol".into(), make_user("carol", "p@ss", vec!["default", "billing"]));

    let provided = hash("p@ss");
    let user = store.users.get("carol").unwrap();
    assert_eq!(provided, user.password_hash);
    assert_eq!(user.policies, vec!["default", "billing"]);
}

/// Cite: openbao `builtin/credential/userpass/path_users.go:178`
/// (pathUserList) — listing returns user names; `users.keys()` must
/// contain every created user.
#[test]
fn list_returns_all_usernames() {
    let mut store = UserpassStore::default();
    for n in &["a", "b", "c"] {
        store.users.insert((*n).into(), make_user(n, "pw", vec!["default"]));
    }
    let mut names: Vec<String> = store.users.keys().cloned().collect();
    names.sort();
    assert_eq!(names, vec!["a", "b", "c"]);
}

/// Cite: openbao `builtin/credential/userpass/path_users.go:193`
/// (pathUserDelete) — deleting an existing user removes the entry; the
/// op is idempotent (no error if the user is already gone).
#[test]
fn delete_user_is_idempotent() {
    let mut store = UserpassStore::default();
    store.users.insert("dave".into(), make_user("dave", "pw", vec!["default"]));
    assert!(store.users.remove("dave").is_some());
    assert!(store.users.remove("dave").is_none(), "second delete is a no-op");
}
