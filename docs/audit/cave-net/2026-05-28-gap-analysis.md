<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-net honest-ratio gap analysis — Cilium v1.19.3 (incl. Hubble)

- **Upstream:** `cilium/cilium` @ `v1.19.3` (Apache-2.0). Baseline clone `/tmp/cilium-baseline`.
- **Crate:** `crates/networking/cave-net`
- **Audit date:** 2026-05-30 (file retains the sprint-series `2026-05-28` slug)
- **Method:** line-by-line read of `bpf/lib/*.h` datapath headers against the
  `src/ebpf_sim/` userspace substitute, then strict RED→GREEN TDD to close the
  concrete datapath gaps. Control-plane parity (agent REST/gRPC, CRDs, policy
  engine, Hubble flow API, L7 parsers, BGP, ClusterMesh) is already deeply
  ported across the 90 `src/cilium/*.rs` files (~49.5k LOC).

## Pre-sprint baseline (origin/main)

`parity.manifest.toml` reports:

| metric        | value |
|---------------|-------|
| mapped        | 42    |
| skipped       | 92    |
| unmapped      | 0     |
| total         | 134   |
| fill_ratio    | 1.0   |
| honest_ratio  | 0.9851 |

Test baseline: **1746 lib tests + 122 integration tests, 0 failures.**

### Honesty note on the `skipped` bucket

The 11 historically-`unmapped` rows (BPF bytecode datapath, cloud IPAM,
Linux netlink) were reclassified `unmapped → skipped` in the 2026-05-19
finalize as "wire-format-detail; cave-net uses the Rust `ebpf_sim` userspace
substitute." `honest_ratio` was *pinned* at the pre-reclassification 0.9851
"for transparency." That is honest accounting **only if the `ebpf_sim`
substitute genuinely models the datapath behaviour it stands in for.** This
audit verifies that claim component-by-component and fills the gaps where the
substitute was thinner than the documentation implied.

## Upstream datapath LOC inventory (`bpf/lib/`)

| header            | LOC  | observable behaviour | ebpf_sim coverage (pre) |
|-------------------|------|----------------------|-------------------------|
| `conntrack.h`     | 1375 | 5-tuple CT state machine, expiry | ✅ `conntrack_sim.rs` (351) |
| `conntrack_map.h` |  205 | CT key/entry layout | ✅ folded into `conntrack_sim` |
| `nat.h`           | 2362 | SNAT/DNAT masquerade, port alloc | ❌ **MISSING** |
| `lb.h`            | 2429 | service→backend DNAT, rev-NAT, backend selection | ❌ **MISSING** |
| `ipv4.h`/`ipv6.h` |  629 | header field accessors | ⚪ out-of-scope (no packet buffer) |
| `policy.h`        |  487 | verdict map lookup | ✅ `bpf_lxc_sim.rs` verdict table |
| `maps.h`+helpers  |  ~9k | map/helper primitives | ✅ `map.rs`, `helpers.rs` |
| `bpf/**` total    | 302 files | full TC/XDP datapath | userspace state-machine subset |

## Phase 2 — gap matrix (datapath, priority order from brief)

| # | component | upstream | pre-sprint cave | gap class | action |
|---|-----------|----------|-----------------|-----------|--------|
| 1a | conntrack datapath | `bpf/lib/conntrack.h` | `ebpf_sim/conntrack_sim.rs` | **covered** | none |
| 1b | **LB datapath** (svc→backend DNAT, rev-NAT, random/maglev select) | `bpf/lib/lb.h` | only control-plane `cilium/lb.rs` | **REAL GAP** | **TDD cycle 2** |
| 1c | **NAT datapath** (SNAT masquerade, port alloc, rev-SNAT) | `bpf/lib/nat.h` | only control-plane `cilium/nat.rs` | **REAL GAP** | **TDD cycle 1** |
| 2 | CNI pod IP alloc | `pkg/ipam`, `plugins/cilium-cni` | `cilium/ipam.rs` (950), `cilium/cni_chain.rs` | covered | none |
| 3 | BGP control plane | `pkg/bgpv1` | `cilium/bgp.rs` (992), `bgp_types.rs` | covered | none |
| 4 | ClusterMesh | `clustermesh-apiserver` | `cilium/clustermesh{,_ext}.rs` | covered | none |
| 5 | Hubble flow API + L7 | `pkg/hubble` | `cilium/hubble{,_ext,_metrics}.rs` (~3.6k), `kafka.rs`, `dns_proxy.rs`, `l7policy.rs` | covered | none |
| 6 | NetworkPolicy / CNP | `pkg/policy` | `cilium/policy.rs` (2068), `selector_cache.rs` | covered | none |
| 7 | sidecarless Envoy | `pkg/envoy` | `cilium/envoy.rs`, `xds.rs`, `ztunnel.rs` | covered | none |
| 8 | identity-based security | `pkg/identity` | `cilium/identity.rs`, `kv_identity.rs`, `reserved_ids.rs` | covered | none |
| 9 | WireGuard | `pkg/wireguard` | `cilium/wireguard.rs` | covered | none |

**Conclusion:** the two genuine datapath gaps are the **NAT** (`nat.h`) and **LB**
(`lb.h`) hot-path state machines — the only `bpf/lib/*.h` headers with no
`ebpf_sim` counterpart, yet both are explicitly priority-#1 in the sprint brief
("eBPF datapath load_balancer/conntrack/NAT"). Everything else the brief lists is
already control-plane-ported. This sprint adds `ebpf_sim/nat_sim.rs` and
`ebpf_sim/lb_sim.rs` as **userspace datapath approximations** (NOT stubs): they
line-port the observable port-allocation, backend-selection, DNAT and reverse-NAT
state transitions from `nat.h`/`lb.h`, with the packet-buffer/checksum mechanics
explicitly out of scope (covered by upstream's kernel BPF tests).

## Phase 3 — TDD cycle log

_(filled in below as cycles land)_

## Phase 4 — post-sprint honest_ratio

_(computed below)_
