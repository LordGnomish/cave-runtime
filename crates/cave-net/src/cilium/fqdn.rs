// SPDX-License-Identifier: AGPL-3.0-or-later
//! FQDN-based identity — Cilium's `toFQDNs` policy support.
//!
//! Mirrors `pkg/fqdn/fqdn.go` plus the FQDN identity-allocation logic
//! in `pkg/fqdn/dnsproxy/dnsproxy.go`. When a CNP `toFQDNs.matchPattern`
//! rule is in effect, cilium-agent intercepts DNS replies, resolves the
//! pattern to one or more IPs, and assigns each (pattern, ip) pair an
//! identity in the FQDN range so the policy can then evaluate L4 against
//! that identity.
//!
//! Semantics (faithful to upstream):
//!
//! * FQDN identities live in the range `[FQDN_MIN, FQDN_MAX)` =
//!   `[16777216, 16777216 + 2^24)` — the upper 24-bit identity space.
//! * Match patterns use the upstream `MatchPattern` glob: `*` matches a
//!   single label, `*.example.com` matches any subdomain, exact strings
//!   match exactly. (Same syntax as `cilium::l7policy::DnsRule`.)
//! * Each `(pattern, ip)` mapping has a TTL inherited from the DNS
//!   reply; expired entries are GC'd via [`FqdnCache::gc`].

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

/// Lowest FQDN-allocated identity. Mirrors upstream
/// `pkg/identity/numericidentity.go::IdentityScopeRemoteNode + 0x01_00_00_00`.
pub const FQDN_IDENTITY_MIN: u32 = 16_777_216;
pub const FQDN_IDENTITY_MAX: u32 = FQDN_IDENTITY_MIN + (1 << 24);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FqdnEntry {
    pub identity: u32,
    pub pattern: String,
    pub fqdn: String,
    pub ip: IpAddr,
    pub ttl_seconds: u64,
    pub created: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FqdnError {
    #[error("FQDN identity space exhausted ({0} identities used)")]
    Exhausted(u32),
    #[error("tenant {tenant} cannot mutate FQDN cache owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct FqdnCache {
    pub tenant: TenantId,
    next_id: u32,
    by_id: HashMap<u32, FqdnEntry>,
    /// `(pattern, ip)` → identity for idempotent lookup.
    by_pattern_ip: HashMap<(String, IpAddr), u32>,
    /// Reverse: ip → identity (datapath ipcache lookup).
    by_ip: HashMap<IpAddr, u32>,
}

impl FqdnCache {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            next_id: FQDN_IDENTITY_MIN,
            by_id: HashMap::new(),
            by_pattern_ip: HashMap::new(),
            by_ip: HashMap::new(),
        }
    }

    /// Resolve / cache a DNS reply. Returns the identity allocated to the
    /// `(pattern, ip)` pair. Idempotent: same pair returns the same id.
    pub fn resolve(
        &mut self,
        pattern: impl Into<String>,
        fqdn: impl Into<String>,
        ip: IpAddr,
        ttl_seconds: u64,
        now: u64,
    ) -> Result<u32, FqdnError> {
        let pattern = pattern.into();
        let key = (pattern.clone(), ip);
        if let Some(&id) = self.by_pattern_ip.get(&key) {
            // Refresh TTL.
            if let Some(entry) = self.by_id.get_mut(&id) {
                entry.created = now;
                entry.ttl_seconds = ttl_seconds;
            }
            return Ok(id);
        }
        if self.next_id >= FQDN_IDENTITY_MAX {
            return Err(FqdnError::Exhausted(self.next_id - FQDN_IDENTITY_MIN));
        }
        let id = self.next_id;
        self.next_id += 1;
        let entry = FqdnEntry { identity: id, pattern, fqdn: fqdn.into(), ip, ttl_seconds, created: now };
        self.by_id.insert(id, entry);
        self.by_pattern_ip.insert(key, id);
        self.by_ip.insert(ip, id);
        Ok(id)
    }

    pub fn lookup_by_id(&self, id: u32) -> Option<&FqdnEntry> {
        self.by_id.get(&id)
    }

    pub fn lookup_by_ip(&self, ip: IpAddr) -> Option<&FqdnEntry> {
        let id = self.by_ip.get(&ip)?;
        self.by_id.get(id)
    }

    /// Remove an entry by identity.
    pub fn release(&mut self, id: u32) -> bool {
        if let Some(entry) = self.by_id.remove(&id) {
            self.by_pattern_ip.remove(&(entry.pattern.clone(), entry.ip));
            self.by_ip.remove(&entry.ip);
            true
        } else {
            false
        }
    }

    /// Garbage-collect entries whose TTL expired before `now`. Returns count.
    pub fn gc(&mut self, now: u64) -> usize {
        let expired: Vec<u32> = self.by_id.iter()
            .filter(|(_, e)| now.saturating_sub(e.created) >= e.ttl_seconds)
            .map(|(id, _)| *id)
            .collect();
        let n = expired.len();
        for id in expired {
            self.release(id);
        }
        n
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// MatchPattern glob test (mirrors `pkg/fqdn/matchpattern/matchpattern.go`).
/// `*` matches a single DNS label, `*.example.com` matches any direct
/// subdomain (and `example.com` itself? No — upstream does NOT match the
/// bare apex). Exact string matches exactly.
pub fn match_pattern(pattern: &str, name: &str) -> bool {
    if pattern == name {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        // Must be a strict subdomain (at least one label before the suffix).
        if let Some(stripped) = name.strip_suffix(suffix) {
            // Allow the dot-separated boundary.
            if let Some(prefix) = stripped.strip_suffix('.') {
                return !prefix.is_empty() && !prefix.contains('.');
            }
        }
        return false;
    }
    if pattern == "*" {
        // Single label.
        return !name.is_empty() && !name.contains('.');
    }
    false
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/fqdn/fqdn.go", "FQDNCache");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    // ── Resolve / lookup ─────────────────────────────────────────────────────

    #[test]
    fn fqdn_resolve_assigns_identity_in_fqdn_range() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Resolve", "tenant-fqdn-rng");
        let mut c = FqdnCache::new(tenant);
        let id = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 4), 300, 100).unwrap();
        assert_eq!(id, FQDN_IDENTITY_MIN);
        assert!(id >= FQDN_IDENTITY_MIN && id < FQDN_IDENTITY_MAX);
    }

    #[test]
    fn fqdn_resolve_idempotent_for_same_pattern_and_ip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Resolve.Idempotent", "tenant-fqdn-idem");
        let mut c = FqdnCache::new(tenant);
        let a = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 4), 300, 100).unwrap();
        let b = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 4), 300, 200).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn fqdn_distinct_ips_get_distinct_identities() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Resolve.Distinct", "tenant-fqdn-dist");
        let mut c = FqdnCache::new(tenant);
        let a = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 4), 300, 100).unwrap();
        let b = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 5), 300, 100).unwrap();
        assert_ne!(a, b);
        assert_eq!(b, a + 1);
    }

    #[test]
    fn fqdn_lookup_by_ip_returns_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.LookupIP", "tenant-fqdn-lkip");
        let mut c = FqdnCache::new(tenant);
        let _ = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 4), 300, 100).unwrap();
        let entry = c.lookup_by_ip(ip(1, 2, 3, 4)).unwrap();
        assert_eq!(entry.fqdn, "api.example.com");
    }

    #[test]
    fn fqdn_lookup_by_id_returns_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.LookupID", "tenant-fqdn-lkid");
        let mut c = FqdnCache::new(tenant);
        let id = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 4), 300, 100).unwrap();
        let entry = c.lookup_by_id(id).unwrap();
        assert_eq!(entry.ip, ip(1, 2, 3, 4));
    }

    #[test]
    fn fqdn_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Lookup.NotFound", "tenant-fqdn-nf");
        let c = FqdnCache::new(tenant);
        assert!(c.lookup_by_ip(ip(8, 8, 8, 8)).is_none());
        assert!(c.lookup_by_id(99_999).is_none());
    }

    #[test]
    fn fqdn_release_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Release", "tenant-fqdn-rel");
        let mut c = FqdnCache::new(tenant);
        let id = c.resolve("*.example.com", "api.example.com", ip(1, 2, 3, 4), 300, 100).unwrap();
        assert!(c.release(id));
        assert!(c.lookup_by_id(id).is_none());
        assert!(c.lookup_by_ip(ip(1, 2, 3, 4)).is_none());
    }

    #[test]
    fn fqdn_release_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Release.NotFound", "tenant-fqdn-relnf");
        let mut c = FqdnCache::new(tenant);
        assert!(!c.release(99_999));
    }

    // ── TTL / GC ─────────────────────────────────────────────────────────────

    #[test]
    fn fqdn_gc_removes_expired_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.GC", "tenant-fqdn-gc");
        let mut c = FqdnCache::new(tenant);
        c.resolve("*.example.com", "a.example.com", ip(1, 2, 3, 4), 60, 100).unwrap();
        c.resolve("*.example.com", "b.example.com", ip(1, 2, 3, 5), 600, 100).unwrap();
        let n = c.gc(200);
        assert_eq!(n, 1);
        assert!(c.lookup_by_ip(ip(1, 2, 3, 4)).is_none());
        assert!(c.lookup_by_ip(ip(1, 2, 3, 5)).is_some());
    }

    #[test]
    fn fqdn_resolve_refreshes_ttl_on_hit() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Resolve.RefreshTTL", "tenant-fqdn-ttl");
        let mut c = FqdnCache::new(tenant);
        let id = c.resolve("*.example.com", "a.example.com", ip(1, 2, 3, 4), 60, 100).unwrap();
        // Re-resolve at t=200 with TTL=600 → entry shouldn't be GC'd at t=300.
        c.resolve("*.example.com", "a.example.com", ip(1, 2, 3, 4), 600, 200).unwrap();
        let n = c.gc(300);
        assert_eq!(n, 0);
        assert!(c.lookup_by_id(id).is_some());
    }

    #[test]
    fn fqdn_len_tracks_active_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNCache.Len", "tenant-fqdn-len");
        let mut c = FqdnCache::new(tenant);
        for i in 0..5u8 {
            c.resolve("*.example.com", format!("h{i}.example.com"), ip(10, 0, 0, i + 1), 300, 100).unwrap();
        }
        assert_eq!(c.len(), 5);
    }

    // ── MatchPattern ─────────────────────────────────────────────────────────

    #[test]
    fn fqdn_match_pattern_exact() {
        let (_c, _t) = cilium_test_ctx!("pkg/fqdn/matchpattern/matchpattern.go", "Validate", "tenant-fqdn-mpex");
        assert!(match_pattern("api.example.com", "api.example.com"));
        assert!(!match_pattern("api.example.com", "other.example.com"));
    }

    #[test]
    fn fqdn_match_pattern_subdomain_wildcard() {
        let (_c, _t) = cilium_test_ctx!("pkg/fqdn/matchpattern/matchpattern.go", "MatchPattern", "tenant-fqdn-mpwc");
        assert!(match_pattern("*.example.com", "api.example.com"));
        assert!(match_pattern("*.example.com", "other.example.com"));
        // Bare apex must NOT match (upstream behaviour).
        assert!(!match_pattern("*.example.com", "example.com"));
        // Two-deep subdomains do not match a single-`*` pattern.
        assert!(!match_pattern("*.example.com", "a.b.example.com"));
    }

    #[test]
    fn fqdn_match_pattern_single_label_wildcard() {
        let (_c, _t) = cilium_test_ctx!("pkg/fqdn/matchpattern/matchpattern.go", "MatchPattern.Star", "tenant-fqdn-mpstar");
        assert!(match_pattern("*", "single"));
        assert!(!match_pattern("*", "two.parts"));
    }

    // ── Identity range ───────────────────────────────────────────────────────

    #[test]
    fn fqdn_identity_constants_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "IdentityScopeRemoteNode", "tenant-fqdn-rng-c");
        assert_eq!(FQDN_IDENTITY_MIN, 16_777_216);
        assert_eq!(FQDN_IDENTITY_MAX, FQDN_IDENTITY_MIN + (1 << 24));
    }

    #[test]
    fn fqdn_serde_round_trip_for_entry() {
        let (_c, _t) = cilium_test_ctx!("pkg/fqdn/fqdn.go", "FQDNEntry.Serde", "tenant-fqdn-serde");
        let entry = FqdnEntry {
            identity: FQDN_IDENTITY_MIN, pattern: "*.example.com".into(),
            fqdn: "api.example.com".into(), ip: ip(1, 2, 3, 4),
            ttl_seconds: 300, created: 100,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: FqdnEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }
}
