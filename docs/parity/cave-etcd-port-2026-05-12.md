# cave-etcd parity — 2026-05-12 audit refresh

**Author:** Real-parity Paket D close-out
**Branch:** `claude/gracious-banach-9be8eb` → merging to `main`
**Upstream pin:** `etcd-io/etcd v3.6.10` (Apache-2.0)

## 1. Why this exists

The `2026-05-01` full-audit doc placed cave-etcd at **tier 100** with
`parity_ratio = 1.0`. That number was the wave3 mechanical
self-reported ratio: every entry in the manifest's `[[files]]` /
`[[functions]]` / `[[tests]]` / `[[surfaces]]` arrays mapped to a
local source-tree symbol, so `matched / total` came out to 1.0 — but
the manifest only declared **13 upstream files** out of an etcd
codebase that ships dozens of packages. The metric was true under its
own definition, and misleading as a parity claim.

Per the close-out brief ("**gerçek line-by-line port**"), this pass
replaces the self-reported number with a measured one: enumerate every
non-trivial top-level etcd v3.6.x package, classify each as
mapped / skipped / unmapped, and land one real port (the WAL) so the
ratio reflects the new work rather than just the relabeling.

## 2. Inventory methodology

Source: hand-curated against the etcd v3.6.x repository layout
(`server/`, `client/`, `api/`, `raft/`, `pkg/`, `etcdctl/`,
`etcdutl/`, `contrib/`, `tools/`, `tests/`, `functional/`). The
inventory targets **observable surface**, not file count — minor
helper sub-packages were folded into their parent so a single
"server/auth/" entry stands in for the whole auth surface.

Per cave-net's pattern (the gold-standard upstream-inventory
manifest at 134 entries with `fill_ratio = 1.0`), each entry is one
of:

- **`[[mapped]]`** — cave-etcd has at least one source file
  implementing the package's observable contract. Notes call out
  reshape choices (e.g. JSON-over-HTTP instead of gRPC for the v3rpc
  surface, in-memory backend instead of bbolt).
- **`[[skipped]]`** — out of scope per Charter. Allowed reasons are
  enumerated (`go-bootstrap` | `proxy-mode` | `CLI` | `test-harness` |
  `wire-format-detail` | `v2-only` | `parallel-track` |
  `stdlib-analog`). Every skip cites one.
- **`[[unmapped]]`** — real port gap, acknowledged with rationale.

## 3. Counts

| Bucket   | Count | Notes |
|----------|------:|-------|
| Mapped   | 30 | Includes the WAL port newly landed in this PR |
| Skipped  | 35 | Charter-justified out-of-scope |
| Unmapped | 6 | Honest gaps |
| **Total** | **71** | |
| **fill_ratio** | **0.9155** | (mapped + skipped) / total |

The previous self-reported `parity_ratio = 1.0` is replaced by
`fill_ratio = 0.9155` in the manifest's `[parity]` block. The disk
overlay in `scripts/build-parity-index.py` will surface the new value
on the next index rebuild.

## 4. The WAL port (only real net-new code in this PR)

`crates/cave-etcd/src/wal.rs` — 459 LOC + 16 unit tests + 8
integration tests in `crates/cave-etcd/tests/wal_replay.rs`.

Mirrors etcd's `server/storage/wal/` in shape, with deliberate
divergences called out in the module header:

| etcd v3.6 | cave-etcd MVP | Reason |
|-----------|---------------|--------|
| `walpb.Record { type, crc, data }` protobuf | JSON-framed records | Forensic readability under a text editor / `jq` during the single-node MVP. Protobuf compatibility tracked as one of the six `[[unmapped]]` entries. |
| `<seq:016x>-<index:016x>.wal` segment files, 64 MiB cut | Single `wal.log` file | Rotation is a follow-up. The current design fits the cluster-runtime's single-binary boot path. |
| `metadataType` / `entryType` / `stateType` / `crcType` / `snapshotType` | `WalRecord::{Metadata, Entry, State, Snapshot}` | `crcType` collapsed into the per-record CRC32 header rather than a separate record type. |
| `wal.Save` does write + fdatasync | `Wal::append` / `append_entry` does write + flush + sync_data | Same guarantee. |
| `wal.ReleaseLockTo` discards segments below a snapshot | `Wal::truncate_through` rewrites the single file atomically (write tmp, fsync, rename) | Same semantic; single-file MVP. |

### WAL ↔ KvStore replay glue

`cave_etcd::wal::replay_into_store(&Wal, &KvStore)` translates every
`WalOp` (Put / Delete / Txn / Compact / LeaseGrant / LeaseRevoke)
into the corresponding `KvStore` mutation. The 8 integration tests in
`tests/wal_replay.rs` exercise this end-to-end:

- `wal_replay_reconstructs_simple_put_state`
- `wal_replay_handles_overwrite_correctly`
- `wal_replay_drops_deleted_keys`
- `wal_replay_expands_txn_into_constituent_ops`
- `wal_replay_restores_lease_grant_and_revoke`
- `wal_survives_crash_between_appends`
- `wal_replay_handles_range_delete`
- `wal_replay_under_truncate_preserves_observable_state`

The WAL is **not** wired into the live mutation path inside
`KvStore::put` / `delete_range` / `lease_grant` etc. — that
orchestration belongs in `cave-runtime::cluster_runtime` alongside
the existing snapshot loop, where it can be sequenced with the TLS
listener and SIGINT-flush hook landed in commit `cd7b1f37`. The
replay helper exposed here is the boot-path side of that integration;
the write-path side is tracked as the next deliverable in this
sequence.

## 5. Honest gaps (the 6 `[[unmapped]]`)

| Package | Why it's unmapped |
|---------|-------------------|
| `server/storage/wal/walpb/` | Protobuf record wire-format; JSON used in MVP, protobuf needed for multi-node Raft compat with etcd peers (if/when we federate). |
| `server/etcdserver/api/v3election/` | Election RPC surface. Primitives exist in `concurrency.rs`; top-level endpoint not exposed. |
| `server/etcdserver/cindex/` | Consistent-index helper — coupled to raft, will land with Paket C. |
| `server/storage/quota/` | Standalone quota module. Db-size-bytes alarm exists in `maintenance.rs`; per-tenant quota enforcement is a gap. |
| `etcdutl/` | Offline data-dir surgery utility (backup/restore/defrag without a running server). Tracked as a future `cavectl etcd-offline` subcommand. |
| `server/lease/leasehttp/` | HTTP transport for cross-peer lease-keepalive. Not needed by single-node MVP. |

## 6. Test surface delta

| Pass | Tests | Notes |
|------|------:|-------|
| Before | 867 | All pre-existing |
| After  | **891** | 16 new WAL unit tests + 8 new WAL integration tests |

`cargo test -p cave-etcd` — **891 passed, 0 failed, 0 ignored**.

## 7. What this PR does NOT claim

- **It does not claim the wave3 ratio was wrong** — the calculator's
  `matched / total = 1.0` was correct against the manifest's
  declared surface. The change is **expanding the surface**, which
  honestly drops the ratio.
- **It does not claim cave-etcd reaches 1.0 against the new
  inventory** — six packages are explicitly `[[unmapped]]`. The
  measured `fill_ratio = 0.9155` is the new ground truth.
- **It does not wire WAL into the live KvStore mutation path** —
  that's a separately-scoped follow-up. The pieces (Wal type,
  WalRecord/WalOp shapes, replay glue, KvStore re-application path)
  are all here and exercised by tests; the integration is a
  documented next step, not a "stub".
- **It does not touch raft/ or cluster_runtime** — Paket C owns that
  track; this PR stays clear of file overlap.

## 8. Follow-ups (sized roughly)

1. **WAL integration into `cluster_runtime`**: open WAL alongside
   the existing snapshot loop, replay on boot, append on every
   KvStore mutation, truncate on snapshot. ~150 LOC + tests.
2. **WAL file rotation**: cut new file at 64 MiB, segment naming.
   ~100 LOC + tests.
3. **Protobuf record format**: swap JSON-framed records for the
   etcd-faithful `walpb.Record` shape once multi-node Raft is live.
4. **Election RPC endpoint**: expose `concurrency::Election` over
   the v3rpc surface. ~80 LOC + tests.
5. **Per-tenant quota enforcement**: port `server/storage/quota/`.
   Coupled with cave-core tenant scoping.
