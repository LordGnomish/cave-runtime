//! Userspace simulation of Cilium's connection tracker.
//!
//! Cite: cilium/bpf/lib/conntrack.h + cilium/bpf/lib/conntrack_map.h
//!       + cilium/pkg/maps/ctmap (v1.19.3).
//!
//! Per-flow entries keyed by the 5-tuple
//! `(saddr, daddr, sport, dport, nexthdr)`. The entry stores the
//! direction (`Ingress`/`Egress`), last-seen timestamp, and a
//! lifetime. On packet arrival the program:
//!
//!   1. Looks up the 5-tuple.
//!   2. If found AND not expired → update `last_seen`, return
//!      `CtAction::Established`.
//!   3. If found BUT expired → delete + treat as new.
//!   4. If not found → insert + return `CtAction::New`.
//!
//! Expiry timeout follows upstream defaults:
//!   * TCP established: 21600 s (6 h)
//!   * UDP / ICMP:      30 s
//!   * TCP SYN-only:    60 s (we don't model SYN flag — same as UDP)

use crate::ebpf_sim::helpers::Helpers;
use crate::ebpf_sim::map::{Map, MapError, UpdateFlag};
use crate::ebpf_sim::program::{Context, L4Proto};
use serde::{Deserialize, Serialize};

/// Cilium's CT_INGRESS / CT_EGRESS / CT_RELATED bitfield. We model
/// the two we care about (ingress + egress); RELATED is for ICMP
/// errors and out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CtDirection {
    Ingress,
    Egress,
}

/// 5-tuple. Matches `struct ipv4_ct_tuple` in conntrack_map.h
/// reduced to userspace fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConntrackKey {
    pub saddr: u32,
    pub daddr: u32,
    pub sport: u16,
    pub dport: u16,
    pub nexthdr: u8,
    pub direction: CtDirection,
}

impl ConntrackKey {
    /// Build from a `Context` + direction. Mirrors the kernel
    /// `ct_lookup4`'s key-construction flow.
    pub fn from_ctx(ctx: &Context, direction: CtDirection) -> Self {
        let (saddr, sport, daddr, dport) = match direction {
            CtDirection::Egress => (ctx.src_ip.0, ctx.src_port, ctx.dst_ip.0, ctx.dst_port),
            CtDirection::Ingress => (ctx.dst_ip.0, ctx.dst_port, ctx.src_ip.0, ctx.src_port),
        };
        Self {
            saddr,
            daddr,
            sport,
            dport,
            nexthdr: ctx.proto.proto_num(),
            direction,
        }
    }
}

/// Per-connection state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConntrackEntry {
    pub created_at_ns: u64,
    pub last_seen_ns: u64,
    pub lifetime_ns: u64,
    pub packets: u64,
    pub bytes: u64,
    pub flags: u32,
}

impl ConntrackEntry {
    pub fn new(now_ns: u64, lifetime_ns: u64) -> Self {
        Self {
            created_at_ns: now_ns,
            last_seen_ns: now_ns,
            lifetime_ns,
            packets: 1,
            bytes: 0,
            flags: 0,
        }
    }

    pub fn is_expired(&self, now_ns: u64) -> bool {
        now_ns.saturating_sub(self.last_seen_ns) >= self.lifetime_ns
    }
}

pub type ConntrackMap = Map<ConntrackKey, ConntrackEntry>;

pub fn new_conntrack_map(capacity: u32) -> ConntrackMap {
    Map::new_lru_hash(capacity)
}

/// Default TCP established lifetime — 6 hours.
pub const CT_TCP_LIFETIME_NS: u64 = 21_600 * 1_000_000_000;
/// Default UDP / ICMP lifetime — 30 seconds.
pub const CT_UDP_LIFETIME_NS: u64 = 30 * 1_000_000_000;

pub fn default_lifetime_ns(proto: L4Proto) -> u64 {
    match proto {
        L4Proto::Tcp => CT_TCP_LIFETIME_NS,
        _ => CT_UDP_LIFETIME_NS,
    }
}

/// Action returned by `ct_lookup`. Upstream uses an enum
/// `CT_NEW` / `CT_ESTABLISHED` / `CT_REPLY` / `CT_RELATED`; we
/// reduce to the three a state-machine test needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CtAction {
    /// No matching entry — caller should let policy evaluate this
    /// packet and (on accept) insert a new entry.
    New,
    /// Entry exists + within lifetime — bump last_seen.
    Established,
    /// Entry existed but expired — caller treats as New AFTER
    /// deleting the stale entry. We surface this as a distinct
    /// action so tests can verify the GC path.
    Expired,
}

/// Look up `key` in the CT map, advancing `last_seen` on hit.
/// `now_ns` is the helper-provided clock so tests can fix it.
pub fn ct_lookup(
    map: &mut ConntrackMap,
    key: &ConntrackKey,
    now_ns: u64,
) -> CtAction {
    match map.lookup(key) {
        Some(entry) => {
            if entry.is_expired(now_ns) {
                // Drop the stale entry. Use `delete` directly so we
                // don't churn the LRU recency state.
                let _ = map.delete(key);
                CtAction::Expired
            } else {
                // Bump last_seen + packets.
                let mut bumped = entry.clone();
                bumped.last_seen_ns = now_ns;
                bumped.packets = bumped.packets.saturating_add(1);
                // The earlier `lookup` already advanced LRU recency.
                let _ = map.update(*key, bumped, UpdateFlag::Exist);
                CtAction::Established
            }
        }
        None => CtAction::New,
    }
}

/// Insert a fresh entry — the verdict after policy returns ACCEPT
/// is conditional on this succeeding.
pub fn ct_create(
    map: &mut ConntrackMap,
    key: ConntrackKey,
    now_ns: u64,
    lifetime_ns: u64,
) -> Result<(), MapError> {
    map.update(key, ConntrackEntry::new(now_ns, lifetime_ns), UpdateFlag::Any)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebpf_sim::map::MapError;
    use crate::ebpf_sim::program::Ipv4;

    fn key_tcp(direction: CtDirection) -> ConntrackKey {
        ConntrackKey {
            saddr: Ipv4::from_octets(10, 0, 0, 1).0,
            daddr: Ipv4::from_octets(10, 0, 0, 2).0,
            sport: 12345,
            dport: 80,
            nexthdr: L4Proto::Tcp.proto_num(),
            direction,
        }
    }

    /// Upstream port — Cilium `pkg/maps/ctmap/ctmap_test.go::TestConntrackV4`.
    /// First packet: New. Insert entry with TCP lifetime. Second
    /// packet at same instant: Established. Packet count bumps.
    #[test]
    fn upstream_test_conntrack_v4_new_then_established() {
        let helpers = Helpers::new();
        let mut m = new_conntrack_map(1024);
        let k = key_tcp(CtDirection::Egress);
        let now = helpers.ktime_get_ns();
        assert_eq!(ct_lookup(&mut m, &k, now), CtAction::New);
        ct_create(&mut m, k, now, default_lifetime_ns(L4Proto::Tcp)).unwrap();
        assert_eq!(ct_lookup(&mut m, &k, now), CtAction::Established);
        // Packets counter incremented.
        let entry = m.peek(&k).unwrap();
        assert_eq!(entry.packets, 2);
    }

    #[test]
    fn upstream_test_conntrack_v4_expires_after_lifetime() {
        let helpers = Helpers::new();
        let mut m = new_conntrack_map(1024);
        let k = key_tcp(CtDirection::Egress);
        ct_create(&mut m, k, helpers.ktime_get_ns(), CT_UDP_LIFETIME_NS).unwrap();
        // Advance past lifetime.
        helpers.clock.advance(CT_UDP_LIFETIME_NS + 1);
        let action = ct_lookup(&mut m, &k, helpers.ktime_get_ns());
        assert_eq!(action, CtAction::Expired);
        // Entry was GC'd by the Expired path.
        assert!(m.peek(&k).is_none());
    }

    #[test]
    fn upstream_test_conntrack_v4_ingress_and_egress_are_distinct_keys() {
        let mut m = new_conntrack_map(1024);
        let helpers = Helpers::new();
        let k_eg = key_tcp(CtDirection::Egress);
        let k_in = key_tcp(CtDirection::Ingress);
        ct_create(&mut m, k_eg, helpers.ktime_get_ns(), CT_TCP_LIFETIME_NS).unwrap();
        // Ingress lookup misses — they are different keys.
        assert_eq!(ct_lookup(&mut m, &k_in, helpers.ktime_get_ns()), CtAction::New);
    }

    #[test]
    fn ct_create_with_duplicate_key_overwrites_under_any_flag() {
        let mut m = new_conntrack_map(1024);
        let k = key_tcp(CtDirection::Egress);
        ct_create(&mut m, k, 0, CT_TCP_LIFETIME_NS).unwrap();
        // Second create with `Any` flag overwrites (matches kernel
        // map behaviour).
        ct_create(&mut m, k, 100, CT_TCP_LIFETIME_NS).unwrap();
        let entry = m.peek(&k).unwrap();
        assert_eq!(entry.created_at_ns, 100);
    }

    #[test]
    fn ct_lookup_advances_last_seen_on_established_hit() {
        let mut m = new_conntrack_map(1024);
        let k = key_tcp(CtDirection::Egress);
        ct_create(&mut m, k, 0, CT_TCP_LIFETIME_NS).unwrap();
        let action = ct_lookup(&mut m, &k, 1_000_000);
        assert_eq!(action, CtAction::Established);
        let entry = m.peek(&k).unwrap();
        assert_eq!(entry.last_seen_ns, 1_000_000);
    }

    #[test]
    fn entry_is_expired_returns_true_at_or_past_lifetime() {
        let e = ConntrackEntry::new(0, 100);
        assert!(!e.is_expired(50));
        assert!(e.is_expired(100));
        assert!(e.is_expired(150));
    }

    #[test]
    fn key_from_ctx_swaps_addr_port_on_ingress() {
        let ctx = Context {
            src_ip: Ipv4::from_octets(10, 0, 0, 1),
            dst_ip: Ipv4::from_octets(10, 0, 0, 2),
            src_port: 12345,
            dst_port: 80,
            proto: L4Proto::Tcp,
            ifindex: 0,
            src_identity: 0,
            dst_identity: 0,
        };
        let k_eg = ConntrackKey::from_ctx(&ctx, CtDirection::Egress);
        let k_in = ConntrackKey::from_ctx(&ctx, CtDirection::Ingress);
        // Egress: saddr=src, daddr=dst
        assert_eq!(k_eg.saddr, ctx.src_ip.0);
        assert_eq!(k_eg.daddr, ctx.dst_ip.0);
        // Ingress: swap.
        assert_eq!(k_in.saddr, ctx.dst_ip.0);
        assert_eq!(k_in.daddr, ctx.src_ip.0);
        // Both carry proto and direction.
        assert_eq!(k_eg.nexthdr, 6);
        assert_eq!(k_in.direction, CtDirection::Ingress);
    }

    #[test]
    fn lru_eviction_under_capacity_pressure() {
        // 4 entries into a 3-cap map — oldest must be evicted.
        let mut m = new_conntrack_map(3);
        for i in 0..4u32 {
            let k = ConntrackKey {
                saddr: i,
                daddr: 0,
                sport: 0,
                dport: 0,
                nexthdr: 6,
                direction: CtDirection::Egress,
            };
            ct_create(&mut m, k, 0, CT_TCP_LIFETIME_NS).unwrap();
        }
        assert_eq!(m.len(), 3);
        // Key 0 (oldest) should be gone.
        let k0 = ConntrackKey { saddr: 0, daddr: 0, sport: 0, dport: 0, nexthdr: 6, direction: CtDirection::Egress };
        assert!(m.peek(&k0).is_none());
    }

    #[test]
    fn create_overflow_on_static_map_returns_at_capacity_when_no_lru() {
        // Build a plain hash map (not LRU) of capacity-via-array.
        let mut m: ConntrackMap = Map::new_array(2);
        let k1 = ConntrackKey { saddr: 1, daddr: 0, sport: 0, dport: 0, nexthdr: 6, direction: CtDirection::Egress };
        let k2 = ConntrackKey { saddr: 2, daddr: 0, sport: 0, dport: 0, nexthdr: 6, direction: CtDirection::Egress };
        let k3 = ConntrackKey { saddr: 3, daddr: 0, sport: 0, dport: 0, nexthdr: 6, direction: CtDirection::Egress };
        ct_create(&mut m, k1, 0, CT_TCP_LIFETIME_NS).unwrap();
        ct_create(&mut m, k2, 0, CT_TCP_LIFETIME_NS).unwrap();
        let err = ct_create(&mut m, k3, 0, CT_TCP_LIFETIME_NS).unwrap_err();
        assert_eq!(err, MapError::AtCapacity(2));
    }
}
