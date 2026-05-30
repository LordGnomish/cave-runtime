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

Strict RED→GREEN, two commits per cycle (test commit fails to compile →
impl commit passes). No `test+impl` in a single commit.

| cycle | commit | kind | result | notes |
|-------|--------|------|--------|-------|
| — | `a3b0d0b5` | docs | — | Phase 1-2 gap analysis |
| 1 NAT | `c6ac436e` | RED  | FAIL (unresolved `nat_sim` + `Helpers::push_prandom`) | `tests/ebpf_nat_sim.rs` |
| 1 NAT | `621a651b` | GREEN | PASS (8 integ + 3 unit) | `ebpf_sim/nat_sim.rs` + `Helpers::get_prandom_u32` |
| 2 LB  | `1835144c` | RED  | FAIL (unresolved `lb_sim`) | `tests/ebpf_lb_sim.rs` |
| 2 LB  | `90848d69` | GREEN | PASS (8 integ + 4 unit) | `ebpf_sim/lb_sim.rs` |

**LOC delta (src):** `nat_sim.rs` 248 + `lb_sim.rs` 340 + `mod.rs`/`helpers.rs`
wiring ≈ **+638 src LOC**; **+323 test LOC** (`ebpf_nat_sim.rs` 171 +
`ebpf_lb_sim.rs` 152).

**Test count delta:** lib `1746 → 1753` (+7 in-module unit tests);
integration `+16` (8 NAT + 8 LB). **+23 tests total.** Full suite:
**1753 lib + 138 integration, 0 failures.**

### Ported behaviour (line-faithful to v1.19.3)

- **`nat_sim`** ← `bpf/lib/nat.h`: `__snat_clamp_port_range`
  (biased-multiply bounded-rand), `__snat_try_keep_port` (source-port
  preservation), `snat_v4_new_mapping` (the 32-retry `for` loop with
  prandom-then-linear port scan, forward SNAT + reverse RevSNAT entry
  creation, rollback on forward-create failure), `snat_v4_track` (forward
  idempotency), `snat_v4_rev_lookup` (reply restore). `SNAT_COLLISION_RETRIES`
  pinned to upstream `32`.
- **`lb_sim`** ← `bpf/lib/lb.h` + `hash.h` + `jhash.h`: `jhash_3words`
  (Bob-Jenkins lookup3 final mix), `maglev_index` (daddr-excluded consistent
  hashing, `% LB_MAGLEV_LUT_SIZE`=32749, seed `0xcafe`),
  `select_backend_id_random` (`(prandom % count)+1`, slot 0 reserved),
  service/backend/rev-nat map lookups, `lb4_local_{random,maglev}` forward
  DNAT, `lb4_rev_nat` reply restore.

### Explicitly out of scope (documented, not stubbed)

Packet-buffer rewriting, L3/L4 checksum recomputation (`csum_l4_replace`,
`ipv4_csum_update_by_diff`), source-range LPM checks, loopback-SNAT, and the
netlink/conntrack-GC side channels. These are pure wire mechanics with no
control-plane state and are covered by upstream's in-kernel BPF test harness
(`bpf/tests/*.c`), which is not reproducible in a deterministic `cargo test`.
Session affinity / quarantine remain in the control-plane `cilium/services.rs`
+ `cilium/lb.rs` ports.

## Phase 4 — post-sprint honest_ratio

`honest_ratio = (fully_ported_mapped + skipped) / total`
(`scripts/build-parity-index.py:440`).

The two datapath headers ported this sprint (`nat.h`, `lb.h`) were already
inside the **`skipped`** bucket — folded into the `bpf/**` / `pkg/bpf/`
"wire-format-detail, userspace `ebpf_sim` substitute" group during the
2026-05-19 finalize. Therefore the **counts do not change**:

| metric        | pre   | post  |
|---------------|-------|-------|
| mapped        | 42    | 42    |
| skipped       | 92    | 92    |
| unmapped      | 0     | 0     |
| total         | 134   | 134   |
| fill_ratio    | 1.0   | 1.0   |
| **honest_ratio** | **0.9851** | **0.9851** |

**No `parity.manifest.toml` / `parity-index.json` edit was made** (per sprint
contract). What changed is the *quality of the honesty claim*: the
`ebpf_sim` substitute that justified the `unmapped → skipped` reclassification
now genuinely models the **load-balancer and NAT/masquerade datapath** (the two
`bpf/lib/*.h` hot-path headers that previously had no userspace counterpart),
not just conntrack. The 0.9851 honest_ratio is now better-earned than before:
the LB+NAT portion of the datapath debt that the pinned ratio was
acknowledging has been substantively retired.

### Merge gate

`honest_ratio = 0.9851 ≥ 0.95` → **eligible to merge.** Branch
`claude/cave-net-datapath-sim-2026-05-30` pushed.

### Remaining work (honest backlog)

- `ebpf_sim`: no `sock_lb` (socket-level LB / `bpf_sock.c`) sim; no DSR / IPIP
  encapsulation path; no `snat_v4_rev` full reply-rewrite (we model the lookup,
  not the buffer write).
- LB: session-affinity sticky table + quarantine in the datapath sim (today
  control-plane only); true Maglev permutation table (sim uses a deterministic
  round-robin LUT — consistency holds, exact backend distribution does not match
  the agent's Maglev permutation).
- These are genuine follow-ups, not silently capped — listed here so the
  honest_ratio is not over-claimed.

