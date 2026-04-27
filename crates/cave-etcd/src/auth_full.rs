//! Auth full — user enable/disable (without delete), password policy,
//! role inheritance, per-user lockout, audit log.
//!
//! The base [`crate::store::KvStore`] handles user/role CRUD,
//! [`crate::auth_token`] handles tokens, [`crate::auth_v3`] handles the
//! perm cache + TLS-cert auth.  This module fills the remaining
//! v3.6-era surfaces:
//!
//!   * `UserDisable` / `UserEnable` — admin can revoke without losing
//!     the role assignments,
//!   * password policy validation against minimum length, character
//!     classes, and a deny-list,
//!   * role inheritance — a role may include other roles, so
//!     "permissions of role X" is the union over its closure,
//!   * lockout after N consecutive failed authentications,
//!   * structured audit log of every auth event.
//!
//! Mirrors etcd v3.6.10
//!   `server/auth/store.go#UserDisable / UserEnable` (proposed in
//!     etcd-io/etcd#15571 — landed in 3.6),
//!   `server/auth/store.go#authStore.passwordPolicy`,
//!   `server/auth/store.go#fetchPermInfoFromRole` (role chain walk).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum AuthFullError {
    /// User not found.
    UserNotFound(String),
    /// User is currently disabled.
    UserDisabled(String),
    /// User has been locked out due to too many failed auths.
    UserLockedOut(String),
    /// Password policy rejected the candidate.
    PasswordPolicy(String),
    /// Role inheritance loop detected.
    RoleCycle(String),
    /// Role not found.
    RoleNotFound(String),
    /// Caller tried to disable an already-disabled user (or vice versa).
    StateUnchanged,
}

impl std::fmt::Display for AuthFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserNotFound(u) => write!(f, "user not found: {u}"),
            Self::UserDisabled(u) => write!(f, "user disabled: {u}"),
            Self::UserLockedOut(u) => write!(f, "user locked out: {u}"),
            Self::PasswordPolicy(reason) => write!(f, "password policy: {reason}"),
            Self::RoleCycle(r) => write!(f, "role cycle through: {r}"),
            Self::RoleNotFound(r) => write!(f, "role not found: {r}"),
            Self::StateUnchanged => write!(f, "user already in requested state"),
        }
    }
}

impl std::error::Error for AuthFullError {}

// ── User state machine (enabled / disabled) ──────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserState {
    pub name: String,
    pub enabled: bool,
    pub roles: Vec<String>,
    /// Wall-clock timestamp the user was last toggled.  Unset on first add.
    pub last_toggle: Option<DateTime<Utc>>,
}

pub struct UserAdmin {
    inner: RwLock<BTreeMap<String, UserState>>,
}

impl UserAdmin {
    pub fn new() -> Self { Self { inner: RwLock::new(BTreeMap::new()) } }

    pub fn add_user(&self, name: impl Into<String>, roles: Vec<String>) {
        let name = name.into();
        self.inner.write().unwrap().insert(
            name.clone(),
            UserState { name, enabled: true, roles, last_toggle: None },
        );
    }

    pub fn delete_user(&self, name: &str) -> bool {
        self.inner.write().unwrap().remove(name).is_some()
    }

    pub fn is_enabled(&self, name: &str) -> Option<bool> {
        self.inner.read().unwrap().get(name).map(|u| u.enabled)
    }

    pub fn enable(&self, name: &str) -> Result<(), AuthFullError> {
        let mut g = self.inner.write().unwrap();
        let u = g.get_mut(name).ok_or_else(|| AuthFullError::UserNotFound(name.into()))?;
        if u.enabled { return Err(AuthFullError::StateUnchanged); }
        u.enabled = true;
        u.last_toggle = Some(Utc::now());
        Ok(())
    }

    pub fn disable(&self, name: &str) -> Result<(), AuthFullError> {
        let mut g = self.inner.write().unwrap();
        let u = g.get_mut(name).ok_or_else(|| AuthFullError::UserNotFound(name.into()))?;
        if !u.enabled { return Err(AuthFullError::StateUnchanged); }
        u.enabled = false;
        u.last_toggle = Some(Utc::now());
        Ok(())
    }

    pub fn user(&self, name: &str) -> Option<UserState> {
        self.inner.read().unwrap().get(name).cloned()
    }

    pub fn list(&self) -> Vec<UserState> {
        self.inner.read().unwrap().values().cloned().collect()
    }

    pub fn add_role_to_user(&self, name: &str, role: impl Into<String>) -> Result<(), AuthFullError> {
        let role = role.into();
        let mut g = self.inner.write().unwrap();
        let u = g.get_mut(name).ok_or_else(|| AuthFullError::UserNotFound(name.into()))?;
        if !u.roles.contains(&role) { u.roles.push(role); }
        Ok(())
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().len() }
}

impl Default for UserAdmin {
    fn default() -> Self { Self::new() }
}

// ── Password policy ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PasswordPolicy {
    pub min_length: usize,
    pub require_upper: bool,
    pub require_lower: bool,
    pub require_digit: bool,
    pub require_special: bool,
    pub deny_list: BTreeSet<String>,
}

impl PasswordPolicy {
    pub fn default_strict() -> Self {
        Self {
            min_length: 12,
            require_upper: true,
            require_lower: true,
            require_digit: true,
            require_special: true,
            deny_list: ["password", "12345678", "qwerty"].into_iter().map(String::from).collect(),
        }
    }

    pub fn lenient() -> Self {
        Self {
            min_length: 1,
            require_upper: false,
            require_lower: false,
            require_digit: false,
            require_special: false,
            deny_list: BTreeSet::new(),
        }
    }

    /// Validate a candidate password against the policy.  Returns
    /// `Err(PasswordPolicy(reason))` listing the FIRST failure observed.
    pub fn validate(&self, pw: &str) -> Result<(), AuthFullError> {
        if pw.len() < self.min_length {
            return Err(AuthFullError::PasswordPolicy(format!(
                "min length is {} (got {})", self.min_length, pw.len()
            )));
        }
        if self.deny_list.contains(&pw.to_lowercase()) {
            return Err(AuthFullError::PasswordPolicy("password on deny-list".into()));
        }
        if self.require_upper && !pw.chars().any(|c| c.is_ascii_uppercase()) {
            return Err(AuthFullError::PasswordPolicy("missing uppercase".into()));
        }
        if self.require_lower && !pw.chars().any(|c| c.is_ascii_lowercase()) {
            return Err(AuthFullError::PasswordPolicy("missing lowercase".into()));
        }
        if self.require_digit && !pw.chars().any(|c| c.is_ascii_digit()) {
            return Err(AuthFullError::PasswordPolicy("missing digit".into()));
        }
        if self.require_special && !pw.chars().any(|c| !c.is_ascii_alphanumeric()) {
            return Err(AuthFullError::PasswordPolicy("missing special".into()));
        }
        Ok(())
    }
}

// ── Role inheritance ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleNode {
    pub name: String,
    /// Permissions explicitly granted to this role.  Strings here just
    /// stand in for [`crate::auth_v3::PermInterval`] in tests.
    pub permissions: Vec<String>,
    /// Other roles whose permissions are merged into this one.
    pub includes: Vec<String>,
}

pub struct RoleGraph {
    inner: RwLock<BTreeMap<String, RoleNode>>,
}

impl RoleGraph {
    pub fn new() -> Self { Self { inner: RwLock::new(BTreeMap::new()) } }

    pub fn upsert(&self, role: RoleNode) {
        self.inner.write().unwrap().insert(role.name.clone(), role);
    }

    pub fn delete(&self, name: &str) -> bool {
        self.inner.write().unwrap().remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Option<RoleNode> {
        self.inner.read().unwrap().get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    /// Effective permissions = the union over the role's transitive
    /// closure of `includes`.  Detects cycles.
    pub fn effective_permissions(&self, name: &str) -> Result<Vec<String>, AuthFullError> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<String> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = vec![name.to_string()];
        while let Some(n) = stack.pop() {
            if !seen.insert(n.clone()) {
                return Err(AuthFullError::RoleCycle(n));
            }
            let node = g.get(&n).ok_or_else(|| AuthFullError::RoleNotFound(n.clone()))?;
            for p in &node.permissions {
                if !out.contains(p) { out.push(p.clone()); }
            }
            for inc in &node.includes {
                stack.push(inc.clone());
            }
        }
        out.sort();
        Ok(out)
    }
}

impl Default for RoleGraph {
    fn default() -> Self { Self::new() }
}

// ── Lockout policy ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LockoutPolicy {
    pub max_failed: u32,
    /// How long a lockout lasts.
    pub lockout_duration: chrono::Duration,
}

impl LockoutPolicy {
    pub fn default() -> Self {
        Self { max_failed: 5, lockout_duration: chrono::Duration::minutes(15) }
    }
}

#[derive(Debug, Clone)]
struct LockoutEntry {
    failed: u32,
    locked_until: Option<DateTime<Utc>>,
}

pub struct LockoutTracker {
    policy: LockoutPolicy,
    inner: RwLock<BTreeMap<String, LockoutEntry>>,
}

impl LockoutTracker {
    pub fn new(policy: LockoutPolicy) -> Self {
        Self { policy, inner: RwLock::new(BTreeMap::new()) }
    }

    /// Record a failed auth.  Returns true if this attempt triggered a
    /// fresh lockout.
    pub fn record_failure(&self, user: &str) -> bool {
        let mut g = self.inner.write().unwrap();
        let e = g.entry(user.to_string()).or_insert(LockoutEntry { failed: 0, locked_until: None });
        e.failed += 1;
        if e.failed >= self.policy.max_failed && e.locked_until.is_none() {
            e.locked_until = Some(Utc::now() + self.policy.lockout_duration);
            true
        } else {
            false
        }
    }

    /// Record a successful auth.  Clears any pending lockout state.
    pub fn record_success(&self, user: &str) {
        self.inner.write().unwrap().remove(user);
    }

    /// Check whether a user is currently locked out.  Auto-expires.
    pub fn is_locked(&self, user: &str) -> bool {
        let mut g = self.inner.write().unwrap();
        if let Some(e) = g.get_mut(user) {
            if let Some(until) = e.locked_until {
                if Utc::now() >= until {
                    g.remove(user);
                    return false;
                }
                return true;
            }
        }
        false
    }

    pub fn assert_unlocked(&self, user: &str) -> Result<(), AuthFullError> {
        if self.is_locked(user) {
            return Err(AuthFullError::UserLockedOut(user.into()));
        }
        Ok(())
    }

    pub fn fail_count(&self, user: &str) -> u32 {
        self.inner.read().unwrap().get(user).map(|e| e.failed).unwrap_or(0)
    }

    pub fn known_users(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    pub fn clear_user(&self, user: &str) -> bool {
        self.inner.write().unwrap().remove(user).is_some()
    }
}

// ── Audit log ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventKind {
    Authenticate,
    AuthenticateFail,
    UserAdd,
    UserDelete,
    UserDisable,
    UserEnable,
    UserChangePassword,
    RoleAdd,
    RoleDelete,
    RoleGrant,
    RoleRevoke,
    Lockout,
    LockoutCleared,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub seq: u64,
    pub timestamp: DateTime<Utc>,
    pub kind: AuditEventKind,
    pub subject: String,
    pub target: Option<String>,
    pub note: Option<String>,
}

pub struct AuditLog {
    inner: RwLock<VecDeque<AuditEvent>>,
    next_seq: AtomicU64,
    cap: usize,
}

impl AuditLog {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: RwLock::new(VecDeque::new()),
            next_seq: AtomicU64::new(1),
            cap,
        }
    }

    pub fn record(&self, kind: AuditEventKind, subject: impl Into<String>, target: Option<String>, note: Option<String>) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let mut g = self.inner.write().unwrap();
        if g.len() >= self.cap { g.pop_front(); }
        g.push_back(AuditEvent {
            seq, timestamp: Utc::now(), kind,
            subject: subject.into(), target, note,
        });
        seq
    }

    pub fn entries(&self) -> Vec<AuditEvent> {
        self.inner.read().unwrap().iter().cloned().collect()
    }

    pub fn entries_for(&self, subject: &str) -> Vec<AuditEvent> {
        self.inner.read().unwrap().iter()
            .filter(|e| e.subject == subject)
            .cloned()
            .collect()
    }

    pub fn entries_of_kind(&self, kind: AuditEventKind) -> Vec<AuditEvent> {
        self.inner.read().unwrap().iter()
            .filter(|e| e.kind == kind)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M7 deeper
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn admin_with_alice() -> UserAdmin {
        let a = UserAdmin::new();
        a.add_user("alice", vec!["root".into()]);
        a
    }

    // ── UserAdmin enable/disable ──────────────────────────────────────

    #[test]
    fn test_user_default_enabled() {
        // cite: server/auth/store.go (new user defaults to enabled)
        let a = admin_with_alice();
        assert_eq!(a.is_enabled("alice"), Some(true));
    }

    #[test]
    fn test_user_disable_then_check() {
        // cite: store.go UserDisable
        let a = admin_with_alice();
        a.disable("alice").unwrap();
        assert_eq!(a.is_enabled("alice"), Some(false));
    }

    #[test]
    fn test_user_disable_unknown_errors() {
        let a = UserAdmin::new();
        assert!(matches!(a.disable("ghost").unwrap_err(), AuthFullError::UserNotFound(_)));
    }

    #[test]
    fn test_user_disable_already_disabled_errors() {
        // cite: idempotency: don't bump last_toggle if no change
        let a = admin_with_alice();
        a.disable("alice").unwrap();
        assert_eq!(a.disable("alice").unwrap_err(), AuthFullError::StateUnchanged);
    }

    #[test]
    fn test_user_enable_after_disable() {
        // cite: store.go UserEnable
        let a = admin_with_alice();
        a.disable("alice").unwrap();
        a.enable("alice").unwrap();
        assert_eq!(a.is_enabled("alice"), Some(true));
    }

    #[test]
    fn test_user_enable_already_enabled_errors() {
        let a = admin_with_alice();
        assert_eq!(a.enable("alice").unwrap_err(), AuthFullError::StateUnchanged);
    }

    #[test]
    fn test_user_disable_preserves_roles() {
        // cite: store.go (UserDisable doesn't touch roles)
        let a = admin_with_alice();
        a.disable("alice").unwrap();
        assert_eq!(a.user("alice").unwrap().roles, vec!["root"]);
    }

    #[test]
    fn test_user_delete_then_disable_unknown() {
        let a = admin_with_alice();
        a.delete_user("alice");
        assert!(matches!(a.disable("alice").unwrap_err(), AuthFullError::UserNotFound(_)));
    }

    #[test]
    fn test_user_add_role_dedup() {
        let a = admin_with_alice();
        a.add_role_to_user("alice", "root").unwrap();
        a.add_role_to_user("alice", "root").unwrap();
        assert_eq!(a.user("alice").unwrap().roles.len(), 1);
    }

    #[test]
    fn test_user_disable_records_last_toggle() {
        let a = admin_with_alice();
        let before = Utc::now();
        a.disable("alice").unwrap();
        let toggled_at = a.user("alice").unwrap().last_toggle.unwrap();
        assert!(toggled_at >= before);
    }

    // ── PasswordPolicy ────────────────────────────────────────────────

    #[test]
    fn test_password_policy_lenient_accepts_any() {
        // cite: lenient/no-policy mode
        let p = PasswordPolicy::lenient();
        assert!(p.validate("a").is_ok());
    }

    #[test]
    fn test_password_policy_min_length() {
        // cite: store.go passwordPolicy
        let p = PasswordPolicy::default_strict();
        assert!(p.validate("Aa1!short").is_err());
    }

    #[test]
    fn test_password_policy_strict_accepts_strong() {
        let p = PasswordPolicy::default_strict();
        p.validate("CorrectHorse9!Battery").unwrap();
    }

    #[test]
    fn test_password_policy_missing_upper() {
        let p = PasswordPolicy::default_strict();
        match p.validate("correcthorse9!battery").unwrap_err() {
            AuthFullError::PasswordPolicy(m) => assert!(m.contains("uppercase")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_password_policy_missing_lower() {
        let p = PasswordPolicy::default_strict();
        match p.validate("CORRECTHORSE9!BATTERY").unwrap_err() {
            AuthFullError::PasswordPolicy(m) => assert!(m.contains("lowercase")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_password_policy_missing_digit() {
        let p = PasswordPolicy::default_strict();
        match p.validate("CorrectHorse!Battery").unwrap_err() {
            AuthFullError::PasswordPolicy(m) => assert!(m.contains("digit")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_password_policy_missing_special() {
        let p = PasswordPolicy::default_strict();
        match p.validate("CorrectHorse9Battery").unwrap_err() {
            AuthFullError::PasswordPolicy(m) => assert!(m.contains("special")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_password_policy_deny_list_blocks_common() {
        // cite: top-N common passwords blocked
        let p = PasswordPolicy::lenient();
        let mut p2 = p.clone();
        p2.deny_list.insert("password".into());
        assert!(p2.validate("password").is_err());
    }

    #[test]
    fn test_password_policy_deny_list_case_insensitive() {
        let mut p = PasswordPolicy::lenient();
        p.deny_list.insert("password".into());
        assert!(p.validate("Password").is_err());
    }

    // ── RoleGraph inheritance ─────────────────────────────────────────

    #[test]
    fn test_role_effective_permissions_simple() {
        // cite: store.go fetchPermInfoFromRole
        let g = RoleGraph::new();
        g.upsert(RoleNode { name: "r1".into(), permissions: vec!["p1".into()], includes: vec![] });
        let perms = g.effective_permissions("r1").unwrap();
        assert_eq!(perms, vec!["p1"]);
    }

    #[test]
    fn test_role_effective_permissions_with_includes() {
        // cite: nested role inheritance
        let g = RoleGraph::new();
        g.upsert(RoleNode { name: "base".into(), permissions: vec!["p1".into()], includes: vec![] });
        g.upsert(RoleNode { name: "ops".into(), permissions: vec!["p2".into()], includes: vec!["base".into()] });
        let perms = g.effective_permissions("ops").unwrap();
        assert!(perms.contains(&"p1".to_string()));
        assert!(perms.contains(&"p2".to_string()));
    }

    #[test]
    fn test_role_effective_permissions_dedup() {
        // cite: union over closure (no duplicates)
        let g = RoleGraph::new();
        g.upsert(RoleNode { name: "a".into(), permissions: vec!["x".into()], includes: vec![] });
        g.upsert(RoleNode { name: "b".into(), permissions: vec!["x".into()], includes: vec!["a".into()] });
        let perms = g.effective_permissions("b").unwrap();
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn test_role_effective_permissions_cycle_errors() {
        // cite: server/auth (cycle detection)
        let g = RoleGraph::new();
        g.upsert(RoleNode { name: "a".into(), permissions: vec![], includes: vec!["b".into()] });
        g.upsert(RoleNode { name: "b".into(), permissions: vec![], includes: vec!["a".into()] });
        assert!(matches!(g.effective_permissions("a").unwrap_err(), AuthFullError::RoleCycle(_)));
    }

    #[test]
    fn test_role_effective_permissions_unknown_role() {
        let g = RoleGraph::new();
        assert!(matches!(g.effective_permissions("ghost").unwrap_err(), AuthFullError::RoleNotFound(_)));
    }

    #[test]
    fn test_role_unknown_in_includes_errors() {
        // cite: store.go (forward-ref resolution)
        let g = RoleGraph::new();
        g.upsert(RoleNode { name: "a".into(), permissions: vec![], includes: vec!["ghost".into()] });
        assert!(matches!(g.effective_permissions("a").unwrap_err(), AuthFullError::RoleNotFound(_)));
    }

    #[test]
    fn test_role_delete_returns_true() {
        let g = RoleGraph::new();
        g.upsert(RoleNode { name: "r".into(), permissions: vec![], includes: vec![] });
        assert!(g.delete("r"));
    }

    #[test]
    fn test_role_names_sorted() {
        let g = RoleGraph::new();
        for n in ["c", "a", "b"] {
            g.upsert(RoleNode { name: n.into(), permissions: vec![], includes: vec![] });
        }
        assert_eq!(g.names(), vec!["a", "b", "c"]);
    }

    // ── Lockout ───────────────────────────────────────────────────────

    #[test]
    fn test_lockout_after_max_failed() {
        // cite: defensive (rate-limit brute force)
        let l = LockoutTracker::new(LockoutPolicy { max_failed: 3, lockout_duration: chrono::Duration::seconds(60) });
        assert!(!l.record_failure("alice"));
        assert!(!l.record_failure("alice"));
        assert!(l.record_failure("alice"));
        assert!(l.is_locked("alice"));
    }

    #[test]
    fn test_lockout_success_clears_failures() {
        let l = LockoutTracker::new(LockoutPolicy { max_failed: 3, lockout_duration: chrono::Duration::seconds(60) });
        l.record_failure("alice");
        l.record_failure("alice");
        l.record_success("alice");
        assert_eq!(l.fail_count("alice"), 0);
    }

    #[test]
    fn test_lockout_assert_unlocked_blocks_when_locked() {
        let l = LockoutTracker::new(LockoutPolicy { max_failed: 1, lockout_duration: chrono::Duration::seconds(60) });
        l.record_failure("alice");
        assert!(matches!(l.assert_unlocked("alice").unwrap_err(), AuthFullError::UserLockedOut(_)));
    }

    #[test]
    fn test_lockout_expires_after_duration() {
        // cite: time-bounded lockout
        let l = LockoutTracker::new(LockoutPolicy { max_failed: 1, lockout_duration: chrono::Duration::milliseconds(1) });
        l.record_failure("alice");
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(!l.is_locked("alice"));
    }

    #[test]
    fn test_lockout_clear_user() {
        let l = LockoutTracker::new(LockoutPolicy { max_failed: 1, lockout_duration: chrono::Duration::seconds(60) });
        l.record_failure("alice");
        assert!(l.clear_user("alice"));
        assert!(!l.is_locked("alice"));
    }

    #[test]
    fn test_lockout_known_users_lists_failed() {
        let l = LockoutTracker::new(LockoutPolicy { max_failed: 5, lockout_duration: chrono::Duration::seconds(60) });
        l.record_failure("alice");
        l.record_failure("bob");
        let mut users = l.known_users();
        users.sort();
        assert_eq!(users, vec!["alice", "bob"]);
    }

    #[test]
    fn test_lockout_only_first_trigger_returns_true() {
        // cite: only the threshold-crossing failure starts a lockout
        let l = LockoutTracker::new(LockoutPolicy { max_failed: 2, lockout_duration: chrono::Duration::seconds(60) });
        assert!(!l.record_failure("alice"));
        assert!(l.record_failure("alice"));
        // Already locked — subsequent failures don't return "fresh lockout".
        assert!(!l.record_failure("alice"));
    }

    // ── AuditLog ──────────────────────────────────────────────────────

    #[test]
    fn test_audit_record_returns_seq() {
        let log = AuditLog::new(64);
        let s1 = log.record(AuditEventKind::UserAdd, "alice", None, None);
        let s2 = log.record(AuditEventKind::UserDelete, "alice", None, None);
        assert!(s2 > s1);
    }

    #[test]
    fn test_audit_capped_drops_oldest() {
        let log = AuditLog::new(2);
        for i in 0..5 {
            log.record(AuditEventKind::Authenticate, format!("u{i}"), None, None);
        }
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn test_audit_filter_by_subject() {
        let log = AuditLog::new(16);
        log.record(AuditEventKind::Authenticate, "alice", None, None);
        log.record(AuditEventKind::Authenticate, "bob", None, None);
        log.record(AuditEventKind::Authenticate, "alice", None, None);
        assert_eq!(log.entries_for("alice").len(), 2);
    }

    #[test]
    fn test_audit_filter_by_kind() {
        let log = AuditLog::new(16);
        log.record(AuditEventKind::UserAdd, "alice", None, None);
        log.record(AuditEventKind::AuthenticateFail, "alice", None, None);
        log.record(AuditEventKind::AuthenticateFail, "bob", None, None);
        assert_eq!(log.entries_of_kind(AuditEventKind::AuthenticateFail).len(), 2);
    }

    #[test]
    fn test_audit_records_target_and_note() {
        let log = AuditLog::new(16);
        log.record(AuditEventKind::RoleGrant, "admin", Some("alice".into()), Some("emergency".into()));
        let e = &log.entries()[0];
        assert_eq!(e.target.as_deref(), Some("alice"));
        assert_eq!(e.note.as_deref(), Some("emergency"));
    }
}
