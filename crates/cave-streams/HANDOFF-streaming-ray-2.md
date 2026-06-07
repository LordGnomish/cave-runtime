<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-streams — streaming-ray-2 handoff (2026-06-07)

Unified Kafka + Pulsar streaming crate. Single Rust process, two wire
protocols (Kafka 9092 / Pulsar 6650), one `parity.manifest.toml`, one
4-track surface. This ray closed the **last two honest parity gaps** so
`honest_ratio` reaches **1.0**.

## What landed

| Gap (was `[[unmapped]]` / `[[scope_cuts]]`) | New module | Upstream |
|---|---|---|
| Pulsar transactions (PIP-31) | `src/pulsar_transactions.rs` | `pulsar-transaction/coordinator` `TransactionMetadataStore` + `TxnMetaImpl`; `pulsar-broker` `TopicTransactionBuffer` |
| Kafka share groups (KIP-932) | `src/share_group.rs` | `core/.../share/SharePartition.java` + `server-common/.../share/{RecordState,AcknowledgeType}` |

Both ported via **strict RED→GREEN TDD** (each: a `test(...) RED` commit
with the core method stubbed to a wrong value, then a `feat(...) GREEN`
commit restoring the real logic).

### pulsar_transactions.rs
- `TxnStatus` state machine — exact `TxnMetaImpl.checkTxnStatusCanBeUpdated`
  legal-transition table (OPEN→COMMITTING/ABORTING, COMMITTING→COMMITTED,
  ABORTING→ABORTED, idempotent restatement).
- `TxnID(most_sig=coordinator_id, least_sig=sequence_id)` monotonic issuance.
- `TransactionCoordinator` (= metadata store): `new_transaction`,
  `add_produced_partitions`/`add_acked_partitions` (OPEN-gated),
  `update_txn_status` (expected-status precond + transition table),
  `commit`/`abort` helpers, `timed_out` tracker.
- `TransactionBuffer` (= `TopicTransactionBuffer`): append buffers invisibly,
  `commit` publishes + advances `max_read_position`, `abort` discards +
  remembers the txn; interleaved txns publish independently.

### share_group.rs
- `SharePartition` in-flight per-offset state machine: `acquire`
  (materialise up to fetch-HWM under acquisition lock, delivery_count++,
  capped at max_records), `acknowledge` (ACCEPT→ACKNOWLEDGED,
  REJECT→ARCHIVED, RELEASE→AVAILABLE unless `delivery_count >=
  max_delivery_count` → ARCHIVED poison-pill guard), SPSO slides over the
  contiguous terminal prefix, `release_expired_locks`.
- `ShareGroup` registry with epoch-fenced member join.

## 4-track

1. **Backend** — the two lib modules + `src/metrics.rs`.
2. **REST** (`src/routes.rs`) — `POST /api/streams/pulsar/transactions/preview`,
   `POST /api/streams/share-groups/preview` (both drive the *real* state
   machines), `GET /api/streams/metrics` (Prometheus 0.0.4).
3. **cavectl** (`crates/cave-cli/src/main.rs`, pkg `cavectl`) —
   `streams pulsar-txn preview --commits N --aborts M`,
   `streams share-group preview --records N --accept K`.
4. **Portal** (`crates/cave-portal/src/admin/streams/mod.rs`) — `/admin/streams`
   now renders the TC status-machine + share-partition record-state panels.
5. **Metrics** — `cave_streams_{kafka_topics,kafka_consumer_groups,
   pulsar_tenants}` gauges + `cave_streams_{pulsar_txn,share_group}_preview_total`
   counters.

## Acceptance (`tests/streams_unified_acceptance.rs`, 4 tests)
- Pulsar txn commit publishes / abort stays invisible.
- Kafka share group acquire→ack→SPSO advance.
- Pulsar multi-region replication fan-out + loop-guard.
- Schema Registry BACKWARD add-optional-field.

Kafka/Pulsar produce-consume wire round-trips remain covered by
`kafka_wire::tests::test_kafka_produce_roundtrip` +
`pulsar_wire::tests::test_pulsar_send_returns_receipt`.

## Parity
`mapped 27→29, unmapped 2→0, skipped 16, total 45`,
`fill_ratio/honest_ratio 0.9556 → 1.0`. The 16 remaining skips are all
CLI/build/test-harness/non-Java-client/crypto cuts — no unmapped
library-surface gaps remain. `last_audit = 2026-06-07`; self-audit
`TODAY` const bumped to match (gate `assertion_8`).

## Verification
- `cargo test -p cave-streams` → **573 lib + all integration green**
  (incl. 9 self-audit gates, 4 acceptance, metrics/txn/share unit tests).
- `cargo build -p cave-runtime` (mounts the router) + `-p cavectl` clean.

## Status / GOTCHAs
- Branch `feature/streams-kafka-pulsar-unified`, worktree
  `../cave-streams-impl`. Local `--no-ff` merge only; **not pushed**
  (per directive).
- Crate path on this branch is the **flat** `crates/cave-streams` (off
  `main`/f8f0aa53); the refactor-sweep branch uses themed
  `crates/data/cave-streams` and already promoted these two gaps — this
  brings the flat/main lineage to parity.
- `post-commit` hook regenerates `docs/parity/parity-index.json` from the
  manifest and amends it into the commit; `assertion_9` keeps them in sync.
- Crate-private module gotcha still applies: test private `mod`s via
  in-file `#[cfg(test)]`; the new modules are `pub mod` so integration
  tests reach them.

## Backlog (genuine, still scoped out)
- Pulsar TC cross-broker failover + on-ledger metadata persistence
  (delegates to `pulsar_managed_ledger.rs` substrate).
- Share-group persistent share-state topic + coordinator RPC transport.
- Both preview endpoints are stateless compute-from-request (cave-etcd
  `/simulate` precedent); broker-lifecycle integration is future work.
