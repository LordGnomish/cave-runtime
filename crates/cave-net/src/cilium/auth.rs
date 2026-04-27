//! Cilium Auth API — per-identity-pair mTLS authentication state.
//!
//! Mirrors `pkg/auth/auth.go` (the AuthManager that drives mutual auth
//! between two Cilium identities), `pkg/auth/spire/spire.go` (SPIFFE/
//! SPIRE-backed credential resolution), and the BPF map shape from
//! `bpf/lib/auth.h::auth_map`.
//!
//! Semantics (faithful to upstream):
//!
//! * A policy `Authentication.Mode` per L4 rule selects how the source/
//!   destination identity pair must authenticate before traffic is
//!   forwarded:
//!   - [`AuthMode::Disabled`] — no auth required (default).
//!   - [`AuthMode::Required`] — mutual auth needed; if the auth map
//!     has no entry the dataplane redirects to the user-space auth
//!     agent, which performs the handshake and installs the entry.
//!   - [`AuthMode::AlwaysFail`] — used in tests; deny always.
//!   - [`AuthMode::Spire`] — SPIFFE-backed; identity pair must have
//!     a fresh SVID exchange with a TTL-bounded entry in the map.
//!
//! * Auth entries are keyed by `(src_identity, dst_identity, mode)` and
//!   carry an absolute expiration timestamp. After expiry the dataplane
//!   re-redirects to the agent.
//!
//! * The agent handshake produces a shared SPIFFE id pair plus a TTL.
//!   The TTL is the **min** of the two endpoints' SVID lifetimes
//!   (mirrors `pkg/auth/spire.go::renewSVID`).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthMode {
    Disabled,
    Required,
    AlwaysFail,
    Spire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthKey {
    pub src_identity: u32,
    pub dst_identity: u32,
    pub mode: AuthMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthEntry {
    pub src_spiffe: String,
    pub dst_spiffe: String,
    pub created: u64,
    pub expires_at: u64,
}

impl AuthEntry {
    pub fn is_valid(&self, now: u64) -> bool {
        now < self.expires_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthVerdict {
    /// Auth not required for this rule.
    NotRequired,
    /// Auth entry present and not expired.
    Authorized,
    /// Auth entry missing — dataplane should redirect to the agent.
    NeedsAuth,
    /// Mode is `AlwaysFail` — explicit deny.
    Denied,
    /// Auth entry present but expired — dataplane should redirect.
    Expired,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("SPIFFE id `{0}` is not in trust domain `{1}`")]
    UntrustedSpiffe(String, String),
    #[error("SVID for `{0}` not found in cache")]
    SvidNotFound(String),
    #[error("agent handshake failed: {0}")]
    HandshakeFailed(String),
    #[error("tenant {tenant} cannot mutate auth map owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// SPIFFE workload identity document (mirrors `pkg/spire/svid.go`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Svid {
    pub spiffe_id: String,
    pub trust_domain: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl Svid {
    pub fn new(spiffe_id: impl Into<String>, trust_domain: impl Into<String>, issued_at: u64, ttl: u64) -> Self {
        let id = spiffe_id.into();
        Self {
            spiffe_id: id, trust_domain: trust_domain.into(), issued_at,
            expires_at: issued_at + ttl,
        }
    }
    pub fn ttl_remaining(&self, now: u64) -> u64 {
        self.expires_at.saturating_sub(now)
    }
    pub fn is_valid(&self, now: u64) -> bool {
        now < self.expires_at && now >= self.issued_at
    }
    pub fn validate_trust_domain(&self, expected_td: &str) -> Result<(), AuthError> {
        if self.trust_domain != expected_td {
            return Err(AuthError::UntrustedSpiffe(self.spiffe_id.clone(), expected_td.to_string()));
        }
        if !self.spiffe_id.starts_with(&format!("spiffe://{}/", expected_td)) {
            return Err(AuthError::UntrustedSpiffe(self.spiffe_id.clone(), expected_td.to_string()));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct AuthManager {
    pub tenant: TenantId,
    pub trust_domain: String,
    /// Auth map (mirrors `bpf/lib/auth.h::auth_map`).
    auth: HashMap<AuthKey, AuthEntry>,
    /// SVID cache per identity (mirrors `pkg/auth/spire/cache.go`).
    svids: HashMap<u32, Svid>,
}

impl AuthManager {
    pub fn new(tenant: TenantId, trust_domain: impl Into<String>) -> Self {
        Self { tenant, trust_domain: trust_domain.into(), auth: HashMap::new(), svids: HashMap::new() }
    }

    pub fn register_svid(&mut self, identity: u32, svid: Svid) -> Result<(), AuthError> {
        svid.validate_trust_domain(&self.trust_domain)?;
        self.svids.insert(identity, svid);
        Ok(())
    }

    pub fn lookup_svid(&self, identity: u32) -> Option<&Svid> {
        self.svids.get(&identity)
    }

    pub fn revoke_svid(&mut self, identity: u32) -> bool {
        let removed = self.svids.remove(&identity).is_some();
        // Drop any auth entries that referenced this identity.
        if removed {
            self.auth.retain(|k, _| k.src_identity != identity && k.dst_identity != identity);
        }
        removed
    }

    /// Resolve an auth verdict for a (src, dst, mode) triple at time `now`.
    /// Mirrors `bpf/lib/auth.h::auth_lookup`.
    pub fn resolve(&self, src: u32, dst: u32, mode: AuthMode, now: u64) -> AuthVerdict {
        match mode {
            AuthMode::Disabled => AuthVerdict::NotRequired,
            AuthMode::AlwaysFail => AuthVerdict::Denied,
            AuthMode::Required | AuthMode::Spire => {
                let key = AuthKey { src_identity: src, dst_identity: dst, mode };
                match self.auth.get(&key) {
                    None => AuthVerdict::NeedsAuth,
                    Some(e) if !e.is_valid(now) => AuthVerdict::Expired,
                    Some(_) => AuthVerdict::Authorized,
                }
            }
        }
    }

    /// Perform the agent-side handshake — given two registered SVIDs,
    /// install an auth entry whose TTL is the **min** of the two SVIDs'
    /// remaining lifetimes. Mirrors `pkg/auth/manager.go::Authenticate`.
    pub fn handshake(
        &mut self,
        src: u32,
        dst: u32,
        mode: AuthMode,
        now: u64,
    ) -> Result<AuthEntry, AuthError> {
        if matches!(mode, AuthMode::Disabled) {
            return Err(AuthError::HandshakeFailed("mode is Disabled".into()));
        }
        if matches!(mode, AuthMode::AlwaysFail) {
            return Err(AuthError::HandshakeFailed("mode is AlwaysFail".into()));
        }
        let src_svid = self.svids.get(&src).cloned()
            .ok_or_else(|| AuthError::SvidNotFound(format!("identity {src}")))?;
        let dst_svid = self.svids.get(&dst).cloned()
            .ok_or_else(|| AuthError::SvidNotFound(format!("identity {dst}")))?;
        if !src_svid.is_valid(now) {
            return Err(AuthError::HandshakeFailed(format!("src SVID {src} expired")));
        }
        if !dst_svid.is_valid(now) {
            return Err(AuthError::HandshakeFailed(format!("dst SVID {dst} expired")));
        }
        let ttl = src_svid.ttl_remaining(now).min(dst_svid.ttl_remaining(now));
        let entry = AuthEntry {
            src_spiffe: src_svid.spiffe_id,
            dst_spiffe: dst_svid.spiffe_id,
            created: now,
            expires_at: now + ttl,
        };
        let key = AuthKey { src_identity: src, dst_identity: dst, mode };
        self.auth.insert(key, entry.clone());
        Ok(entry)
    }

    /// Reap expired auth entries. Returns the count of removed entries.
    pub fn gc_expired(&mut self, now: u64) -> usize {
        let before = self.auth.len();
        self.auth.retain(|_, e| e.is_valid(now));
        before - self.auth.len()
    }

    pub fn auth_count(&self) -> usize {
        self.auth.len()
    }

    pub fn svid_count(&self) -> usize {
        self.svids.len()
    }

    /// Force-invalidate every auth entry (used when policy changes
    /// require re-handshake). Mirrors `pkg/auth/manager.go::PurgeAll`.
    pub fn purge_all(&mut self) -> usize {
        let n = self.auth.len();
        self.auth.clear();
        n
    }

    /// Renew an existing auth entry — returns the renewed entry on
    /// success. Useful when the agent receives a fresh SVID push.
    pub fn renew(&mut self, src: u32, dst: u32, mode: AuthMode, now: u64) -> Result<AuthEntry, AuthError> {
        // Re-handshake replaces the existing entry.
        self.handshake(src, dst, mode, now)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/auth/auth.go", "AuthManager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn mgr(tenant: TenantId) -> AuthManager {
        AuthManager::new(tenant, "cluster.local")
    }

    fn svid(workload: &str, ttl: u64, now: u64) -> Svid {
        Svid::new(format!("spiffe://cluster.local/{workload}"), "cluster.local", now, ttl)
    }

    // ── AuthMode resolution ──────────────────────────────────────────────────

    #[test]
    fn auth_mode_disabled_resolves_to_not_required() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Resolve.Disabled", "tenant-auth-dis");
        let m = mgr(tenant);
        assert_eq!(m.resolve(256, 257, AuthMode::Disabled, 100), AuthVerdict::NotRequired);
    }

    #[test]
    fn auth_mode_always_fail_resolves_to_denied() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Resolve.AlwaysFail", "tenant-auth-fail");
        let m = mgr(tenant);
        assert_eq!(m.resolve(256, 257, AuthMode::AlwaysFail, 100), AuthVerdict::Denied);
    }

    #[test]
    fn auth_mode_required_no_entry_returns_needs_auth() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Resolve.Required.NoEntry", "tenant-auth-need");
        let m = mgr(tenant);
        assert_eq!(m.resolve(256, 257, AuthMode::Required, 100), AuthVerdict::NeedsAuth);
    }

    #[test]
    fn auth_mode_spire_no_entry_returns_needs_auth() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Resolve.Spire.NoEntry", "tenant-auth-spneed");
        let m = mgr(tenant);
        assert_eq!(m.resolve(256, 257, AuthMode::Spire, 100), AuthVerdict::NeedsAuth);
    }

    // ── SVID register + validation ───────────────────────────────────────────

    #[test]
    fn svid_register_valid_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/spire/cache.go", "RegisterSVID", "tenant-auth-reg");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("workload-a", 3600, 100)).unwrap();
        assert_eq!(m.svid_count(), 1);
    }

    #[test]
    fn svid_register_wrong_trust_domain_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/spire/cache.go", "RegisterSVID.TrustDomain", "tenant-auth-td");
        let mut m = mgr(tenant);
        let bad = Svid::new("spiffe://other.example/workload", "other.example", 100, 3600);
        let err = m.register_svid(256, bad).unwrap_err();
        assert!(matches!(err, AuthError::UntrustedSpiffe(_, _)));
    }

    #[test]
    fn svid_register_id_not_in_trust_domain_path_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/spire/cache.go", "RegisterSVID.IDPath", "tenant-auth-idp");
        let mut m = mgr(tenant);
        // trust_domain field is right but id doesn't have proper prefix.
        let bad = Svid {
            spiffe_id: "spiffe://other.example/workload".into(),
            trust_domain: "cluster.local".into(),
            issued_at: 100, expires_at: 3700,
        };
        let err = m.register_svid(256, bad).unwrap_err();
        assert!(matches!(err, AuthError::UntrustedSpiffe(_, _)));
    }

    #[test]
    fn svid_lookup_returns_registered() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/spire/cache.go", "LookupSVID", "tenant-auth-lk");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("workload-a", 3600, 100)).unwrap();
        let s = m.lookup_svid(256).unwrap();
        assert_eq!(s.spiffe_id, "spiffe://cluster.local/workload-a");
    }

    #[test]
    fn svid_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/spire/cache.go", "LookupSVID.NotFound", "tenant-auth-lknf");
        let m = mgr(tenant);
        assert!(m.lookup_svid(999).is_none());
    }

    #[test]
    fn svid_revoke_drops_attached_auth_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/spire/cache.go", "RevokeSVID.CascadeAuth", "tenant-auth-rev");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        assert_eq!(m.auth_count(), 1);
        assert!(m.revoke_svid(256));
        assert_eq!(m.auth_count(), 0);
    }

    #[test]
    fn svid_revoke_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/spire/cache.go", "RevokeSVID.NotFound", "tenant-auth-revnf");
        let mut m = mgr(tenant);
        assert!(!m.revoke_svid(999));
    }

    #[test]
    fn svid_validity_window() {
        let (_c, _t) = cilium_test_ctx!("pkg/auth/spire/cache.go", "Svid.IsValid", "tenant-auth-val");
        let s = svid("w", 1000, 100);
        assert!(s.is_valid(500));
        assert!(!s.is_valid(50));
        assert!(!s.is_valid(2000));
    }

    #[test]
    fn svid_ttl_remaining_decays_with_now() {
        let (_c, _t) = cilium_test_ctx!("pkg/auth/spire/cache.go", "Svid.TtlRemaining", "tenant-auth-ttl");
        let s = svid("w", 1000, 100);
        assert_eq!(s.ttl_remaining(100), 1000);
        assert_eq!(s.ttl_remaining(600), 500);
        assert_eq!(s.ttl_remaining(2000), 0);
    }

    // ── Handshake ────────────────────────────────────────────────────────────

    #[test]
    fn handshake_creates_authorized_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate", "tenant-auth-hs");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        let entry = m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        assert_eq!(entry.src_spiffe, "spiffe://cluster.local/a");
        assert_eq!(entry.dst_spiffe, "spiffe://cluster.local/b");
        assert_eq!(m.resolve(256, 257, AuthMode::Required, 100), AuthVerdict::Authorized);
    }

    #[test]
    fn handshake_uses_min_of_src_and_dst_ttl() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.MinTTL", "tenant-auth-min");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 1000, 100)).unwrap(); // expires 1100
        m.register_svid(257, svid("b", 200, 100)).unwrap(); // expires 300
        let entry = m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        // min remaining at t=100: src=1000, dst=200 → 200.
        assert_eq!(entry.expires_at, 300);
    }

    #[test]
    fn handshake_rejects_disabled_mode() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.Disabled", "tenant-auth-hsdis");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        assert!(m.handshake(256, 257, AuthMode::Disabled, 100).is_err());
    }

    #[test]
    fn handshake_rejects_always_fail_mode() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.AlwaysFail", "tenant-auth-hsaf");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        assert!(m.handshake(256, 257, AuthMode::AlwaysFail, 100).is_err());
    }

    #[test]
    fn handshake_missing_src_svid_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.SvidMissing", "tenant-auth-hsmiss");
        let mut m = mgr(tenant);
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        let err = m.handshake(256, 257, AuthMode::Required, 100).unwrap_err();
        assert!(matches!(err, AuthError::SvidNotFound(_)));
    }

    #[test]
    fn handshake_missing_dst_svid_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.DstMissing", "tenant-auth-hsdst");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        let err = m.handshake(256, 257, AuthMode::Required, 100).unwrap_err();
        assert!(matches!(err, AuthError::SvidNotFound(_)));
    }

    #[test]
    fn handshake_with_expired_src_svid_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.SrcExpired", "tenant-auth-hsexp");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 100, 100)).unwrap(); // expires 200
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        let err = m.handshake(256, 257, AuthMode::Required, 500).unwrap_err();
        assert!(matches!(err, AuthError::HandshakeFailed(_)));
    }

    #[test]
    fn handshake_with_expired_dst_svid_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.DstExpired", "tenant-auth-hsdexp");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 50, 100)).unwrap();
        let err = m.handshake(256, 257, AuthMode::Required, 200).unwrap_err();
        assert!(matches!(err, AuthError::HandshakeFailed(_)));
    }

    #[test]
    fn handshake_spire_mode_creates_separate_entry_from_required() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Authenticate.ModeKey", "tenant-auth-hsmode");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        m.handshake(256, 257, AuthMode::Spire, 100).unwrap();
        assert_eq!(m.auth_count(), 2);
    }

    // ── Expiry / GC ──────────────────────────────────────────────────────────

    #[test]
    fn auth_entry_expires_after_ttl() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "AuthEntry.Expire", "tenant-auth-aexp");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 200, 100)).unwrap();
        m.register_svid(257, svid("b", 200, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        assert_eq!(m.resolve(256, 257, AuthMode::Required, 250), AuthVerdict::Authorized);
        assert_eq!(m.resolve(256, 257, AuthMode::Required, 350), AuthVerdict::Expired);
    }

    #[test]
    fn auth_gc_expired_drops_stale_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "GC", "tenant-auth-gc");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 100, 100)).unwrap();
        m.register_svid(257, svid("b", 100, 100)).unwrap();
        m.register_svid(258, svid("c", 1000, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap(); // expires 200
        m.handshake(256, 258, AuthMode::Required, 100).unwrap(); // expires 200 (min of 100,1000 = 100)
        let n = m.gc_expired(300);
        assert_eq!(n, 2);
        assert_eq!(m.auth_count(), 0);
    }

    #[test]
    fn auth_gc_keeps_valid_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "GC.KeepValid", "tenant-auth-gck");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 1000, 100)).unwrap();
        m.register_svid(257, svid("b", 1000, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        let n = m.gc_expired(500);
        assert_eq!(n, 0);
        assert_eq!(m.auth_count(), 1);
    }

    #[test]
    fn auth_purge_all_clears_map() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "PurgeAll", "tenant-auth-purge");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        assert_eq!(m.purge_all(), 1);
        assert_eq!(m.auth_count(), 0);
    }

    #[test]
    fn auth_renew_replaces_entry_with_fresh_expiry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/manager.go", "Renew", "tenant-auth-renew");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        let first = m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        // Replace SVIDs with fresher ones.
        m.register_svid(256, svid("a", 3600, 1000)).unwrap();
        m.register_svid(257, svid("b", 3600, 1000)).unwrap();
        let second = m.renew(256, 257, AuthMode::Required, 1000).unwrap();
        assert!(second.expires_at > first.expires_at);
    }

    // ── Verdict edge cases ───────────────────────────────────────────────────

    #[test]
    fn auth_resolve_after_handshake_authorized_within_window() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Resolve.Authorized", "tenant-auth-rauth");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 1000, 100)).unwrap();
        m.register_svid(257, svid("b", 1000, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        assert_eq!(m.resolve(256, 257, AuthMode::Required, 500), AuthVerdict::Authorized);
    }

    #[test]
    fn auth_resolve_only_matches_exact_src_dst_pair() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Resolve.Asymmetric", "tenant-auth-rsym");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        // Reverse direction must NOT be authorized.
        assert_eq!(m.resolve(257, 256, AuthMode::Required, 200), AuthVerdict::NeedsAuth);
    }

    #[test]
    fn auth_resolve_mode_specific_required_does_not_satisfy_spire() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Resolve.ModeKey", "tenant-auth-rmodek");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        assert_eq!(m.resolve(256, 257, AuthMode::Spire, 200), AuthVerdict::NeedsAuth);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn auth_mode_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/auth/auth.go", "AuthMode.Serde", "tenant-auth-mserde");
        for m in [AuthMode::Disabled, AuthMode::Required, AuthMode::AlwaysFail, AuthMode::Spire] {
            let s = serde_json::to_string(&m).unwrap();
            let back: AuthMode = serde_json::from_str(&s).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn auth_key_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/auth.h", "AuthKey.Serde", "tenant-auth-kserde");
        let k = AuthKey { src_identity: 256, dst_identity: 257, mode: AuthMode::Required };
        let s = serde_json::to_string(&k).unwrap();
        let back: AuthKey = serde_json::from_str(&s).unwrap();
        assert_eq!(back, k);
    }

    #[test]
    fn auth_entry_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/auth/auth.go", "AuthEntry.Serde", "tenant-auth-eserde");
        let e = AuthEntry {
            src_spiffe: "spiffe://cluster.local/a".into(),
            dst_spiffe: "spiffe://cluster.local/b".into(),
            created: 100, expires_at: 3700,
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: AuthEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn auth_svid_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/auth/spire/cache.go", "Svid.Serde", "tenant-auth-svidserde");
        let s = svid("w", 3600, 100);
        let json = serde_json::to_string(&s).unwrap();
        let back: Svid = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn auth_verdict_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/auth/auth.go", "AuthVerdict.Serde", "tenant-auth-vserde");
        for v in [
            AuthVerdict::NotRequired,
            AuthVerdict::Authorized,
            AuthVerdict::NeedsAuth,
            AuthVerdict::Denied,
            AuthVerdict::Expired,
        ] {
            let s = serde_json::to_string(&v).unwrap();
            let back: AuthVerdict = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }

    // ── Multi-pair scenarios ─────────────────────────────────────────────────

    #[test]
    fn auth_multiple_pairs_independent() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Multi.Independent", "tenant-auth-multi");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        m.register_svid(258, svid("c", 3600, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        m.handshake(258, 257, AuthMode::Required, 100).unwrap();
        assert_eq!(m.auth_count(), 2);
        assert_eq!(m.resolve(256, 257, AuthMode::Required, 200), AuthVerdict::Authorized);
        assert_eq!(m.resolve(258, 257, AuthMode::Required, 200), AuthVerdict::Authorized);
        // Cross pairs not authorized.
        assert_eq!(m.resolve(256, 258, AuthMode::Required, 200), AuthVerdict::NeedsAuth);
    }

    #[test]
    fn auth_count_tracks_handshakes() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "AuthCount", "tenant-auth-cnt");
        let mut m = mgr(tenant);
        for i in 0..5u32 {
            m.register_svid(256 + i, svid(&format!("w{i}"), 3600, 100)).unwrap();
        }
        for i in 1..5u32 {
            m.handshake(256, 256 + i, AuthMode::Required, 100).unwrap();
        }
        assert_eq!(m.auth_count(), 4);
    }

    #[test]
    fn auth_renew_existing_pair_overwrites() {
        let (_c, tenant) = cilium_test_ctx!("pkg/auth/auth.go", "Renew.Overwrite", "tenant-auth-rwo");
        let mut m = mgr(tenant);
        m.register_svid(256, svid("a", 3600, 100)).unwrap();
        m.register_svid(257, svid("b", 3600, 100)).unwrap();
        m.handshake(256, 257, AuthMode::Required, 100).unwrap();
        m.handshake(256, 257, AuthMode::Required, 200).unwrap();
        assert_eq!(m.auth_count(), 1);
    }

    // ── SPIFFE id structure ──────────────────────────────────────────────────

    #[test]
    fn svid_spiffe_id_must_have_workload_path() {
        let (_c, tenant) = cilium_test_ctx!("pkg/spire/svid.go", "Svid.PathRequired", "tenant-auth-spath");
        let mut m = mgr(tenant);
        // Apex without trailing path must be rejected.
        let bad = Svid {
            spiffe_id: "spiffe://cluster.local".into(),
            trust_domain: "cluster.local".into(),
            issued_at: 100, expires_at: 3700,
        };
        let err = m.register_svid(256, bad).unwrap_err();
        assert!(matches!(err, AuthError::UntrustedSpiffe(_, _)));
    }

    #[test]
    fn svid_validate_trust_domain_succeeds_for_correct_td() {
        let (_c, _t) = cilium_test_ctx!("pkg/spire/svid.go", "Svid.ValidateTD", "tenant-auth-tdok");
        let s = svid("w", 3600, 100);
        s.validate_trust_domain("cluster.local").unwrap();
    }

    #[test]
    fn svid_validate_trust_domain_fails_for_wrong_td() {
        let (_c, _t) = cilium_test_ctx!("pkg/spire/svid.go", "Svid.ValidateTD.Mismatch", "tenant-auth-tdbad");
        let s = svid("w", 3600, 100);
        assert!(s.validate_trust_domain("other.example").is_err());
    }

    // ── AuthEntry validity helper ────────────────────────────────────────────

    #[test]
    fn auth_entry_is_valid_within_window() {
        let (_c, _t) = cilium_test_ctx!("pkg/auth/auth.go", "AuthEntry.IsValid", "tenant-auth-eval");
        let e = AuthEntry {
            src_spiffe: "x".into(), dst_spiffe: "y".into(),
            created: 100, expires_at: 200,
        };
        assert!(e.is_valid(150));
        assert!(!e.is_valid(200));
        assert!(!e.is_valid(300));
    }
}
