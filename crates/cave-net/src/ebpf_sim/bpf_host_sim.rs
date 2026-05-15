//! Userspace simulation of Cilium's `bpf_host.c` policy map.
//!
//! Cite: cilium/bpf/host/bpf_host.c + cilium/pkg/maps/policymap (v1.19.3).
//!
//! The L3/L4 policy map answers "does identity X get to talk to
//! identity Y on (proto, dport)?". Each entry is keyed by
//! `(remote_id, dport, proto, traffic_direction)` and stores a
//! verdict + retention flags.
//!
//! Lookup precedence — verbatim from upstream:
//!   1. Exact match `(peer, port, proto)`.
//!   2. Wildcard port `(peer, 0, proto)`.
//!   3. Wildcard proto `(peer, port, ANY)`.
//!   4. Wildcard peer (`ID_ALL`) — "world" fallback. KEY change in v1.19!
//!   5. Default — deny.

use crate::ebpf_sim::helpers::Helpers;
use crate::ebpf_sim::map::{Map, UpdateFlag};
use crate::ebpf_sim::program::{Context, L4Proto, Program, Verdict};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostVerdict {
    Allow,
    Deny,
    /// Audit-mode — log + allow. Mirrors Cilium's policy_audit_mode.
    Audit,
}

impl From<HostVerdict> for Verdict {
    fn from(v: HostVerdict) -> Self {
        match v {
            HostVerdict::Allow | HostVerdict::Audit => Verdict::Pass,
            HostVerdict::Deny => Verdict::Drop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PolicyKey {
    pub peer_identity: u32,
    pub dport: u16,
    /// 0 = ANY-proto wildcard, otherwise IANA proto number.
    pub proto: u8,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Ingress,
    Egress,
}

/// Cilium's `ID_ALL = 0` — the "world" / fallback identity.
pub const ID_ALL: u32 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyValue {
    pub verdict: HostVerdict,
    /// Bitmask of `POLICY_*` flags. We don't model them; tests
    /// inspect the verdict only.
    pub flags: u32,
}

pub type PolicyMap = Map<PolicyKey, PolicyValue>;

pub fn new_policy_map() -> PolicyMap {
    Map::new_hash()
}

pub fn policy_map_update(
    map: &mut PolicyMap,
    key: PolicyKey,
    value: PolicyValue,
    flag: UpdateFlag,
) -> Result<(), crate::ebpf_sim::map::MapError> {
    map.update(key, value, flag)
}

/// Upstream `policy_can_access_ingress` / `_egress`. Returns the
/// most-specific match in precedence order.
pub fn policy_lookup(
    map: &mut PolicyMap,
    peer_identity: u32,
    dport: u16,
    proto: L4Proto,
    direction: Direction,
) -> HostVerdict {
    // 1. exact (peer, port, proto)
    if let Some(v) = map.lookup(&PolicyKey {
        peer_identity,
        dport,
        proto: proto.proto_num(),
        direction,
    }) {
        return v.verdict;
    }
    // 2. wildcard port (peer, 0, proto)
    if let Some(v) = map.lookup(&PolicyKey {
        peer_identity,
        dport: 0,
        proto: proto.proto_num(),
        direction,
    }) {
        return v.verdict;
    }
    // 3. wildcard proto (peer, port, ANY)
    if let Some(v) = map.lookup(&PolicyKey {
        peer_identity,
        dport,
        proto: 0,
        direction,
    }) {
        return v.verdict;
    }
    // 4. wildcard peer — "world" fallback. ID_ALL with exact port/proto.
    if let Some(v) = map.lookup(&PolicyKey {
        peer_identity: ID_ALL,
        dport,
        proto: proto.proto_num(),
        direction,
    }) {
        return v.verdict;
    }
    // 5. wildcard peer + wildcard port.
    if let Some(v) = map.lookup(&PolicyKey {
        peer_identity: ID_ALL,
        dport: 0,
        proto: proto.proto_num(),
        direction,
    }) {
        return v.verdict;
    }
    // 6. wildcard peer + wildcard proto.
    if let Some(v) = map.lookup(&PolicyKey {
        peer_identity: ID_ALL,
        dport,
        proto: 0,
        direction,
    }) {
        return v.verdict;
    }
    HostVerdict::Deny
}

pub struct HostProgram<'a> {
    pub map: &'a mut PolicyMap,
}

impl<'a> Program for HostProgram<'a> {
    fn name(&self) -> &'static str {
        "bpf_host"
    }

    fn run(&mut self, ctx: &Context, helpers: &Helpers) -> Verdict {
        let _ = helpers; // hot path doesn't emit perf events on policy hit
        let verdict = policy_lookup(
            self.map,
            ctx.src_identity,
            ctx.dst_port,
            ctx.proto,
            Direction::Ingress,
        );
        verdict.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebpf_sim::program::Ipv4;

    fn allow() -> PolicyValue {
        PolicyValue {
            verdict: HostVerdict::Allow,
            flags: 0,
        }
    }

    fn deny() -> PolicyValue {
        PolicyValue {
            verdict: HostVerdict::Deny,
            flags: 0,
        }
    }

    fn key(peer: u32, dport: u16, proto: L4Proto, dir: Direction) -> PolicyKey {
        PolicyKey {
            peer_identity: peer,
            dport,
            proto: proto.proto_num(),
            direction: dir,
        }
    }

    /// Upstream port — Cilium `pkg/maps/policymap/policymap_test.go::TestPolicyMapV4`.
    /// Exact match has highest precedence over wildcards.
    #[test]
    fn upstream_test_policy_map_v4_exact_match_beats_wildcard() {
        let mut m = new_policy_map();
        // Exact: allow (peer=42, port=80, TCP, ingress)
        policy_map_update(
            &mut m,
            key(42, 80, L4Proto::Tcp, Direction::Ingress),
            allow(),
            UpdateFlag::Any,
        )
        .unwrap();
        // Wildcard port: deny (peer=42, port=0, TCP, ingress)
        let wildcard_key = PolicyKey {
            peer_identity: 42,
            dport: 0,
            proto: L4Proto::Tcp.proto_num(),
            direction: Direction::Ingress,
        };
        policy_map_update(&mut m, wildcard_key, deny(), UpdateFlag::Any).unwrap();
        assert_eq!(
            policy_lookup(&mut m, 42, 80, L4Proto::Tcp, Direction::Ingress),
            HostVerdict::Allow,
        );
    }

    #[test]
    fn upstream_test_policy_map_v4_wildcard_port_when_no_exact() {
        let mut m = new_policy_map();
        let wildcard_key = PolicyKey {
            peer_identity: 42,
            dport: 0,
            proto: L4Proto::Tcp.proto_num(),
            direction: Direction::Ingress,
        };
        policy_map_update(&mut m, wildcard_key, allow(), UpdateFlag::Any).unwrap();
        // No exact entry for port 8080 → wildcard port matches.
        assert_eq!(
            policy_lookup(&mut m, 42, 8080, L4Proto::Tcp, Direction::Ingress),
            HostVerdict::Allow,
        );
    }

    #[test]
    fn upstream_test_policy_map_v4_default_deny_when_no_match() {
        let mut m = new_policy_map();
        assert_eq!(
            policy_lookup(&mut m, 42, 80, L4Proto::Tcp, Direction::Ingress),
            HostVerdict::Deny,
        );
    }

    /// Upstream port — `TestPolicyMap/world_fallback_for_non_cluster_peer`.
    /// Precedence #4: when the peer doesn't match any cluster
    /// identity, fall back to ID_ALL.
    #[test]
    fn upstream_test_policy_map_world_fallback() {
        let mut m = new_policy_map();
        let world_key = PolicyKey {
            peer_identity: ID_ALL,
            dport: 80,
            proto: L4Proto::Tcp.proto_num(),
            direction: Direction::Ingress,
        };
        policy_map_update(&mut m, world_key, allow(), UpdateFlag::Any).unwrap();
        // Peer 9999 has NO direct entry — falls back to world.
        assert_eq!(
            policy_lookup(&mut m, 9999, 80, L4Proto::Tcp, Direction::Ingress),
            HostVerdict::Allow,
        );
    }

    #[test]
    fn upstream_test_policy_map_v4_wildcard_proto_when_specific_proto_missing() {
        let mut m = new_policy_map();
        let any_proto_key = PolicyKey {
            peer_identity: 42,
            dport: 80,
            proto: 0,
            direction: Direction::Ingress,
        };
        policy_map_update(&mut m, any_proto_key, allow(), UpdateFlag::Any).unwrap();
        // UDP port 80 → no exact, no wildcard port, wildcard proto matches.
        assert_eq!(
            policy_lookup(&mut m, 42, 80, L4Proto::Udp, Direction::Ingress),
            HostVerdict::Allow,
        );
    }

    #[test]
    fn upstream_test_policy_map_v4_audit_mode_treated_as_allow() {
        let mut m = new_policy_map();
        let key = PolicyKey {
            peer_identity: 42,
            dport: 80,
            proto: L4Proto::Tcp.proto_num(),
            direction: Direction::Ingress,
        };
        policy_map_update(
            &mut m,
            key,
            PolicyValue {
                verdict: HostVerdict::Audit,
                flags: 0,
            },
            UpdateFlag::Any,
        )
        .unwrap();
        let verdict = policy_lookup(&mut m, 42, 80, L4Proto::Tcp, Direction::Ingress);
        assert_eq!(verdict, HostVerdict::Audit);
        // The Program wrapping should treat Audit as Pass.
        let pass: Verdict = verdict.into();
        assert_eq!(pass, Verdict::Pass);
    }

    #[test]
    fn upstream_test_policy_map_v4_direction_isolation() {
        // An ingress entry must NOT satisfy an egress lookup.
        let mut m = new_policy_map();
        policy_map_update(
            &mut m,
            key(42, 80, L4Proto::Tcp, Direction::Ingress),
            allow(),
            UpdateFlag::Any,
        )
        .unwrap();
        let egress = policy_lookup(&mut m, 42, 80, L4Proto::Tcp, Direction::Egress);
        assert_eq!(egress, HostVerdict::Deny);
    }

    #[test]
    fn host_program_drops_when_no_policy_matches() {
        let mut m = new_policy_map();
        let helpers = Helpers::new();
        let ctx = Context {
            src_ip: Ipv4::from_octets(10, 0, 0, 1),
            dst_ip: Ipv4::from_octets(10, 0, 0, 2),
            src_port: 12345,
            dst_port: 443,
            proto: L4Proto::Tcp,
            ifindex: 0,
            src_identity: 5000,
            dst_identity: 0,
        };
        let mut prog = HostProgram { map: &mut m };
        assert_eq!(prog.run(&ctx, &helpers), Verdict::Drop);
    }

    #[test]
    fn host_program_passes_when_world_fallback_allows() {
        let mut m = new_policy_map();
        let world_key = PolicyKey {
            peer_identity: ID_ALL,
            dport: 443,
            proto: L4Proto::Tcp.proto_num(),
            direction: Direction::Ingress,
        };
        policy_map_update(&mut m, world_key, allow(), UpdateFlag::Any).unwrap();
        let ctx = Context {
            src_ip: Ipv4::from_octets(10, 0, 0, 1),
            dst_ip: Ipv4::from_octets(10, 0, 0, 2),
            src_port: 12345,
            dst_port: 443,
            proto: L4Proto::Tcp,
            ifindex: 0,
            src_identity: 5000,
            dst_identity: 0,
        };
        let helpers = Helpers::new();
        let mut prog = HostProgram { map: &mut m };
        assert_eq!(prog.run(&ctx, &helpers), Verdict::Pass);
    }
}
