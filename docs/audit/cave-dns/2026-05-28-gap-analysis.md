<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-dns ↔ CoreDNS v1.14.3 — Gap Analysis (2026-05-28)

Upstream: `github.com/coredns/coredns` @ `v1.14.3`. Crate: `crates/networking/cave-dns`.

## 1. Upstream scale & why raw-LOC parity is unreachable
156,243 src LOC over 52 plugins; ~124k of that is in `plugin/`. A literal
`cave_LOC / upstream_LOC` ratio is ~0.07, but that is **not** the parity model:
cave-dns is scored on pure-DNS feature coverage (was 0.9583). CoreDNS's bulk is
k8s/etcd/route53/azure/clouddns/dnstap/trace/traffic — all architecturally
scoped to **other** cave crates (cave-net, cave-etcd, cloud crates, cave-trace,
cave-gateway), so re-homing them into cave-dns would violate crate boundaries.

## 2. cave-dns is built on hickory-proto 0.24
Records are `hickory_proto::rr::Record` carrying `RData`; EDNS lives in
`protocol::edns`. (`src/types.rs`, `src/models.rs`, `src/message.rs` exist on
disk but are NOT in `lib.rs` — dead/legacy, not imported.)

## 3. This ray — strict-TDD closes (against the real hickory model)
| feature | module | notes |
|---------|--------|-------|
| `bufsize` | `bufsize.rs` | EDNS size policy `[512,4096]` + clamp-down |
| `acl` | `acl.rs` | source-CIDR (v4/v6)+qtype, first-match, Block=REFUSED / Filter=NOERROR+TC |
| `dns64` | `dns64.rs` | RFC 6052 §2.2 A→AAAA over hickory `Record`; RFC 6147 trigger |
| `transfer` IXFR | `ixfr.rs` | RFC 1995 §4 diff sequence + AXFR fallback — closes transfer partial |
| `secondary` refresh | `secondary_refresh.rs` | RFC 1035/1982 refresh/retry/expire FSM — closes secondary partial |

Each lands as a RED→GREEN cycle: a test commit (module absent, fails to compile)
then an impl commit (module + `pub mod`). Git log shows every `test(...)` before
its `feat(...)`.

## 4. Honestly deferred
`minimal` (minimal-responses) acts on a full DNS message and belongs in the
plugin dispatch path, not as a standalone core. k8s/etcd/cloud/dnstap/trace/
traffic are out-of-crate by architecture, credited to their owning crates.

## 5. honest_ratio disposition
These are tested library cores, not yet wired into `server.rs`/`plugins.rs`
dispatch or the transfer handler. Per the ray mandate (no adr-justify, no
manifest manipulation), the parity manifest and `parity-index.json` are left
unchanged: marking features "mapped" before they are reachable in the running
server would be inflation. The honest next step is to wire the cores into
dispatch and only then update the manifest.
