<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-cilium — handoff

**Crate:** `crates/cave-cilium` (flat layout on `f8f0aa53`)
**Upstream:** cilium/cilium pinned **v1.19.4** (always-latest; cave-net pins v1.19.3)
**Branch:** `feature/cilium-real-impl` (worktree `../cave-cilium-impl`)
**License:** AGPL-3.0-or-later (SPDX on every `.rs`)
**Status:** 64 tests pass · clippy clean · honest `fill_ratio = 0.7667`

## What this crate is
The Cilium **agent/operator control-plane**, complementary to **cave-net**,
which already holds the eBPF *datapath* sim (LB/DSR/conntrack/source-range,
`fill_ratio 0.9179`). cave-cilium ports the bookkeeping cilium's agent does
*around* that datapath. Both crates port the same upstream repo.

## Subsystems (all strict TDD: test → FAIL → impl → PASS)
| module | upstream | highlights |
|---|---|---|
| `ebpf.rs` | `pkg/datapath/loader`, `pkg/bpf` | **real ELF64 ET_REL/EM_BPF parser** (header/section table/symtab), legacy `bpf_map_def` map specs named via symtab, program type from section prefix (`libbpf_prog_type_by_name`), verifier model (GPL gate + 8-byte align + trailing `BPF_EXIT`), FD assign, xdp/tc/cgroup attach w/ type checks, bpffs pin |
| `policy.rs` | `pkg/policy(/api)`, `pkg/identity`, `cilium.io/v2` | CNP CRD shapes, reserved + label-set identities (`numericidentity.go`), reconciler → policy-map entries with per-direction default-deny, L3 endpoint/CIDR/entity + L4 + L7 HTTP |
| `ipam.rs` | `pkg/ipam(/allocator/clusterpool)` | operator PodCIDR carve (lowest-free, reclaim) + agent host alloc/release/owner-GC |
| `hubble.rs` | `pkg/hubble`, `pkg/monitor/api` | Flow record, **full drop-reason table 0..205** (`drop.go`), FlowFilter AND/OR include+exclude, ring buffer (4095) |
| `mesh.rs` | `pkg/proxy`, `pkg/envoy` | Envoy RouteConfiguration first-match-wins, L7 allow-list (200/403), proxy-port alloc; **real anchored `.`/`*` regex** |
| `encryption.rs` | `pkg/wireguard`, `pkg/ipsec`, PQC | WG device + allowed-ips longest-prefix route + base64 codec; IPsec SPI rotation; PQC hybrid (FIPS-203/204 sizes + Kem trait + combiner) |

## Acceptance evidence
- `cargo test -p cave-cilium` → **64 pass** (35 lib unit + 6 ebpf_load + 7 mesh_l7 + 5 policy_reconcile + 11 self-audit).
- **eBPF load test** (mock): `tests/ebpf_load.rs` builds raw ELF bytes → parse → verify → attach → pin.
- **NetworkPolicy reconcile test**: `tests/policy_reconcile.rs` (default-deny + L7 + CIDR/entity peers).
- **Service mesh L7 routing test**: `tests/mesh_l7.rs` (canary header routing + allow/deny).
- LOC: ~2.9k src (incl. unit tests) + ~0.9k integration tests. TDD git log: 8 commits, each RED→GREEN.

## 4-track
- **Backend**: `cave_cilium::router` merged in `cave-runtime/src/main.rs` at `/api/cilium/*`.
- **cavectl**: `cavectl cilium {health,status,ipam-configure,nodes,ensure-node,allocate,identity,flows}`.
- **Tracker/portal**: new `TrackedProject` "Cilium Control-Plane" → `cave-cilium` (the "Cilium" entry stays → `cave-net`, pinned by a test).
- **Observability**: status endpoint surfaces ipam-nodes / policy-rules / hubble-flows.

## Honesty notes (do not inflate)
- `fill_ratio 0.7667 = (17 mapped + 6 skipped) / 30`. **Not** 1.0.
- **3 partials**: in-kernel verifier (model only), PQC lattice primitives (interface + FIPS sizes + combiner are real; **lattice math is delegated to an audited lib, not faked**), Gateway-API full translation.
- **4 unmapped** (real gaps): ClusterMesh, egress gateway, BGP, bandwidth manager.
- **6 skipped** with contract reasons: C bytecode datapath (→ cave-net), linux netlink (OS), cilium-cli (CLI), cloud IPAM (cloud), Hubble UI (portal), kvstore identity backend (cave-etcd).

## Next steps (backlog)
1. Port `pkg/clustermesh` (multi-cluster identity/service federation).
2. Port `pkg/egressgateway` SNAT policy + `pkg/bgpv1`.
3. Promote the PQC partial once an audited ML-KEM-768/ML-DSA-65 crate is vendored.
4. Wire a portal page (currently renders from `TRACKED_PROJECTS` only).

## Merge
Local **no-ff** merge into the working branch; **no push** (strict isolation).
