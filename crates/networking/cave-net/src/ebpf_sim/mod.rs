// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! eBPF userspace simulation.
//!
//! Cilium's hot path lives in BPF programs compiled by clang and
//! attached to TC/XDP hooks. Running the upstream tests against
//! that requires a live kernel + a clang toolchain, neither of
//! which fits a deterministic `cargo test` run.
//!
//! This module provides a userspace simulator with the **observable
//! state-machine behaviour** every Cilium BPF program ships:
//!
//!   * **Maps** — `Map<K, V>` with `lookup` / `update` / `delete` /
//!     `iter_keys` / `len` mirroring the kernel `bpf_map_*` helpers.
//!     LRU + LFU + Array variants approximate the kernel choices.
//!   * **Helpers** — `bpf_ktime_get_ns`, `bpf_get_smp_processor_id`,
//!     `perf_event_output`, `bpf_redirect`, … reduced to deterministic
//!     userspace shims so a test can pin time and CPU.
//!   * **Program trait** — `Program::run(&self, &mut Context)` returns
//!     a `Verdict` (`Pass`, `Drop`, `Redirect(ifindex)`). Concrete
//!     programs (`bpf_lxc_sim`, `bpf_host_sim`, `conntrack_sim`)
//!     live as siblings.
//!
//! The simulator is NOT a packet emulator. Buffer manipulation,
//! header parsing, and checksum updates are out of scope — Cilium's
//! datapath tests cover those at the kernel level. This sim covers
//! the **control-plane behaviour**: map state transitions, policy
//! verdict tables, conntrack expiry semantics. That's the shape the
//! 2026-05-14 cave-net behavioral-parity audit measured.

pub mod bpf_host_sim;
pub mod bpf_lxc_sim;
pub mod conntrack_sim;
pub mod edt_sim;
pub mod helpers;
pub mod lb_sim;
pub mod map;
pub mod nat46x64;
pub mod nat_sim;
pub mod policy_lpm;
pub mod port_range;
pub mod program;

pub use bpf_host_sim::{HostProgram, HostVerdict};
pub use bpf_lxc_sim::{LxcEndpointInfo, LxcMap, LxcProgram};
pub use conntrack_sim::{ConntrackEntry, ConntrackKey, ConntrackMap, CtAction, CtDirection};
pub use edt_sim::{
    edt_sched_departure, EdtInfo, EdtThrottleMap, EdtVerdict, DEFAULT_DROP_HORIZON_NS, NSEC_PER_SEC,
};
pub use helpers::{Helpers, MockClock};
pub use lb_sim::{
    LbAlgo, LbBackend, LbKey, LbMaps, LbServiceMaster, LbTuple, LbXlate, RevNatEntry, RevNatResult,
    HASH_INIT4_SEED, LB_MAGLEV_LUT_SIZE,
};
pub use map::{Map, MapError, MapKind};
pub use nat46x64::{
    build_v4_in_v6, build_v4_in_v6_rfc6052, get_v4_from_v6, is_v4_in_v6, is_v4_in_v6_rfc6052,
    V6Addr, RFC6052_WELL_KNOWN_PREFIX,
};
pub use nat_sim::{NatDir, NatEntry, NatError, NatMap, NatTarget, NatTuple, SNAT_COLLISION_RETRIES};
pub use policy_lpm::RangePolicyMap;
pub use port_range::{port_range_to_masked_ports, MaskedPort};
pub use program::{Context, Program, Verdict};
