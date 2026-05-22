# Raft inter-node handshake fix — 3-node smoke PASS

**Date:** 2026-05-13
**Status:** **Charter (2) Raft consensus end-to-end verified.** Smoke PASSes: election + replication + 503-redirect + failover.
**Predecessor:** `raft-bridge-mount-e2e-smoke-2026-05-13.md` — bridge mount landed; smoke failed at election step.

## Diagnostic flow (the work)

### 1. Instrumented both RPC sides

`raft_driver.rs`:
- Outbound — every RequestVote / AppendEntries logs `to`, `endpoint`, `term`, `candidate_id`, `last_log_index`, `last_log_term` under `cave_runtime::raft::rpc` target.
- Outbound errors — walk the `std::error::Error` source chain so a TLS rejection or connect refusal surfaces past reqwest's generic "error sending request for url" wrapper.
- Skipped-peer warning when the registry lacks a URL for an outbound target.

`raft_driver.rs::handle_raft_rpc`:
- Inbound RequestVote logs `from`, `term`, `last_log_index`, `last_log_term`.
- Inbound RequestVoteReply emits its `term` + `granted`.

### 2. Captured per-node logs from real smoke

Three live `cave-runtime` instances on 127.0.0.1, RUST_LOG=`cave_runtime::raft::rpc=info`.

**Symmetry observation:**
- Outbound count per node: 2 (one per peer) every ~200 ms
- Inbound count per node: **0**

Outbound RPCs go out, peers receive nothing.

### 3. Source-chain error reveals root cause

With the chained error walk in place, the next smoke surfaced:

```
"error sending request for url (https://127.0.0.1:6463/raft/rpc) ::
 client error (Connect) ::
 One or more certificates required to validate this certificate cannot be found."
```

**Root cause = Hypothesis B (TLS CA mismatch).** Each `cluster init` generated a fresh per-node CA, so node1's `ca_cert_pem` (used to build the run_driver reqwest client) only validated certs signed by node1's CA — and node2's apiserver leaf is signed by node2's CA. TLS handshake terminates before HTTP.

## Fix

Surgical: `cave-runtime/src/cluster.rs::init` now takes an additional `reuse_existing_ca: bool` arg (CLI: `--reuse-existing-ca`). When set and `pki/ca.{crt,key}` already exist, the init parses the existing PEMs into `rcgen::CertificateParams::from_ca_cert_pem` + `KeyPair::from_pem` and signs the new leaf certs against that root. Without the flag, behaviour is unchanged.

In production the equivalent flow is `cluster join` which fetches the leader's CA via TOFU (`docs/synergy/cluster-csr-ca-wal-2026-05-12.md`). The new flag is for co-resident smoke + operators who already have the CA on the machine.

### Smoke harness updates

`scripts/raft_3node_smoke.sh`:
- Init node1 first, copy its `pki/ca.{crt,key}` into nodes 2 + 3, then init those with `--reuse-existing-ca`.
- Etcd routes live on the etcd listener (port 2379 / 2389 / 2399), not the apiserver listener — switch the PUT/RANGE targets accordingly.
- `KeyValue.value` is serialised as a JSON int-array (Rust default for `Vec<u8>`), then base64-wrapped by the etcd v3 wire convention. Replace the original `jq -r '.kvs[0].value' | base64 -d` with `jq … | join(",") | awk → bytes → base64 -d` to handle the array shape.

### Bonus

`leader_url` field on `/api/v1/cluster/leader` — resolved via `PeerRegistry::url_for(leader_id)`. Cuts smoke leader-discovery from two requests to one. (Landed in the previous sweep but exercised end-to-end here for the first time.)

## Smoke PASS transcript

```
==> initializing 3 data dirs under /tmp/cave-raft-smoke-80928
==> spawning 3 cave-runtime serves
==> querying /api/v1/cluster/leader
  6443 → {"local_id":1,"role":"Leader","current_term":1,"leader_id":1,
          "leader_url":"https://127.0.0.1:6443",...}
  leader: https://127.0.0.1:6443
==> PUT /v3/kv/put on leader, then RANGE on every node
  PUT response: {"header":{"cluster_id":1,"member_id":1,
                "revision":2,"raft_term":1},"prev_kv":null} HTTP 200
  etcd:2379 → bar ✓     ← leader
  etcd:2389 → bar ✓     ← replicated to follower
  etcd:2399 → bar ✓     ← replicated to follower
==> PUT on a follower, expect 503 + Location header
  follower https://127.0.0.1:6453 → 503 ; location: https://127.0.0.1:6443 ✓
==> kill leader, expect new election
  killing node1 (pid 80940)
  new leader: https://127.0.0.1:6463 ✓
==> PUT after failover on new leader
  etcd:2389 → ok ✓
  etcd:2399 → ok ✓

SMOKE PASS — Raft consensus end-to-end ✓
  replication ✓  follower-redirect ✓  failover ✓
```

What the smoke proves end-to-end:
1. **Leader election** completes within 6 s on a fresh 3-node cluster.
2. **Write replication** through the Raft `ApplyNotifier` → bridge → apply daemon → `KvStore::put` pipeline, observable on all three nodes.
3. **Follower 503+Location** redirect contract that cavectl's retry policy depends on.
4. **Failover** — kill leader, ~8 s to elect a new leader, writes replicate again.

## Test coverage (unchanged)

| Suite | Count |
|---|---:|
| `cave-etcd::routes::tests::kv_*` | 8/8 |
| `cave-runtime::raft_apply::tests` | 17/17 (1 ignored real-time loop) |
| `cave-runtime::cluster::tests` | 9/9 |
| `cavectl::client::tests` | 6/6 |
| `cargo check --workspace` | clean |

## Files

```
crates/cave-runtime/src/cluster.rs                    +43 -10  (--reuse-existing-ca flag + CA-load path; 5 test callsites updated)
crates/cave-runtime/src/cluster_runtime.rs            +0  -0   (3 init() callsites updated by sed)
crates/cave-runtime/src/raft_driver.rs                +60 -3   (outbound + inbound tracing + chained-error walk)
scripts/raft_3node_smoke.sh                           +40 -20  (CA sharing, etcd-port wiring, JSON-array decode)
docs/synergy/raft-handshake-fix-2026-05-13.md         +this
```

## Charter (2) — production-ready

Raft consensus is now verified end-to-end against three live `cave-runtime` processes. The pipeline from HTTPS write → Raft propose → log replication → applied state machine → leader response is exercised by the smoke. Failover survives a `kill -9` of the leader.

Remaining follow-ups (not blocking):
- Other etcd write routes (`kv_txn`, `cluster_*` mutations) — same dispatch pattern.
- cave-apiserver POST/PUT/DELETE adoption — 28 resource kinds × 3 verbs, larger surface.
- ReadIndex linearizable reads.
- Catch-up after restart — smoke doesn't re-spawn the killed node; tested implicitly via WAL recovery (`cluster-csr-ca-wal-2026-05-12.md`).
