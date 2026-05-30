# cave-streams — honest_ratio uplift gap analysis (2026-05-30)

**Crate:** `crates/data/cave-streams` · **Branch:** `claude/cave-streams-honest-2026-05-30` off origin/main `cd111010`
**Upstreams:** apache/kafka 4.2.0 (`9f8b3ad4…`) + apache/pulsar v4.2.0 (`1940aebc…`), both Apache-2.0.
**Unified crate** per `ADR-RUNTIME-STREAMING-CONSOLIDATION-001`: one Rust broker, two wire protocols.

---

## 1. Baseline (origin/main `cd111010`)

| metric | value |
| --- | --- |
| cave-streams src LOC (all `.rs` on disk) | 31,412 |
| cave-streams test suite | 747 passed / 0 failed |
| manifest `mapped_count` | 27 |
| manifest `skipped_count` | 18 |
| manifest `unmapped_count` | 0 |
| manifest `total` | 45 |
| manifest `fill_ratio` | 1.0 |
| manifest **`honest_ratio`** | **0.9556** |

`honest_ratio` is **authored in the manifest `[parity]` block**, read verbatim by
`scripts/build-parity-index.py` (`_re_float(parity_block, "honest_ratio")`) — it is **not**
recomputed from counts. The post-commit hook regenerates `docs/parity/parity-index.json`
from the manifest, so the honest path is: genuinely close gaps → bump manifest counts +
`honest_ratio` → let the hook regenerate the index. **No hand-edit of parity-index.json.**

The 0.9556 (= 43/45) gap is exactly the **two formally scope-cut subsystems**:

1. `apache/kafka:server/group-coordinator/share/` — KIP-932 queue-style share groups
2. `apache/pulsar:pulsar-broker/.../transactions/` — Pulsar transaction coordinator

Both were demoted to `[[skipped]]` + `[[scope_cuts]]` (deferred to `streaming-ray-2`) on
2026-05-28. **Closing them via real ports is the direct, honest lift to 1.0.**

---

## 2. Upstream LOC (sparse-checkout at pinned SHAs, non-test Java)

| upstream subsystem | path | LOC |
| --- | --- | --- |
| Kafka share-coordinator | `share-coordinator/` | 3,793 |
| Kafka share ecosystem (coordinator + clients share consumer + records) | `*share*` | ~15,773 |
| Kafka SCRAM | `clients/.../common/security/scram/` | 1,629 |
| Kafka OAUTHBEARER | `clients/.../common/security/oauthbearer/` | 9,922 |
| Pulsar transactions | `pulsar-transaction/` + `pulsar-broker/.../transaction/` | 15,582 |

cave-streams ports the **behavioral core** of each subsystem (state machines, invariants,
wire/algorithm semantics) — not the JVM bootstrapping, RPC plumbing, gradle, or test
harness. LOC ratios are not the parity metric (repo parity is count-based per
`ADR-RUNTIME-PARITY-100-PCT-001`); they bound the surface this ray touches.

---

## 3. Priority-list presence matrix

Status against the task's Kafka/Pulsar/core/integration priority list.

### Kafka
| subsystem | cave module | status |
| --- | --- | --- |
| KRaft controller (Raft) | `kraft/{mod,epoch,metadata,metadata_log,quorum_controller,rpc}` | ✅ present |
| broker log segments + index | `segment_log.rs` | ✅ present |
| compaction + retention | `log_compaction.rs` | ✅ present |
| producer (batching/compression/acks) | `compression.rs` + `idempotent_producer.rs` | ✅ present¹ |
| idempotent + transactional producer (EOS) | `idempotent_producer.rs` + `transactions.rs` + `txn_markers.rs` | ✅ present |
| consumer group coordination + rebalance | `consumer_group.rs` + `incremental_rebalance.rs` | ✅ present |
| Connect framework | `connect.rs` + `connect_worker/*` + `connect_rest.rs` | ✅ present |
| Streams Processor API | `kafka_streams_processor.rs` | ✅ present |
| **Streams DSL (KStream/KTable)** | `streams_api.rs` (orphan stub, not wired) | ⚠️ **GAP — stub only** |
| **SASL/SCRAM/OAUTHBEARER** | — | ❌ **GAP — absent** |
| tiered storage (KIP-405) | `tiered_storage/mod.rs` | ✅ present (skeleton) |
| MirrorMaker 2 | `mirror.rs` | ✅ present |
| **KIP-932 share groups** | — | ❌ **GAP (scope-cut) — TARGET this ray** |

### Pulsar
| subsystem | cave module | status |
| --- | --- | --- |
| broker + binary protocol | `pulsar_wire.rs` | ✅ present |
| topics (persistent/partitioned) | `pulsar_topic.rs` + `partitioned_topic.rs` | ✅ present |
| subscriptions (excl/shared/failover/key_shared) | `pulsar_dispatch.rs` | ✅ present |
| multi-tenancy | `tenant.rs` + `pulsar_admin.rs` | ✅ present |
| geo-replication | `pulsar_geo_replication.rs` | ✅ present |
| managed ledger (BookKeeper-compat) | `pulsar_managed_ledger.rs` | ✅ present |
| schema registry | `schema_registry.rs` + `schema_evolution.rs` | ✅ present |
| IO connectors | `pulsar_io_connectors.rs` | ✅ present |
| Functions framework | — (scope-cut → cave-pipelines) | ⏭️ skipped (ADR) |
| **TLS/JWT/OAuth2/Athenz auth** | — | ❌ **GAP — absent** |
| **transactions (TC + buffer + pending-ack)** | — | ❌ **GAP (scope-cut) — TARGET this ray** |

¹ `producer.rs` / `consumer.rs` exist on disk but are **not wired** (see §5).

---

## 4. This ray's scope (strict TDD, RED→GREEN, separate commits)

| # | subsystem | upstream | plan |
| --- | --- | --- | --- |
| 3A | **KIP-932 share groups** | `share-coordinator/` 3.8k LOC | port RecordState/AcknowledgeType state machines, SharePartition acquire/ack/release/reject/renew, lock sweep, move_start_offset, ShareGroup join/leave epoch, ShareSession |
| 3B | **Pulsar transactions** | `pulsar-transaction/` 15.6k LOC | port TxnID/TxnStatus state machine, TransactionMetadataStore, TxnMeta, TransactionBuffer, AbortedTxnProcessor, PendingAckHandle, TransactionTimeoutTracker (min-heap), TransactionCoordinator |

Each lands as a **RED commit** (failing tests) then a **GREEN commit** (impl + lib wiring).
On close, both promote `[[skipped]]`/`[[scope_cuts]]` → `[[mapped]]`:
`mapped 27→29`, `skipped 18→16`, `honest_ratio 0.9556→1.0`, `fill_ratio` stays 1.0.

---

## 5. Honest findings NOT closed this ray (documented, not hidden)

These are real gaps left open; recorded here so the parity numbers stay honest.

- **Orphan-mapped inflation (~4,961 LOC).** The following `src/*.rs` are on disk and
  **credited in the manifest `[[mapped]]` `local_files`** but are **not declared in
  `lib.rs`**, so they do not compile into the crate:
  `admin.rs`(148), `compaction.rs`(232), `consumer.rs`(439), `kafka_protocol.rs`(835),
  `models.rs`(527), `producer.rs`(419), `storage.rs`(720), `store.rs`(97),
  `streams_api.rs`(405), `topic.rs`(149), `tests.rs`(990). Their behavior is provided by
  the wired siblings (`broker.rs`, `segment_log.rs`, `idempotent_producer.rs`, etc.).
  **Recommendation:** a follow-up ray should either wire or delete them and drop the dead
  paths from `[[mapped]].local_files`. Not touched here (would balloon the diff and risk
  breakage; orthogonal to honest_ratio).
- **SASL/SCRAM-SHA-256/512 + OAUTHBEARER + PLAIN** (Kafka auth) — absent. ~1.6k+9.9k LOC.
- **Pulsar TLS/JWT/OAuth2/Athenz** auth — absent.
- **Kafka Streams DSL** (KStream/KTable/KGroupedStream) — only the low-level Processor API
  is wired; `streams_api.rs` is an unwired stub.
- **Kafka Connect ↔ Pulsar IO bridge** (cross-protocol connector adapter) — absent.

These remain genuine `streaming-ray-3` candidates; they are **not** counted toward
honest_ratio in this ray (no inflation).

---

## 6. PQC posture

Per repo convention, any new auth/crypto surface ships an ML-KEM/ML-DSA **hybrid
placeholder** (deterministic backend) alongside the classical path. The two subsystems in
this ray (share groups, transactions) carry no crypto surface, so no PQC code lands here;
the SASL/SCRAM follow-up ray will.
