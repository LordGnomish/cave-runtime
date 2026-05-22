# cave-cache parity — 2026-05-12 audit

**Upstream:** `redis/redis 7.2.0` (BSD-3-Clause). Note: this is
the last BSD-3-licensed Redis release; later versions use SSPL.

## Methodology

Standard cave-etcd pattern. Inventory enumerates the top-level
Redis source files in `src/` (Redis lives in one giant `src/`,
not packages, so I map per-file). cave-cache's
`src/commands/` directory mirrors Redis's `t_*.c` 1:1, so most
data-type packages are mapped cleanly.

## Counts

| Bucket   | Count |
|----------|------:|
| Mapped   | 18 |
| Skipped  | 13 |
| Unmapped | 7 |
| **Total** | **38** |
| **fill_ratio** | **0.8158** |

## What lands in the inventory

* **Mapped (18)** covers EVERY data type Redis ships (strings,
  lists, hashes, sets, sorted-sets, streams, bitmap, geo,
  hyperloglog), the core engine (server, networking RESP2/RESP3,
  db, expire), ACL, pub/sub, persistence (AOF + RDB), eviction,
  and Lua scripting.
* **Skipped (13)** covers bundled deps (Lua interpreter, jemalloc,
  linenoise, hiredis), CLI binaries (redis-cli, redis-benchmark,
  redis-sentinel, redis-check-rdb), C-stdlib analogs (sds.c, dict.c,
  anet.c, util.c), tests, build scripts.
* **Unmapped (7)** covers the honest gaps: cluster mode (gossip
  + slot routing), Sentinel HA, master-replica replication,
  loadable modules, native TLS, FUNCTION LOAD (Redis 7), and
  cluster_slot_stats.

## What this PR does NOT claim

* `fill_ratio = 0.8158` does NOT mean cave-cache is a drop-in
  Redis replacement. It claims 82% of Redis's source files are
  either covered (47%, all data types + core) or honestly skipped
  (34%, deps + CLI + stdlib glue).
* The 7 unmapped entries — particularly cluster mode + Sentinel
  + replication — are real production blockers. Single-node only
  today.
