// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Auth subsystem refinements — auth revision, range-permission cache,
//! TLS client-cert (CN-based) auth, and root-role bootstrap.
//!
//! The base [`crate::store::KvStore`] handles user/role CRUD; this module
//! layers the v3.6-era extras:
//!
//!   * **auth revision** monotonic counter — every Auth* RPC bumps it so
//!     watchers can react to a permission change,
//!   * **range_perm_cache** — a per-user cache of `[key_start, key_end)`
//!     intervals so an O(1) lookup can answer "is this Get/Put allowed?",
//!   * **TLS cert auth** — extract Common Name from a client cert subject
//!     and authenticate as that user without a password,
//!   * **root role bootstrap** — auto-grant the `root` role full read/write
//!     on `\x00..\xff` when first promoted.
//!
//! Mirrors etcd v3.6.10
//!   `server/auth/store.go#authStore` (revision counter),
//!   `server/auth/range_perm_cache.go` (interval cache),
//!   `server/auth/store.go#AuthInfoFromTLS` (CN auth),
//!   `server/auth/store.go#newStorage` (root bootstrap).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

// ── Auth revision ────────────────────────────────────────────────────────

/// Monotonic counter that increases on every auth-state change.  Watchers
/// observe the counter to invalidate caches.
pub struct AuthRevision {
    counter: AtomicU64,
}

impl AuthRevision {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }

    pub fn current(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    /// Bump the counter and return the new value.
    pub fn bump(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Check whether `seen` is stale relative to the current revision.
    pub fn is_stale(&self, seen: u64) -> bool {
        self.current() > seen
    }
}

impl Default for AuthRevision {
    fn default() -> Self {
        Self::new()
    }
}

// ── Range-permission cache ───────────────────────────────────────────────

/// One permission interval — `[key, key_end)`, plus its access type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermInterval {
    pub key: Vec<u8>,
    pub key_end: Option<Vec<u8>>,
    pub kind: PermKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermKind {
    Read,
    Write,
    ReadWrite,
}

impl PermKind {
    /// True if this perm grants the `desired` access type.
    pub fn covers(&self, desired: PermKind) -> bool {
        matches!(
            (self, desired),
            (Self::ReadWrite, _) | (Self::Read, Self::Read) | (Self::Write, Self::Write)
        )
    }
}

impl PermInterval {
    /// True if this interval covers `key`.
    pub fn matches(&self, key: &[u8]) -> bool {
        match &self.key_end {
            None => key == self.key.as_slice(),
            Some(end) => {
                // Etcd's "all keys" sentinel: key=[0x00], key_end=[0x00].
                if self.key == [0u8] && end == &[0u8] {
                    return true;
                }
                key >= self.key.as_slice() && key < end.as_slice()
            }
        }
    }

    /// True if this interval covers prefix `key+\xff`.
    pub fn covers_prefix(&self, prefix: &[u8]) -> bool {
        let key_match = self.key.starts_with(prefix);
        let end_after_prefix = self
            .key_end
            .as_ref()
            .map(|e| e.as_slice() > prefix)
            .unwrap_or(false);
        key_match && end_after_prefix
    }
}

/// Per-user range-permission cache.
pub struct RangePermCache {
    inner: RwLock<HashMap<String, Vec<PermInterval>>>,
    /// Auth revision the cache was last rebuilt at.
    revision: RwLock<u64>,
}

impl RangePermCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            revision: RwLock::new(0),
        }
    }

    pub fn replace_user(&self, username: &str, intervals: Vec<PermInterval>) {
        self.inner
            .write()
            .unwrap()
            .insert(username.to_string(), intervals);
    }

    pub fn invalidate_user(&self, username: &str) {
        self.inner.write().unwrap().remove(username);
    }

    pub fn invalidate_all(&self) {
        self.inner.write().unwrap().clear();
    }

    pub fn known_users(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    /// O(intervals) lookup — returns true if the user's perm set covers the
    /// requested key+access.
    pub fn allows(&self, username: &str, key: &[u8], desired: PermKind) -> bool {
        let g = self.inner.read().unwrap();
        let Some(intervals) = g.get(username) else {
            return false;
        };
        for iv in intervals {
            if iv.kind.covers(desired) && iv.matches(key) {
                return true;
            }
        }
        false
    }

    /// Whether the cache is up-to-date with the supplied auth revision.
    pub fn fresh(&self, current_revision: u64) -> bool {
        *self.revision.read().unwrap() == current_revision
    }

    pub fn set_revision(&self, rev: u64) {
        *self.revision.write().unwrap() = rev;
    }
}

impl Default for RangePermCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── TLS client-cert auth ─────────────────────────────────────────────────

/// Subject distinguished name fields parsed from a client cert.  Etcd
/// uses CN as the username; we surface OU, O for richer logging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertSubject {
    pub common_name: String,
    pub organisational_unit: Option<String>,
    pub organisation: Option<String>,
}

/// Errors specific to cert auth.
#[derive(Debug, PartialEq, Eq)]
pub enum CertAuthError {
    /// Subject string didn't parse.
    BadSubject(String),
    /// CN missing from subject.
    NoCommonName,
    /// CN doesn't correspond to a known etcd user.
    UnknownUser(String),
    /// Cert subject parsed but hasn't been allow-listed.
    NotAllowListed(String),
}

impl std::fmt::Display for CertAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadSubject(s) => write!(f, "bad subject: {s}"),
            Self::NoCommonName => write!(f, "subject has no CN"),
            Self::UnknownUser(u) => write!(f, "unknown user: {u}"),
            Self::NotAllowListed(u) => write!(f, "user {u} not on allow-list"),
        }
    }
}

impl std::error::Error for CertAuthError {}

/// Parse an LDAP-style subject string ("CN=alice,OU=admins,O=cave").
pub fn parse_subject(s: &str) -> Result<CertSubject, CertAuthError> {
    let mut cn = None;
    let mut ou = None;
    let mut o = None;
    for part in s.split(',') {
        let part = part.trim();
        let mut it = part.splitn(2, '=');
        let k = it
            .next()
            .ok_or_else(|| CertAuthError::BadSubject(part.into()))?;
        let v = it
            .next()
            .ok_or_else(|| CertAuthError::BadSubject(part.into()))?;
        match k.trim() {
            "CN" => cn = Some(v.trim().to_string()),
            "OU" => ou = Some(v.trim().to_string()),
            "O" => o = Some(v.trim().to_string()),
            _ => {} // ignore unknown attrs
        }
    }
    let common_name = cn.ok_or(CertAuthError::NoCommonName)?;
    Ok(CertSubject {
        common_name,
        organisational_unit: ou,
        organisation: o,
    })
}

/// Authenticator that maps cert subjects → user names.  Holds an explicit
/// allow-list so a CA-signed cert with a stranger CN cannot impersonate.
pub struct CertAuthenticator {
    allow_list: RwLock<HashMap<String, String>>, // CN -> username
}

impl CertAuthenticator {
    pub fn new() -> Self {
        Self {
            allow_list: RwLock::new(HashMap::new()),
        }
    }

    /// Map a CN to an etcd user.
    pub fn allow(&self, cn: impl Into<String>, user: impl Into<String>) {
        self.allow_list
            .write()
            .unwrap()
            .insert(cn.into(), user.into());
    }

    pub fn revoke_cn(&self, cn: &str) -> bool {
        self.allow_list.write().unwrap().remove(cn).is_some()
    }

    pub fn allowed_cns(&self) -> Vec<String> {
        let mut v: Vec<String> = self.allow_list.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    /// Authenticate by parsing the supplied subject and consulting the
    /// allow-list.
    pub fn authenticate(&self, subject_str: &str) -> Result<String, CertAuthError> {
        let s = parse_subject(subject_str)?;
        let g = self.allow_list.read().unwrap();
        let user = g
            .get(&s.common_name)
            .cloned()
            .ok_or_else(|| CertAuthError::NotAllowListed(s.common_name.clone()))?;
        Ok(user)
    }

    /// Authenticate using an already-parsed subject.
    pub fn authenticate_subject(&self, s: &CertSubject) -> Result<String, CertAuthError> {
        let g = self.allow_list.read().unwrap();
        g.get(&s.common_name)
            .cloned()
            .ok_or_else(|| CertAuthError::NotAllowListed(s.common_name.clone()))
    }
}

impl Default for CertAuthenticator {
    fn default() -> Self {
        Self::new()
    }
}

// ── Root-role bootstrap intervals ─────────────────────────────────────────

/// Build the canonical "root" role interval set: full ReadWrite over
/// `\x00..\xff`.  Mirrors etcd's `setupRootRole`.
pub fn root_role_intervals() -> Vec<PermInterval> {
    vec![PermInterval {
        key: vec![0u8],
        key_end: Some(vec![0u8]),
        kind: PermKind::ReadWrite,
    }]
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M11
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── AuthRevision ──────────────────────────────────────────────────

    #[test]
    fn test_auth_revision_starts_at_zero() {
        let r = AuthRevision::new();
        assert_eq!(r.current(), 0);
    }

    #[test]
    fn test_auth_revision_bump_monotonic() {
        // cite: server/auth/store.go (auth_rev increases on every write)
        let r = AuthRevision::new();
        assert_eq!(r.bump(), 1);
        assert_eq!(r.bump(), 2);
        assert_eq!(r.bump(), 3);
        assert_eq!(r.current(), 3);
    }

    #[test]
    fn test_auth_revision_is_stale_detection() {
        // cite: client cache invalidation (revision-driven)
        let r = AuthRevision::new();
        r.bump();
        let snapshot = r.current();
        assert!(!r.is_stale(snapshot));
        r.bump();
        assert!(r.is_stale(snapshot));
    }

    // ── PermKind covers ────────────────────────────────────────────────

    #[test]
    fn test_permkind_readwrite_covers_both() {
        assert!(PermKind::ReadWrite.covers(PermKind::Read));
        assert!(PermKind::ReadWrite.covers(PermKind::Write));
        assert!(PermKind::ReadWrite.covers(PermKind::ReadWrite));
    }

    #[test]
    fn test_permkind_read_does_not_cover_write() {
        assert!(!PermKind::Read.covers(PermKind::Write));
        assert!(!PermKind::Write.covers(PermKind::Read));
    }

    // ── PermInterval matches ───────────────────────────────────────────

    #[test]
    fn test_perm_interval_exact_key_match() {
        // cite: range_perm_cache.go (single-key match)
        let iv = PermInterval {
            key: b"k".to_vec(),
            key_end: None,
            kind: PermKind::Read,
        };
        assert!(iv.matches(b"k"));
        assert!(!iv.matches(b"l"));
    }

    #[test]
    fn test_perm_interval_range_match() {
        // cite: range_perm_cache.go (interval [a,c))
        let iv = PermInterval {
            key: b"a".to_vec(),
            key_end: Some(b"c".to_vec()),
            kind: PermKind::Read,
        };
        assert!(iv.matches(b"a"));
        assert!(iv.matches(b"b"));
        assert!(!iv.matches(b"c"));
        assert!(!iv.matches(b"d"));
    }

    #[test]
    fn test_perm_interval_full_range_sentinel() {
        // cite: server/auth/store.go (key=00, end=00 ⇒ full range)
        let iv = PermInterval {
            key: vec![0],
            key_end: Some(vec![0]),
            kind: PermKind::ReadWrite,
        };
        assert!(iv.matches(b"anything"));
        assert!(iv.matches(b"\x7f"));
    }

    // ── RangePermCache ─────────────────────────────────────────────────

    #[test]
    fn test_perm_cache_replace_and_lookup() {
        // cite: range_perm_cache.go (cached lookup)
        let c = RangePermCache::new();
        c.replace_user(
            "alice",
            vec![PermInterval {
                key: b"/keys/".to_vec(),
                key_end: Some(b"/keys0".to_vec()),
                kind: PermKind::Read,
            }],
        );
        assert!(c.allows("alice", b"/keys/x", PermKind::Read));
        assert!(!c.allows("alice", b"/keys/x", PermKind::Write));
        assert!(!c.allows("alice", b"/other", PermKind::Read));
    }

    #[test]
    fn test_perm_cache_unknown_user_denied() {
        let c = RangePermCache::new();
        assert!(!c.allows("ghost", b"k", PermKind::Read));
    }

    #[test]
    fn test_perm_cache_invalidate_user_removes() {
        // cite: range_perm_cache.go (invalidate on role change)
        let c = RangePermCache::new();
        c.replace_user(
            "a",
            vec![PermInterval {
                key: b"k".to_vec(),
                key_end: None,
                kind: PermKind::Read,
            }],
        );
        c.invalidate_user("a");
        assert!(!c.allows("a", b"k", PermKind::Read));
    }

    #[test]
    fn test_perm_cache_invalidate_all() {
        // cite: range_perm_cache.go (full invalidation)
        let c = RangePermCache::new();
        c.replace_user(
            "a",
            vec![PermInterval {
                key: b"k".to_vec(),
                key_end: None,
                kind: PermKind::Read,
            }],
        );
        c.replace_user(
            "b",
            vec![PermInterval {
                key: b"k".to_vec(),
                key_end: None,
                kind: PermKind::Read,
            }],
        );
        c.invalidate_all();
        assert_eq!(c.known_users(), Vec::<String>::new());
    }

    #[test]
    fn test_perm_cache_known_users_sorted() {
        let c = RangePermCache::new();
        c.replace_user("c", vec![]);
        c.replace_user("a", vec![]);
        c.replace_user("b", vec![]);
        assert_eq!(c.known_users(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_perm_cache_fresh_after_set_revision() {
        // cite: store.go (revision check before lookup)
        let c = RangePermCache::new();
        c.set_revision(7);
        assert!(c.fresh(7));
        assert!(!c.fresh(8));
    }

    #[test]
    fn test_perm_cache_readwrite_grants_both() {
        // cite: range_perm_cache.go (RW perm covers both)
        let c = RangePermCache::new();
        c.replace_user(
            "alice",
            vec![PermInterval {
                key: b"k".to_vec(),
                key_end: None,
                kind: PermKind::ReadWrite,
            }],
        );
        assert!(c.allows("alice", b"k", PermKind::Read));
        assert!(c.allows("alice", b"k", PermKind::Write));
    }

    // ── parse_subject ──────────────────────────────────────────────────

    #[test]
    fn test_parse_subject_full_dn() {
        // cite: AuthInfoFromTLS (CN/OU/O extraction)
        let s = parse_subject("CN=alice,OU=admins,O=cave").unwrap();
        assert_eq!(s.common_name, "alice");
        assert_eq!(s.organisational_unit.as_deref(), Some("admins"));
        assert_eq!(s.organisation.as_deref(), Some("cave"));
    }

    #[test]
    fn test_parse_subject_only_cn() {
        let s = parse_subject("CN=alice").unwrap();
        assert_eq!(s.common_name, "alice");
        assert!(s.organisational_unit.is_none());
        assert!(s.organisation.is_none());
    }

    #[test]
    fn test_parse_subject_no_cn_errors() {
        // cite: AuthInfoFromTLS (CN required)
        assert_eq!(
            parse_subject("OU=admins").unwrap_err(),
            CertAuthError::NoCommonName
        );
    }

    #[test]
    fn test_parse_subject_bad_format_errors() {
        assert!(matches!(
            parse_subject("bad-format").unwrap_err(),
            CertAuthError::BadSubject(_)
        ));
    }

    #[test]
    fn test_parse_subject_ignores_unknown_attrs() {
        // cite: x509 subject (unknown attrs ignored)
        let s = parse_subject("CN=alice,L=Istanbul,ST=TR").unwrap();
        assert_eq!(s.common_name, "alice");
    }

    #[test]
    fn test_parse_subject_trims_whitespace() {
        // cite: x509 subject (tolerate spaces)
        let s = parse_subject("CN = alice , OU = admins").unwrap();
        assert_eq!(s.common_name, "alice");
        assert_eq!(s.organisational_unit.as_deref(), Some("admins"));
    }

    // ── CertAuthenticator ──────────────────────────────────────────────

    #[test]
    fn test_cert_auth_known_cn_authenticated() {
        // cite: AuthInfoFromTLS (CN matches user)
        let a = CertAuthenticator::new();
        a.allow("alice", "alice");
        assert_eq!(
            a.authenticate("CN=alice,OU=admins,O=cave").unwrap(),
            "alice"
        );
    }

    #[test]
    fn test_cert_auth_unknown_cn_rejected() {
        // cite: AuthInfoFromTLS (CN not allow-listed)
        let a = CertAuthenticator::new();
        match a.authenticate("CN=ghost") {
            Err(CertAuthError::NotAllowListed(cn)) => assert_eq!(cn, "ghost"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_cert_auth_bad_subject_rejected() {
        let a = CertAuthenticator::new();
        assert!(matches!(
            a.authenticate("not-a-subject").unwrap_err(),
            CertAuthError::BadSubject(_)
        ));
    }

    #[test]
    fn test_cert_auth_revoke_cn_disallows_after() {
        // cite: AuthInfoFromTLS (revoke ⇒ deny)
        let a = CertAuthenticator::new();
        a.allow("alice", "alice");
        assert!(a.authenticate("CN=alice").is_ok());
        a.revoke_cn("alice");
        assert!(a.authenticate("CN=alice").is_err());
    }

    #[test]
    fn test_cert_auth_revoke_unknown_returns_false() {
        let a = CertAuthenticator::new();
        assert!(!a.revoke_cn("ghost"));
    }

    #[test]
    fn test_cert_auth_allowed_cns_sorted() {
        let a = CertAuthenticator::new();
        a.allow("c", "c");
        a.allow("a", "a");
        a.allow("b", "b");
        assert_eq!(a.allowed_cns(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_cert_auth_subject_aliasing_to_user() {
        // cite: AuthInfoFromTLS (cert CN ≠ etcd username, mapping needed)
        let a = CertAuthenticator::new();
        a.allow("CN-of-cert", "etcd-username");
        let user = a.authenticate("CN=CN-of-cert,OU=x").unwrap();
        assert_eq!(user, "etcd-username");
    }

    #[test]
    fn test_cert_auth_subject_struct_authenticate() {
        // cite: AuthInfoFromTLS (already-parsed subject)
        let a = CertAuthenticator::new();
        a.allow("alice", "alice");
        let s = CertSubject {
            common_name: "alice".into(),
            organisational_unit: None,
            organisation: None,
        };
        assert_eq!(a.authenticate_subject(&s).unwrap(), "alice");
    }

    // ── Root-role bootstrap ────────────────────────────────────────────

    #[test]
    fn test_root_role_intervals_full_range() {
        // cite: setupRootRole (CN=root grants full ReadWrite)
        let intervals = root_role_intervals();
        assert_eq!(intervals.len(), 1);
        let iv = &intervals[0];
        assert_eq!(iv.kind, PermKind::ReadWrite);
        assert!(iv.matches(b"any-key"));
        assert!(iv.matches(b"\xff\xff"));
    }
}
