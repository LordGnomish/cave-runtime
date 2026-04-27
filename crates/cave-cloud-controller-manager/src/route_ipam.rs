//! IPAM (IP Address Management) — pod CIDR allocator + route retry policy.
//!
//! Mirrors the upstream pieces in
//! `staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam` plus the
//! retry/backoff logic the route controller wraps around its provider
//! calls.
//!
//! * **CidrAllocator** — slices a parent /16 (or /48 for v6) into
//!   per-node /24 (or /64) blocks, returning fresh ones to callers.
//! * **CidrMaskSize** — defaults match upstream's
//!   `--node-cidr-mask-size`(-ipv4/-ipv6) flags.
//! * **BackoffSchedule** — exponential backoff with jitter caps and a
//!   max retries bound.
//! * **CIDR overlap** detection — used to refuse misconfigured cluster
//!   CIDRs that would collide with existing routes.

use crate::route_controller::{cidr_family, CidrFamily};
use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── Default mask sizes ──────────────────────────────────────────────────────

/// Default per-node IPv4 mask size. Matches `--node-cidr-mask-size-ipv4=24`.
pub const DEFAULT_NODE_CIDR_MASK_V4: u8 = 24;
/// Default per-node IPv6 mask size. Matches `--node-cidr-mask-size-ipv6=64`.
pub const DEFAULT_NODE_CIDR_MASK_V6: u8 = 64;

/// Reject mask sizes upstream's nodeipam controller does. v4 mask must be
/// in `(parent, 30]`; v6 mask must be in `(parent, 64]`.
pub fn validate_mask_size(family: CidrFamily, parent_mask: u8, node_mask: u8) -> Result<(), CloudError> {
    let max_node = match family {
        CidrFamily::V4 => 30u8,
        CidrFamily::V6 => 64u8,
    };
    let parent_limit = match family {
        CidrFamily::V4 => 32u8,
        CidrFamily::V6 => 128u8,
    };
    if parent_mask >= parent_limit {
        return Err(CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: format!("parent mask {parent_mask} must be < {parent_limit}"),
        });
    }
    if node_mask <= parent_mask {
        return Err(CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: format!(
                "node mask {node_mask} must be greater than parent mask {parent_mask}"
            ),
        });
    }
    if node_mask > max_node {
        return Err(CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: format!("node mask {node_mask} exceeds {max_node} for family {family:?}"),
        });
    }
    Ok(())
}

// ─── IPv4 helpers ────────────────────────────────────────────────────────────

fn parse_v4_addr(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut out: u32 = 0;
    for p in parts {
        let n: u32 = p.parse().ok()?;
        if n > 255 {
            return None;
        }
        out = (out << 8) | n;
    }
    Some(out)
}

fn parse_v4_cidr(s: &str) -> Option<(u32, u8)> {
    let (addr, prefix) = s.split_once('/')?;
    let mask: u8 = prefix.parse().ok()?;
    if mask > 32 {
        return None;
    }
    Some((parse_v4_addr(addr)?, mask))
}

fn format_v4(addr: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        (addr >> 24) & 0xff,
        (addr >> 16) & 0xff,
        (addr >> 8) & 0xff,
        addr & 0xff
    )
}

// ─── CIDR overlap ────────────────────────────────────────────────────────────

/// True iff CIDRs `a` and `b` overlap — either contains the other or shares
/// a prefix. Mirrors `utilnet.IsCIDROverlap`.
pub fn cidrs_overlap(a: &str, b: &str) -> bool {
    match (parse_v4_cidr(a), parse_v4_cidr(b)) {
        (Some((aa, am)), Some((ba, bm))) => {
            let common = am.min(bm);
            let mask: u32 = if common == 0 { 0 } else { (!0u32) << (32 - common) };
            (aa & mask) == (ba & mask)
        }
        // V6 / mixed-family / malformed: report as non-overlapping —
        // callers should validate first.
        _ => false,
    }
}

// ─── CidrAllocator ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CidrAllocator {
    family: CidrFamily,
    parent_addr: u32,
    parent_mask: u8,
    node_mask: u8,
    next_index: u64,
    in_use: Vec<u64>,
}

impl CidrAllocator {
    pub fn new(parent_cidr: &str, node_mask: u8) -> Result<Self, CloudError> {
        let family = cidr_family(parent_cidr).ok_or_else(|| CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: format!("parent_cidr {parent_cidr:?} is not a valid CIDR"),
        })?;
        match family {
            CidrFamily::V4 => {
                let (addr, parent_mask) =
                    parse_v4_cidr(parent_cidr).ok_or_else(|| CloudError::InvalidConfig {
                        provider: ProviderName::Hetzner,
                        reason: format!("parent_cidr {parent_cidr:?} is not v4"),
                    })?;
                validate_mask_size(family, parent_mask, node_mask)?;
                Ok(Self {
                    family,
                    parent_addr: addr,
                    parent_mask,
                    node_mask,
                    next_index: 0,
                    in_use: Vec::new(),
                })
            }
            CidrFamily::V6 => {
                // We don't run an allocator on v6 in tests; reject with a
                // clear error so callers know to use a v4 parent.
                Err(CloudError::Unimplemented(
                    "v6 CidrAllocator is not yet implemented",
                ))
            }
        }
    }

    pub fn capacity(&self) -> u64 {
        let bits = (self.node_mask - self.parent_mask) as u32;
        if bits >= 32 {
            u32::MAX as u64
        } else {
            1u64 << bits
        }
    }

    pub fn used(&self) -> u64 {
        self.in_use.len() as u64
    }

    pub fn family(&self) -> CidrFamily {
        self.family
    }

    /// Allocate a fresh CIDR.
    pub fn allocate(&mut self) -> Result<String, CloudError> {
        if self.used() >= self.capacity() {
            return Err(CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: "pod CIDR space exhausted".into(),
            });
        }
        // Find the next free index.
        loop {
            if !self.in_use.contains(&self.next_index) {
                break;
            }
            self.next_index += 1;
            if self.next_index >= self.capacity() {
                return Err(CloudError::Upstream {
                    provider: ProviderName::Hetzner,
                    reason: "pod CIDR space exhausted".into(),
                });
            }
        }
        let idx = self.next_index;
        self.in_use.push(idx);
        self.next_index += 1;
        Ok(self.cidr_at(idx))
    }

    /// Reserve a specific CIDR (used when re-loading state at controller
    /// boot). Refuses overlap.
    pub fn reserve(&mut self, cidr: &str) -> Result<(), CloudError> {
        let idx = self.index_of(cidr)?;
        if self.in_use.contains(&idx) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("CIDR {cidr} already allocated"),
            });
        }
        self.in_use.push(idx);
        Ok(())
    }

    pub fn release(&mut self, cidr: &str) -> Result<(), CloudError> {
        let idx = self.index_of(cidr)?;
        self.in_use.retain(|i| *i != idx);
        Ok(())
    }

    fn index_of(&self, cidr: &str) -> Result<u64, CloudError> {
        let (addr, mask) = parse_v4_cidr(cidr).ok_or_else(|| CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: format!("not a v4 CIDR: {cidr:?}"),
        })?;
        if mask != self.node_mask {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("CIDR mask /{mask} does not match allocator /{}", self.node_mask),
            });
        }
        let parent_bits = self.parent_mask as u32;
        let parent_mask: u32 = if parent_bits == 0 { 0 } else { !0u32 << (32 - parent_bits) };
        if (addr & parent_mask) != (self.parent_addr & parent_mask) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("CIDR {cidr} not within allocator parent"),
            });
        }
        let block_size = 1u32 << (32 - self.node_mask as u32);
        let offset = addr.wrapping_sub(self.parent_addr);
        let idx = offset / block_size;
        Ok(idx as u64)
    }

    fn cidr_at(&self, idx: u64) -> String {
        let block_size: u32 = 1u32 << (32 - self.node_mask as u32);
        let addr = self.parent_addr + (idx as u32 * block_size);
        format!("{}/{}", format_v4(addr), self.node_mask)
    }
}

// ─── Backoff schedule ────────────────────────────────────────────────────────

/// Exponential backoff schedule. Mirrors `utilretry.DefaultBackoff` with the
/// caps the route controller uses by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackoffSchedule {
    pub initial_ms: u32,
    pub multiplier: u32,
    pub max_ms: u32,
    pub max_retries: u32,
}

impl BackoffSchedule {
    pub const fn default_route() -> Self {
        Self { initial_ms: 250, multiplier: 2, max_ms: 30_000, max_retries: 8 }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if self.initial_ms == 0 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "backoff initial_ms must be > 0".into(),
            });
        }
        if self.multiplier < 2 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "backoff multiplier must be >= 2".into(),
            });
        }
        if self.max_ms < self.initial_ms {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "backoff max_ms must be >= initial_ms".into(),
            });
        }
        if self.max_retries == 0 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "backoff max_retries must be > 0".into(),
            });
        }
        Ok(())
    }

    /// Delay for retry `n` (0-indexed). Caps at `max_ms`.
    pub fn delay_ms(&self, retry: u32) -> u32 {
        let mut v = self.initial_ms as u64;
        for _ in 0..retry {
            v = v.saturating_mul(self.multiplier as u64);
            if v > self.max_ms as u64 {
                return self.max_ms;
            }
        }
        v.min(self.max_ms as u64) as u32
    }

    pub fn should_retry(&self, retry: u32) -> bool {
        retry < self.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) {
        let (cite, _t) = test_ctx!(path, sym, tenant);
        assert_eq!(cite.repo, "kubernetes/kubernetes");
    }

    // ─── Mask defaults ───────────────────────────────────────────────────────

    #[test]
    fn default_node_cidr_masks_match_upstream_flags() {
        ctx("acme", "cmd/kube-controller-manager/app/options/options.go", "NodeCIDRMaskSize");
        assert_eq!(DEFAULT_NODE_CIDR_MASK_V4, 24);
        assert_eq!(DEFAULT_NODE_CIDR_MASK_V6, 64);
    }

    #[test]
    fn validate_mask_size_accepts_v4_default() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "validateMaskSize");
        assert!(validate_mask_size(CidrFamily::V4, 16, 24).is_ok());
        assert!(validate_mask_size(CidrFamily::V4, 8, 30).is_ok());
    }

    #[test]
    fn validate_mask_size_rejects_node_mask_smaller_than_parent() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "validateMaskSize");
        let err = validate_mask_size(CidrFamily::V4, 24, 16).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn validate_mask_size_rejects_v4_node_mask_above_30() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "validateMaskSize");
        let err = validate_mask_size(CidrFamily::V4, 16, 31).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn validate_mask_size_v6_default_validates() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "validateMaskSize");
        assert!(validate_mask_size(CidrFamily::V6, 48, 64).is_ok());
    }

    #[test]
    fn validate_mask_size_rejects_v6_node_mask_above_64() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "validateMaskSize");
        let err = validate_mask_size(CidrFamily::V6, 48, 96).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn validate_mask_size_rejects_full_parent_mask() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "validateMaskSize");
        assert!(validate_mask_size(CidrFamily::V4, 32, 24).is_err());
    }

    // ─── CIDR overlap ────────────────────────────────────────────────────────

    #[test]
    fn cidrs_overlap_detects_identical_cidrs() {
        ctx("acme", "staging/src/k8s.io/utils/net/ipnet.go", "IsCIDROverlap");
        assert!(cidrs_overlap("10.0.0.0/16", "10.0.0.0/16"));
    }

    #[test]
    fn cidrs_overlap_detects_supernet_subnet() {
        ctx("acme", "staging/src/k8s.io/utils/net/ipnet.go", "IsCIDROverlap");
        assert!(cidrs_overlap("10.0.0.0/8", "10.1.0.0/16"));
        assert!(cidrs_overlap("10.1.0.0/16", "10.0.0.0/8"));
    }

    #[test]
    fn cidrs_overlap_returns_false_for_disjoint() {
        ctx("acme", "staging/src/k8s.io/utils/net/ipnet.go", "IsCIDROverlap");
        assert!(!cidrs_overlap("10.0.0.0/16", "192.168.0.0/16"));
    }

    #[test]
    fn cidrs_overlap_handles_zero_mask() {
        ctx("acme", "staging/src/k8s.io/utils/net/ipnet.go", "IsCIDROverlap");
        assert!(cidrs_overlap("0.0.0.0/0", "10.0.0.0/24"));
    }

    // ─── CidrAllocator ───────────────────────────────────────────────────────

    #[test]
    fn allocator_construction_validates_parent_cidr() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "NewCIDRRangeAllocator");
        assert!(CidrAllocator::new("garbage", 24).is_err());
        assert!(CidrAllocator::new("10.0.0.0/16", 24).is_ok());
    }

    #[test]
    fn allocator_capacity_matches_2_to_the_diff() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "NewCIDRRangeAllocator");
        let a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        assert_eq!(a.capacity(), 256);
        assert_eq!(a.used(), 0);
        assert_eq!(a.family(), CidrFamily::V4);
    }

    #[test]
    fn allocator_emits_distinct_consecutive_cidrs() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "AllocateNext");
        let mut a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        assert_eq!(a.allocate().unwrap(), "10.0.0.0/24");
        assert_eq!(a.allocate().unwrap(), "10.0.1.0/24");
        assert_eq!(a.allocate().unwrap(), "10.0.2.0/24");
        assert_eq!(a.used(), 3);
    }

    #[test]
    fn allocator_release_returns_cidr_to_pool() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "Release");
        let mut a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        a.allocate().unwrap();
        a.allocate().unwrap();
        a.release("10.0.0.0/24").unwrap();
        assert_eq!(a.used(), 1);
    }

    #[test]
    fn allocator_reserve_records_pre_existing_cidr() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "Occupy");
        let mut a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        a.reserve("10.0.5.0/24").unwrap();
        assert_eq!(a.used(), 1);
    }

    #[test]
    fn allocator_reserve_rejects_double_allocation() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "Occupy");
        let mut a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        a.reserve("10.0.5.0/24").unwrap();
        let err = a.reserve("10.0.5.0/24").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn allocator_reserve_rejects_wrong_mask() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "Occupy");
        let mut a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        let err = a.reserve("10.0.5.0/25").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn allocator_reserve_rejects_outside_parent() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "Occupy");
        let mut a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        let err = a.reserve("192.168.0.0/24").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn allocator_exhaustion_returns_upstream_error() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "AllocateNext");
        let mut a = CidrAllocator::new("10.0.0.0/28", 30).unwrap();
        // /28 → /30 = 4 entries
        for _ in 0..4 {
            a.allocate().unwrap();
        }
        let err = a.allocate().unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[test]
    fn allocator_skips_already_reserved_indices() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "AllocateNext");
        let mut a = CidrAllocator::new("10.0.0.0/16", 24).unwrap();
        a.reserve("10.0.0.0/24").unwrap();
        // Next free index is 1 (10.0.1.0/24).
        assert_eq!(a.allocate().unwrap(), "10.0.1.0/24");
    }

    #[test]
    fn allocator_v6_construction_is_unimplemented_for_now() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go", "NewCIDRRangeAllocator");
        let err = CidrAllocator::new("2001:db8::/48", 64).unwrap_err();
        assert!(matches!(err, CloudError::Unimplemented(_)));
    }

    // ─── BackoffSchedule ─────────────────────────────────────────────────────

    #[test]
    fn default_backoff_schedule_validates() {
        ctx("acme", "staging/src/k8s.io/client-go/util/retry/util.go", "DefaultBackoff");
        assert!(BackoffSchedule::default_route().validate().is_ok());
    }

    #[test]
    fn backoff_delay_grows_exponentially_then_caps() {
        ctx("acme", "staging/src/k8s.io/apimachinery/pkg/util/wait/backoff.go", "Step");
        let s = BackoffSchedule::default_route();
        assert_eq!(s.delay_ms(0), 250);
        assert_eq!(s.delay_ms(1), 500);
        assert_eq!(s.delay_ms(2), 1_000);
        // Caps at max_ms (30_000).
        assert_eq!(s.delay_ms(20), 30_000);
    }

    #[test]
    fn backoff_should_retry_respects_max_retries() {
        ctx("acme", "staging/src/k8s.io/apimachinery/pkg/util/wait/backoff.go", "Step");
        let s = BackoffSchedule::default_route();
        assert!(s.should_retry(0));
        assert!(s.should_retry(7));
        assert!(!s.should_retry(8));
        assert!(!s.should_retry(99));
    }

    #[test]
    fn backoff_validate_rejects_zero_initial() {
        ctx("acme", "staging/src/k8s.io/apimachinery/pkg/util/wait/backoff.go", "Step");
        let mut s = BackoffSchedule::default_route();
        s.initial_ms = 0;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn backoff_validate_rejects_multiplier_below_2() {
        ctx("acme", "staging/src/k8s.io/apimachinery/pkg/util/wait/backoff.go", "Step");
        let mut s = BackoffSchedule::default_route();
        s.multiplier = 1;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn backoff_validate_rejects_max_below_initial() {
        ctx("acme", "staging/src/k8s.io/apimachinery/pkg/util/wait/backoff.go", "Step");
        let mut s = BackoffSchedule::default_route();
        s.max_ms = s.initial_ms - 1;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn backoff_validate_rejects_zero_max_retries() {
        ctx("acme", "staging/src/k8s.io/apimachinery/pkg/util/wait/backoff.go", "Step");
        let mut s = BackoffSchedule::default_route();
        s.max_retries = 0;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }
}
