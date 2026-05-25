# cave-cache — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-19
**Primary upstream**: `valkey-io/valkey @ 8.0.0` (BSD-3-Clause, C) — Linux Foundation fork of Redis 7.2.4
**Legacy upstream**: `redis/redis @ 7.2.0` (BSD-3-Clause; SSPL from 7.4 — *not* tracked)
**Crate root**: `crates/cave-cache/`

## Scope

cave-cache is a Rust reimplementation of the Valkey/Redis server surface:

- RESP2 + RESP3 wire protocol on TCP (default port 6379)
- All data types — strings, lists, hashes, sets, sorted-sets, streams,
  hyperloglog, bitmap, geo
- pub/sub
- ACL (rule-based authentication)
- Scripting (server-side Lua-compatible interpreter)
- Transactions (MULTI/EXEC/DISCARD/WATCH)
- Expiry (active + lazy)
- Eviction (LRU/LFU/random)
- Persistence (AOF + RDB)
- Cluster slot routing
- Side REST surface under `/api/cache/*` for cave-runtime + cave-portal

## License posture

We track **Valkey 8.x** (BSD-3-Clause, maintained by the Linux Foundation)
rather than `redis/redis` since Redis Inc. relicensed Redis 7.4+ to
RSALv2/SSPL in 2024. SSPL is forbidden by the workspace `deny.toml`.
Valkey 8.0 was forked from the last BSD-3 Redis release (7.2.4), so the
file mappings remain accurate.

## Inventory measurement

Hand-curated against the Valkey 8.0.0 source layout
(`src/{server,networking,db,t_*,replication,cluster,scripting,
expire,aof,rdb,acl,connection,eviction,modules}.c`).

| Bucket   | Count | Examples                                                                            |
|----------|------:|-------------------------------------------------------------------------------------|
| Mapped   |    21 | server, networking (resp), db, expire (expiry), evict (eviction), aof, rdb, acl,    |
|          |       | scripting, cluster, t_string, t_list, t_hash, t_set, t_zset, t_stream, t_hll,       |
|          |       | t_bitmap, t_geo, **CLUSTER FAILOVER takeover state machine**, **ACL log persistence** |
| Partial  |     4 | replication (PSYNC2 framing, no diskless sync), cluster slot fan-out (single-node   |
|          |       | resolver), scripting (no `EVALSHA` cache eviction policies), modules (load/init     |
|          |       | hook only — no full Module API surface)                                              |
| Skipped  |    13 | sentinel, RedisGears, RediSearch, RedisJSON, RedisTimeSeries, RedisBloom, RedisAI,  |
|          |       | RedisGraph, monitor command, debug subsystem, latency profile, IO threads (libuv    |
|          |       | path), TLS (deferred to cave-mesh proxy)                                             |
| Unmapped |     0 |                                                                                     |
| **Total**|  **38** | |

- **fill_ratio  = (mapped + partial + skipped) / total = 38 / 38 = 1.0000**
- **honest_ratio = (mapped + skipped) / total             = 34 / 38 = 0.8947**

### 2026-05-19 c-tier uplift

`CLUSTER FAILOVER` takeover state machine and the persistent `ACL LOG`
audit tail were promoted **unmapped → mapped**:

- `src/cluster/failover.rs` — `FailoverState` covers all three operator
  variants (graceful / `FORCE` / `TAKEOVER`), quorum-driven auth ACK
  promotion, epoch bump on promotion, and timeout-driven failure with a
  recorded reason.
- `src/acl_log.rs` — `AclLog` keeps a capacity-bounded ring buffer with
  optional on-disk JSONL backing. Append is persisted synchronously,
  reload trims to capacity newest-first, `ACL LOG RESET` truncates both
  the in-memory and the disk view. Escape/unescape round-trip handles
  tabs and newlines in usernames / object identifiers.

## 8-gate close-out

| # | Gate                              | Result | Evidence                                  |
|---|-----------------------------------|--------|-------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | 51/51 `src/**/*.rs` carry AGPL-3.0-or-later |
| 2 | `source_sha` pinned in manifest   | PASS   | `source_sha = "8.0.0"` (Valkey 8.0.0)     |
| 3 | `last_audit = "2026-05-19"`       | PASS   | `[parity].last_audit`                     |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly  |
| 5 | `fill_ratio >= 0.90`              | PASS   | 1.0000 (≥0.95 ctier-uplift target met)   |
| 6 | mapped + partial + skipped + unmapped == total | PASS | 21 + 4 + 13 + 0 = 38       |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/`                |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                 |

**Charter v2 verdict: 8/8 PASS.**

## Test coverage

`cargo test -p cave-cache --lib --tests` exercises:

- 3 integration test suites (`tests/*.rs`)
- 9 close-out self-audit assertions (`tests/parity_self_audit.rs`)

## Next sweep (out of this close-out)

Both former unmapped items landed in the 2026-05-19 c-tier uplift.
`unmapped_count = 0`; remaining gap to a true 1.0 honest_ratio is the
four `partial` subsystems (PSYNC2 diskless, scripting `EVALSHA` cache,
modules API beyond load/init, single-node cluster fan-out) — deferred
to obs-stack-ray-2.
