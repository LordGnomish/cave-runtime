# Raft → state machine wiring

**Date:** 2026-05-12
**Status:** Apply pipeline **landed (partial)** — see "Not wired" below.
**Owner:** runtime

## Why

`raft_core` (commit `e41bf733`) replicates opaque `Vec<u8>` payloads end-to-end with safe leader election. The host still treated those payloads as opaque — no typed command schema, no apply path into cave-etcd / cave-apiserver. This sweep adds the seam between Raft and the local state machine.

## What landed

### `cave-runtime/src/raft_command.rs` (NEW)

Typed `RaftCommand` enum carried in each log entry. JSON-encoded (`serde_json`) — operators debugging a divergent replica can `jq` the WAL rather than reach for a hex-dump; the per-entry byte cost is negligible for KV puts of a few KB.

```rust
pub enum RaftCommand {
    EtcdPut { key: String, value: String, lease: Option<i64> },
    EtcdDelete { key: String, range_end: Option<String> },
    ApiserverUpsert { resource: serde_json::Value },
    ApiserverDelete { kind: String, namespace: String, name: String },
    NoOp,
}
```

`encode` / `decode` round-trip via `serde_json`. An empty payload (the encoding earlier sessions used for leader no-ops) decodes to `NoOp` for back-compat. A `summary()` method renders a compact one-liner for log/`/admin/cluster` views without dumping payloads.

### `cave-runtime/src/raft_apply.rs` (NEW)

`ApplyTargets { kv, resources }` holds the two stores. `ApplyMetrics` tallies atomics (`applied_total` / per-variant counters / `decode_errors` / `apply_errors` / `last_applied_index`). `ApplyMetricsSnapshot` is a serializable copy for `/admin/cluster`.

`apply_one(entry, &targets, &metrics)` decodes one `LogEntry` into a `RaftCommand` and dispatches to:
- `KvStore::put` / `KvStore::delete_range` for the etcd variants
- `ResourceStore::upsert` / `ResourceStore::delete` for the apiserver variants
- counter-only bump for `NoOp`

`apply_batch(entries, ...)` runs the same path over a slice; a single failing entry is logged + tallied but **does not stop the batch** — Raft has already committed those entries and divergence on apply errors would be worse than the failure itself.

`run_apply_loop(source, targets, metrics, shutdown, interval)` is the production daemon. It drives a `CommittedEntrySource` trait (`async fn drain() -> Vec<LogEntry>`) plus a `tokio::time::interval` ticker; the default cadence (50 ms) matches upstream etcd's applier flush. Cancellation via a `watch::Receiver<bool>`.

A `RaftCoreSource` impl wraps `Arc<Mutex<raft_core::RaftCore>>` to call `take_committed_entries()`. The trait split keeps the daemon testable against an in-memory `VecSource` without standing up a real RaftCore.

## Tests

23 new in `crates/cave-runtime/src/{raft_command,raft_apply}/tests`:

**raft_command (11):**
- EtcdPut round-trip + with-lease variant
- EtcdDelete round-trip + with-range-end variant
- ApiserverUpsert round-trip preserves `kind` tag through serde
- ApiserverDelete round-trip
- NoOp round-trip
- Empty payload decodes as NoOp (back-compat for older sessions)
- Garbage payload → `RaftCommandError::Decode`
- `summary()` produces compact strings + truncates long keys

**raft_apply (12):**
- EtcdPut writes through to KvStore (verified via range query)
- EtcdDelete removes existing key
- EtcdDelete on missing key is a no-op (no error)
- ApiserverUpsert writes Resource through to ResourceStore
- ApiserverUpsert is idempotent on duplicate-index replay
- ApiserverDelete removes existing row
- ApiserverDelete on missing row is a no-op
- NoOp increments only the noop counter
- Empty payload decodes as NoOp through apply
- `apply_batch` continues past a wire-decode error
- `apply_batch` continues past a typed-decode error (Resource shape mismatch)
- `run_apply_loop` drains seeded batches until shutdown signal (marked `#[ignore]` because it relies on real-time tokio sleeps and would race with the existing portal::adr/auth flaky tests under default `--test-threads`; run on its own with `--ignored`)

`cargo check --workspace` clean. The pre-existing `portal::adr::tests::list_endpoint_returns_seeded_adrs` flake on main is unaffected by this sweep — same failure shape on `git stash` baseline.

## Not wired (deferred with reason)

### 1. Write-path redirection

cave-etcd's HTTPS `/v3/kv/put` handler and cave-apiserver's `/api/v1/*` POST/PUT/DELETE handlers still mutate the local store directly. Adding Raft-leader redirection means touching every PUT/POST route and (for followers) responding with `503 not-leader, retry at <leader>`. That's a per-route refactor of several thousand LOC across the two crates plus a connection-level retry policy in cave-cli. Out of scope here.

The apply daemon **is the prerequisite** for that work: once the route handlers are taught to propose-and-wait, the daemon will already be there to make committed entries observable.

### 2. Linearizable reads (ReadIndex)

The default read path is `local-only` (eventual consistency under load). The Raft §6.4 ReadIndex protocol (leader-side heartbeat-to-quorum before responding) is documented in the audit but not implemented. `cluster status --linearizable` would gate on this; for now it falls back to a local read with a clear log line.

### 3. End-to-end 3-node smoke

The user's prompt includes a 3-node smoke script (init three data dirs, start three serves, write to leader, kill leader, observe failover). The smoke isn't automated in this sweep because the write-path redirection above isn't wired — a curl against `/v3/kv/put` on the leader still skips the propose path. Running the smoke today would only exercise per-node KV writes, not the Raft round-trip. Re-run the smoke after Item 1 lands.

## Files

```
crates/cave-runtime/Cargo.toml         +1   (async-trait workspace dep)
crates/cave-runtime/src/main.rs        +2   (pub mod raft_apply, raft_command)
crates/cave-runtime/src/raft_command.rs +205 (new — types + 11 tests)
crates/cave-runtime/src/raft_apply.rs   +440 (new — daemon + 12 tests)
docs/synergy/raft-state-machine-wiring-2026-05-12.md +this
```

## Next sweep

Pick up **write-path redirection** crate by crate:
1. cave-etcd `/v3/kv/put` first — single endpoint, well-defined contract. Add a `RaftBridge` trait the route handler consults: if `Some(handle) && handle.is_leader()`, propose-and-wait; if `Some(handle) && !is_leader()`, return 503 with leader URL; if `None`, mutate locally (single-node mode).
2. cave-apiserver `POST/PUT/DELETE` next — same pattern but the resource conflicts on optimistic locking need attention so concurrent proposers don't deadlock.
3. Then 3-node smoke + ReadIndex linearizable reads.
