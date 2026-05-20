# cave-etcd — etcd v3 parity report

Pinned upstream: **etcd-io/etcd @ v3.6.10** (`source_sha = "v3.6.10"`)
Audit landed: 2026-05-12 · Charter v2 FINALIZE: 2026-05-18 · UPLIFT: 2026-05-19

This document is the honest companion to `parity.manifest.toml`. The
manifest proves *coverage*; this report describes *fidelity* — which
upstream packages are wire-faithful, which are semantic-only, and what
remains for follow-up sprints.

---

## TL;DR

| metric | value |
|---|---|
| upstream packages enumerated | 71 |
| mapped | **31** |
| partial | 2 |
| skipped (Go-toolchain / browser-UI / cluster-bootstrapper) | 35 |
| unmapped (acknowledged real port gaps) | **3** |
| `fill_ratio` (mapped + partial + skipped) / total | **0.9577** (measured) |
| `honest_ratio` (mapped + skipped) / total | **0.9296** |
| cave-etcd `.rs` files | 40 |
| SPDX AGPL-3.0-or-later coverage | **40/40 (100 %)** |
| `unimplemented!()` / `todo!()` / `panic!("not …")` | **0** |
| `#[deprecated]` | **0** |
| `#[test]` + `#[tokio::test]` | 937+ |
| release build | clean |

---

## Charter v2 8-gate scoreboard

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | TDD-strict (RED→GREEN→REFACTOR) | ✅ | this branch shape |
| 2 | SPDX AGPL coverage 100 % | ✅ | `tests/parity_self_audit::every_rs_file_carries_agpl_spdx` |
| 3 | `source_sha` upstream pin | ✅ | `[upstream] source_sha = "v3.6.10"` |
| 4 | No stubs | ✅ | grep count 0 |
| 5 | No back-compat | ✅ | grep count 0 |
| 6 | Latest upstream pinned | ✅ | etcd v3.6.10 = current stable line |
| 7 | 4-track full | ✅ | see "4-track green status" below |
| 8 | Honest measured manifest | ✅ | `fill_ratio = 0.9577` from `mapped+partial+skipped`/total enumeration |

All 8 gates: **PASS**.

---

## 4-track green status

| Track | Surface | Pre-close status |
|---|---|---|
| Backend lib | `crates/cave-etcd/src/{kv,watch,lease,txn,maintenance,wal,…}.rs` | 913 tests pass |
| Portal | `cave-portal/src/admin/etcd/` | live wired (etcd metrics, db-size, member list) |
| cavectl | `EtcdCmd` (put/get/del/lease/snapshot/defrag/…) | parse-tests green |
| Observability | `cave-etcd` alert group + Grafana panel | rules + JSON committed |

---

## Unmapped surface (honest scope-cut)

The 3 [[unmapped]] rows in the manifest are real port gaps:

| upstream package | reason | follow-up |
|---|---|---|
| `server/storage/wal/walpb/` | etcd's protobuf record wire format. cave-etcd's WAL uses JSON-framed records for forensic readability in the single-node MVP; protobuf shape is a follow-up for multi-node Raft compatibility. | walpb record format |
| `etcdutl/` | Offline data-dir surgery utility (backup-restore, defrag without server). | separate cavectl subcommand |
| `server/lease/leasehttp/` | HTTP transport for lease-keepalive between peers. Single-node MVP doesn't need it; multi-node will. | peer transport batch |

### Promoted to [[mapped]] in 2026-05-19 uplift

| upstream package | local file | notes |
|---|---|---|
| `server/etcdserver/api/v3election/` | `src/election.rs` (+ `src/concurrency.rs`) | v3rpc election façade — campaign / proclaim / leader / resign / observe / subscribe, keyed by name. 16 tests. |
| `server/storage/quota/` | `src/quota.rs` | `BackendQuota` — workspace db-size-bytes (2 GiB default) + per-tenant byte and key caps with `QuotaDecision` variants matching etcd's `Quota.Available` ordering. 15 tests. |
| `server/etcdserver/cindex/` | `src/cindex.rs` | `ConsistentIndex` atomic (index, term) pair with 16-byte big-endian on-disk format, `unsafe_save`/`unsafe_load` tx round-trip, FNV-1a hash for snapshot integrity. 13 tests. |

The 2 [[partial]] rows are `v3alarm/` (we have the alarm-emit path
but not the full GET/POST/DEACTIVATE RPC surface) and `v3compactor/`
(routes-level compaction; the periodic-compactor goroutine equivalent
runs but is not configurable from a `[compactor]` config block).

---

## What changed in the 2026-05-19 UPLIFT

Three [[unmapped]] rows promoted to [[mapped]] via real-impl ports:

  * `src/election.rs` — `ElectionService` keyed by election name with
    `campaign`, `proclaim`, `leader`, `resign`, `observe`, `subscribe`
    and `expire_lease`. Backed by the existing `DistElection` primitive
    in `src/concurrency.rs`. Per-name table + `tokio::sync::broadcast`
    leader-change channel.
  * `src/quota.rs` — `BackendQuota` with workspace db-size cap
    (2 GiB default, matching upstream) and per-tenant byte / key caps,
    runtime-mutable via `RwLock`. `QuotaDecision` precedence mirrors
    etcd's `Quota.Available`.
  * `src/cindex.rs` — `ConsistentIndex` atomic (index, term) pair with
    16-byte big-endian on-disk record, `unsafe_save`/`unsafe_load` tx
    round-trip, FNV-1a snapshot hash, refuse-regress on writes.

Count delta: mapped 28→31, unmapped 6→3, fill_ratio 0.9155→0.9577,
honest_ratio 0.8873→0.9296. `.rs` files 36→40. Self-audit floors
bumped: FILL_FLOOR 0.90→0.95, MAPPED 28→31, RS_FILES 30→40.

## What changed in the FINALIZE

No code or count delta. The 2026-05-18 close-out added:

  * `[upstream] source_sha = "v3.6.10"` — reproducibility pin.
  * `[parity] last_audit = "2026-05-18"` — close-out date.
  * `tests/parity_self_audit.rs` — 9 deterministic assertions.

Behavioural depth, fill_ratio, and honest_ratio remained at their
measured 2026-05-12 baseline (which included the WAL port that lifted
cave-etcd from audit-doc 1.0 → measured 0.9155).
