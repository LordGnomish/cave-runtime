// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace simulation of Cilium's `bpf_lxc.c` LXC endpoint map.
//!
//! Cite: cilium/bpf/lxc/bpf_lxc.c + cilium/bpf/lib/lxc.h (v1.19.3).
//!
//! Each pod (LXC = "Linux container", the upstream nomenclature for
//! a network endpoint) has an entry in `ENDPOINTS_MAP` keyed by
//! `(ifindex, mac)`. The entry stores the endpoint's security
//! identity, pod IP, and lxc id. On packet ingress the program
//! looks up the entry, sets the security context, and stamps the
//! identity into the conntrack flow.
//!
//! Our simulator covers the **map state-machine behaviour**:
//!
//!   * `lxc_map_update` adds / overwrites entries.
//!   * `lxc_map_delete` removes entries.
//!   * `lxc_map_lookup` returns `Some(info)` / `None`.
//!   * `LxcProgram::run` looks up by (ifindex, source identity) +
//!     emits `Verdict::Drop` for unknown endpoints.

use crate::ebpf_sim::helpers::Helpers;
use crate::ebpf_sim::map::{Map, UpdateFlag};
use crate::ebpf_sim::program::{Context, Ipv4, Program, Verdict};
use serde::{Deserialize, Serialize};

/// Per-endpoint data the kernel program stores in its hash map.
/// Mirrors Cilium's `struct endpoint_info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LxcEndpointInfo {
    pub ifindex: u32,
    pub lxc_id: u16,
    pub security_identity: u32,
    pub pod_ip: Ipv4,
    pub mac: [u8; 6],
}

/// LXC map key: `(ifindex, lxc_id)`. The kernel composes it from
/// the packet's metadata; we accept the tuple directly.
pub type LxcKey = (u32, u16);

/// Type alias matching the upstream BPF map name.
pub type LxcMap = Map<LxcKey, LxcEndpointInfo>;

/// Construct a fresh hash-typed LXC map. Cilium uses
/// `BPF_MAP_TYPE_HASH` for `ENDPOINTS_MAP_V2`.
pub fn new_lxc_map() -> LxcMap {
    Map::new_hash()
}

/// Helper that mirrors the upstream `endpoint_map_update`. Returns
/// `Ok(())` on success; the underlying `MapError` propagates so
/// tests can assert on the no-exist / exist flag semantics.
pub fn lxc_map_update(
    map: &mut LxcMap,
    info: LxcEndpointInfo,
    flag: UpdateFlag,
) -> Result<(), crate::ebpf_sim::map::MapError> {
    map.update((info.ifindex, info.lxc_id), info, flag)
}

pub fn lxc_map_delete(map: &mut LxcMap, key: LxcKey) -> Result<(), crate::ebpf_sim::map::MapError> {
    map.delete(&key)
}

pub fn lxc_map_lookup(map: &mut LxcMap, key: LxcKey) -> Option<LxcEndpointInfo> {
    map.lookup(&key)
}

/// Simulated `bpf_lxc` ingress program.
pub struct LxcProgram<'a> {
    pub map: &'a mut LxcMap,
}

impl<'a> Program for LxcProgram<'a> {
    fn name(&self) -> &'static str {
        "bpf_lxc"
    }

    fn run(&mut self, ctx: &Context, helpers: &Helpers) -> Verdict {
        // Upstream behaviour: drop if no endpoint entry for the
        // incoming interface + source identity (treated as lxc_id
        // for our simulator).
        let key: LxcKey = (ctx.ifindex, ctx.src_identity as u16);
        match self.map.lookup(&key) {
            Some(info) => {
                // Emit a perf event mimicking trace_lxc().
                let mut payload = Vec::with_capacity(8);
                payload.extend_from_slice(&info.ifindex.to_le_bytes());
                payload.extend_from_slice(&info.lxc_id.to_le_bytes());
                payload.extend_from_slice(&info.security_identity.to_le_bytes()[..2]);
                helpers.perf_event_output(&payload);
                Verdict::Pass
            }
            None => Verdict::Drop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebpf_sim::map::MapError;
    use crate::ebpf_sim::program::L4Proto;

    fn ep(ifindex: u32, lxc_id: u16, identity: u32, pod_ip: Ipv4) -> LxcEndpointInfo {
        LxcEndpointInfo {
            ifindex,
            lxc_id,
            security_identity: identity,
            pod_ip,
            mac: [0; 6],
        }
    }

    /// Upstream port — Cilium `pkg/maps/lxcmap/lxcmap_test.go::TestLxcMapUpdate`.
    /// Behavior: insert new entry, then update the security_identity
    /// in place, lookup observes the change, delete returns Ok,
    /// second delete returns EntryMissing.
    #[test]
    fn upstream_test_lxc_map_update_create_replace_delete_cycle() {
        let mut m = new_lxc_map();
        let ep1 = ep(7, 100, 1234, Ipv4::from_octets(10, 0, 0, 1));
        // 1. Insert new entry.
        lxc_map_update(&mut m, ep1.clone(), UpdateFlag::Any).unwrap();
        assert_eq!(lxc_map_lookup(&mut m, (7, 100)), Some(ep1));
        // 2. Replace identity in place.
        let ep2 = ep(7, 100, 9999, Ipv4::from_octets(10, 0, 0, 1));
        lxc_map_update(&mut m, ep2, UpdateFlag::Any).unwrap();
        let after = lxc_map_lookup(&mut m, (7, 100)).unwrap();
        assert_eq!(after.security_identity, 9999);
        // 3. Delete.
        lxc_map_delete(&mut m, (7, 100)).unwrap();
        assert!(lxc_map_lookup(&mut m, (7, 100)).is_none());
        // 4. Second delete = EntryMissing.
        assert_eq!(
            lxc_map_delete(&mut m, (7, 100)).unwrap_err(),
            MapError::EntryMissing,
        );
    }

    #[test]
    fn upstream_test_lxc_map_update_no_exist_flag_rejects_duplicate() {
        let mut m = new_lxc_map();
        let ep1 = ep(7, 100, 1234, Ipv4::from_octets(10, 0, 0, 1));
        lxc_map_update(&mut m, ep1.clone(), UpdateFlag::Any).unwrap();
        let err = lxc_map_update(&mut m, ep1, UpdateFlag::NoExist).unwrap_err();
        assert_eq!(err, MapError::EntryExists);
    }

    #[test]
    fn lxc_program_drops_unknown_endpoint() {
        let mut m = new_lxc_map();
        let helpers = Helpers::new();
        let ctx = Context {
            src_ip: Ipv4::from_octets(10, 0, 0, 1),
            dst_ip: Ipv4::from_octets(10, 0, 0, 2),
            src_port: 12345,
            dst_port: 80,
            proto: L4Proto::Tcp,
            ifindex: 5,
            src_identity: 42,
            dst_identity: 0,
        };
        let mut prog = LxcProgram { map: &mut m };
        assert_eq!(prog.run(&ctx, &helpers), Verdict::Drop);
        // No perf event for the drop path.
        assert_eq!(helpers.perf_events_len(), 0);
    }

    #[test]
    fn lxc_program_passes_known_endpoint_and_emits_perf_event() {
        let mut m = new_lxc_map();
        lxc_map_update(
            &mut m,
            ep(5, 42, 1234, Ipv4::from_octets(10, 0, 0, 5)),
            UpdateFlag::Any,
        )
        .unwrap();
        let helpers = Helpers::new();
        let ctx = Context {
            src_ip: Ipv4::from_octets(10, 0, 0, 1),
            dst_ip: Ipv4::from_octets(10, 0, 0, 2),
            src_port: 12345,
            dst_port: 80,
            proto: L4Proto::Tcp,
            ifindex: 5,
            src_identity: 42,
            dst_identity: 0,
        };
        let mut prog = LxcProgram { map: &mut m };
        assert_eq!(prog.run(&ctx, &helpers), Verdict::Pass);
        assert_eq!(helpers.perf_events_len(), 1);
    }
}
