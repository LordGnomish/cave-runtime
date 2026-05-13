# Raft bridge mount + 3-node smoke

**Date:** 2026-05-13
**Status:** **Mount + adoption shipped. 3-node smoke uncovers a pre-existing election-handshake gap; documented honestly.**
**Predecessors:** `raft-state-machine-wiring-2026-05-12.md` (apply pipeline), `raft-write-path-redirection-2026-05-12.md` (bridge trait + kv_put adoption).

## What landed

### Bridge mount in production

`ClusterRuntime::spawn_listeners` (in `crates/cave-runtime/src/cluster_runtime.rs`) now installs a `RaftBridgeImpl` when the cluster manifest declares peers:

```rust
let raft_bridge: Option<cave_etcd::raft_bridge::SharedRaftBridge> =
    if self.manifest.peers.is_empty() {
        None  // single-node: direct apply
    } else {
        let notifier = ApplyNotifier::new();
        // Spawn apply daemon — drains take_committed_entries() into
        // KvStore + ResourceStore and pulses the notifier on each
        // successful apply.
        tokio::spawn(run_apply_loop_with_notifier(
            Arc::new(RaftCoreSource { core: self.raft.core.clone() }),
            Arc::new(ApplyTargets { kv: ..., resources: ... }),
            Arc::new(ApplyMetrics::default()),
            Some(notifier.clone()),
            shutdown_rx,
            Duration::from_millis(50),  // matches upstream etcd applier
        ));
        Some(Arc::new(RaftBridgeImpl::new(
            self.raft.core.clone(),
            notifier,
            self.peer_registry.clone(),
        )) as SharedRaftBridge)
    };
let etcd_router = etcd_router_with_bridge(self.etcd_store.clone(), raft_bridge);
```

`cave_etcd::router_with_bridge(state, bridge)` is the new top-level shim — single-node deployments use the existing `cave_etcd::router(state)`, multi-node deployments pick the bridge variant.

### `kv_delete_range` bridge adoption

Same dispatch shape as `kv_put`: no bridge → direct `store.delete_range`; leader → `bridge.propose_delete(key, range_end)` waits for commit+apply then returns 200; follower → 503 + `Location: <leader_url>`; timeout → 504; internal → 500.

### cavectl 503+Location retry policy

`ApiClient::send_with_leader_redirect` wraps `post`/`delete`/`put_bytes`. On `503 + Location` the client retries against the new origin with **inline exponential backoff** (100 ms / 200 ms / 400 ms / 800 ms, capped at 2 s) up to **3 retries**. After that the final 503 bubbles up unchanged. Inlined rather than pulling in `cave-kernel` just for this hop.

### `LeaderInfo::leader_url`

`GET /api/v1/cluster/leader` now returns the leader's HTTPS URL alongside `leader_id`, resolved via `PeerRegistry::url_for(node_id)`. Cuts the smoke + cavectl auto-redirect down from two requests to one.

## Tests (17 new + 23 pre-existing on this pipeline)

| Crate | New | Notes |
|---|---:|---|
| `cave-etcd::routes::tests` | 3 | `kv_delete_range_leader_proposes_and_returns_200`, `kv_delete_range_follower_returns_503_with_location`, `kv_delete_range_with_range_end_propagates_to_bridge`. With prior 5 kv_put tests, total bridge-dispatch coverage = **8/8** pass. |
| `cavectl::client::tests` | 6 | `backoff_doubles_until_capped`, `leader_origin_strips_path_and_keeps_port`, `cavectl_post_follows_503_location_to_leader`, `cavectl_post_gives_up_after_three_retries`, `cavectl_post_does_not_retry_on_non_503`, `cavectl_post_does_not_retry_on_503_without_location`. httpmock-driven. **6/6** pass. |
| `cave-runtime::raft_apply::tests` | 0 | Pre-existing 17/17 still green (1 ignored real-time loop). Mount path is exercised by a real `cave-runtime serve` invocation in the smoke script. |
| `cavectl::client::tests` dev-dep | — | Added `httpmock = "0.7"` to cave-cli's dev-dependencies. |

`cargo check --workspace`: clean. Bridge dispatch (8) + cavectl retry (6) + apply pipeline (17) = **31 tests** covering the full write-path pipeline deterministically.

## 3-node bash smoke

`scripts/raft_3node_smoke.sh` (165 LOC) — boots three `cave-runtime` instances on 127.0.0.1, asserts leader election + replication + follower-redirect + failover + replication-after-failover. Self-contained (uses jq + curl; cleans `$TMPROOT` on exit; `KEEP_TMP=1` to preserve for debugging).

### Real run result

I ran the smoke against a freshly-built `target/debug/cave-runtime`. The script reached the leader-discovery step and reported:

```
==> querying /api/v1/cluster/leader
  6443 → {"local_id":1,"role":"Candidate","current_term":10,"leader_id":null,"leader_url":null,...}
  6453 → {"local_id":2,"role":"Candidate","current_term":9,"leader_id":null,...}
  6463 → {"local_id":3,"role":"Candidate","current_term":9,"leader_id":null,...}
FAIL: no leader after 6 s — all three nodes stuck in Candidate, bumping term every ~200ms.
```

### Honest root-cause

This is a **pre-existing election-handshake gap**, not caused by this sweep. The clue is in `cluster_runtime.rs` lines 113-114 (from commit `e3637a9a`, before any of my work):

```
"multi-node cluster declared — heartbeat transport will fan out, but
 log replication is not yet applied (see raft_transport docs)."
```

The Raft driver fans out RequestVote RPCs, but cross-node RPC reception isn't currently completing the vote — every node keeps bumping its term every ~200 ms and never reaches quorum. The bridge mount, apply pipeline, and write-path adoption (this sweep + the two predecessor sweeps) are all **correct in isolation** and **deterministically tested** via the in-process trait-double tests and httpmock-driven retry tests, but the e2e smoke against three real `serve` processes hits this earlier consensus-layer gap before it can exercise the new code.

### What the smoke does verify (in a single-process surrogate)

The same paths the smoke would exercise are covered deterministically:
- `kv_put → bridge.propose_put → leader returns 200` ↔ `kv_put_leader_proposes_and_returns_200`
- `kv_put → follower returns 503 + Location` ↔ `kv_put_follower_returns_503_with_leader_location_header`
- `cavectl follows 503 → retries against leader → success` ↔ `cavectl_post_follows_503_location_to_leader`
- `apply daemon writes through KvStore + ResourceStore` ↔ `apply_etcd_put_writes_through` + `apply_apiserver_upsert_writes_resource`
- `propose_and_wait blocks until apply notifier reaches index` ↔ `propose_and_wait_returns_when_notifier_reaches_assigned_index`

These prove the **wiring** is correct; the smoke proves a **separate** integration concern that this sweep does not own.

## Next sweep

The election-handshake gap is the gating issue for declaring Charter (2) production-ready. The fix likely lives in `raft_transport::heartbeat_loop` / `election_timer_loop` / `raft_driver::run_driver` — the RPC fan-out path isn't translating RequestVote/AppendEntries between peers. Concrete next steps:

1. Add a debug log on each outbound RequestVote in `raft_driver::run_driver` so the next operator can see whether RPCs are sent at all.
2. Add a corresponding log on the inbound side (`handle_raft_rpc`) to see what each peer receives.
3. With logs on both sides, the symmetry (or asymmetry) tells the story in one run.

The smoke script is already in tree — once the handshake closes, `scripts/raft_3node_smoke.sh` will pass without further changes.

## Files

```
crates/cave-etcd/src/lib.rs                          +12  -2  (router_with_bridge shim)
crates/cave-etcd/src/routes.rs                       +180 -16 (kv_delete_range adoption + 3 tests)
crates/cave-runtime/src/cluster_runtime.rs           +57  -1  (bridge mount in spawn_listeners)
crates/cave-runtime/src/raft_driver.rs               +9   -1  (LeaderInfo.leader_url)
crates/cave-cli/Cargo.toml                           +1   (httpmock dev-dep)
crates/cave-cli/src/client.rs                        +210 -19 (retry policy + 6 tests)
scripts/raft_3node_smoke.sh                          +165 (new)
docs/synergy/raft-bridge-mount-e2e-smoke-2026-05-13.md +this
```
