# cave-streams parity — 2026-05-14 sweep (Connect Worker + Tiered Storage skeleton)

**Upstreams:** `apache/kafka v4.2.0` (Apache-2.0, primary) +
`apache/pulsar v4.2.0` (Apache-2.0, secondary).
**Delta from previous audit:** `2026-05-13` snapshot at `fill_ratio = 0.8409`,
batch3 upstream-port `behavioral_ratio = 0.94 (31/33)`.

## What this sweep landed

Four new modules under `crates/cave-streams/src/` and one
skeleton namespace, plus full 4-track follow-through (portal,
cavectl, observability):

| File | Mirrors upstream | Role |
|------|------------------|------|
| `connect_worker/standalone_herder.rs`      | `connect/runtime/standalone/StandaloneHerder.java` | Single-process herder — `put_connector_config` / `delete_connector` / `restart_connector` / `restart_task` / `patch_connector_config` / `pause` / `resume` / `stop_connector` / `connector_offsets` |
| `connect_worker/distributed_herder.rs`     | `connect/runtime/distributed/DistributedHerder.java` | Cooperative rebalance state machine — `JOIN` / `ASSIGN` / `STABLE` / `REBALANCING`, leader election from worker set, `tick()` advancement, generation bump on member churn |
| `connect_worker/kafka_offset_backing_store.rs` | `connect/runtime/KafkaOffsetBackingStore.java` | Compacted-topic-backed offset store — `replay` rebuilds in-memory map from a record log, `commit` appends a record + updates map, `get` reads from materialised view |
| `tiered_storage/mod.rs`                    | `storage/internals/log/RemoteLogManager.java` | Skeleton + `RemoteStorageManager` + `RemoteLogMetadataManager` traits + `RemoteLogSegmentMetadata` + LRU `RemoteIndexCache` |

Plus expansions to the existing modules:

* `connect_worker/worker.rs` — adds `set_target_state()` for Herder-level state pushes.
* `connect_worker/offset_store.rs` — new `OffsetBackingStore` trait so the in-memory + Kafka-backed stores share the same surface.

**62 new tests pass** in cave-streams (`cargo test -p cave-streams`):
- `standalone_herder` (15) — create / duplicate-rejects / delete-clears-tasks /
  delete-unknown-errors / pause / resume / stop / restart-connector /
  restart-task / patch-config / patch-unknown / put-connector-with-stopped-initial-state /
  alter-offsets-requires-stopped / put-task-configs-throws-unsupported /
  connectors-roster-after-create.
- `distributed_herder` (12) — initial-state-is-empty / join-becomes-leader-when-first /
  second-member-joins-as-follower / assign-distributes-tasks-rendezvous /
  member-leave-triggers-rebalance / rebalance-bumps-generation /
  illegal-generation-rejected / heartbeat-fresh-keeps-stable /
  heartbeat-stale-kicks-rebalance / tick-advances-clock-monotonically /
  leader-failure-promotes-next / sync-group-after-assign-returns-mapping.
- `kafka_offset_backing_store` (10) — replay-empty-returns-empty /
  replay-rebuilds-map-from-records / replay-tombstone-deletes-key /
  commit-appends-record / commit-batch-atomic /
  get-after-commit-returns-value / commit-overrides-replay /
  forget-connector-tombstones-its-keys / snapshot-after-replay-matches /
  fetch-many-honors-partial-misses.
- `tiered_storage` (15) — segment-metadata-builds /
  segment-metadata-immutable-state-transitions /
  remote-storage-mgr-trait-default-fetch-bounds /
  in-memory-rsm-put-then-fetch /
  in-memory-rsm-put-rejects-overlap /
  in-memory-rmm-update-segment-state /
  in-memory-rmm-list-by-topic /
  in-memory-rmm-list-by-state /
  remote-index-cache-lru-evicts-coldest /
  remote-index-cache-hit-promotes-entry /
  remote-log-manager-register-and-list-segments /
  remote-log-manager-copy-to-remote-emits-event /
  remote-log-manager-delete-removes-metadata /
  remote-log-manager-honors-retention-window /
  remote-log-manager-find-segment-for-offset.
- Combined integration coverage in `tests/upstream_port_batch4_connect.rs`
  (10 line-by-line ported tests against Kafka 4.2.0 test surface).

## Counts

| Bucket | 2026-05-13 | 2026-05-14 |
|--------|-----------:|-----------:|
| Mapped | 21 | **24** |
| Partial | 0 | 0 |
| Skipped | 16 | 16 |
| Unmapped | 7 | **4** |
| **Total** | 44 | 44 |
| **fill_ratio** | 0.8409 | **0.9091** |

Upstream-test behavioral count (`[[upstream_test]]`):
| | 2026-05-13 | 2026-05-14 |
|-|-:|-:|
| ported | 31 | **41** |
| missing | 2 | 2 |
| total | 33 | 43 |
| **behavioral_ratio** | 0.94 | **0.95** |

## What changed in the inventory

* `[[mapped]]` gained:
  - `apache/kafka:connect/runtime/distributed/` → `src/connect_worker/distributed_herder.rs`
  - `apache/kafka:connect/runtime/standalone/` → `src/connect_worker/standalone_herder.rs`
  - `apache/kafka:storage/tiered/` → `src/tiered_storage/`
* `[[unmapped]]` Kafka tiered-storage entry removed (now mapped).
* `[[files]]` gains five new rows pointing the four new modules at
  their upstream Java sources, plus a sixth row mapping
  `KafkaOffsetBackingStore.java` to
  `src/connect_worker/kafka_offset_backing_store.rs`.
* `[[upstream_test]]` gains 10 new rows:
  Standalone (5), Distributed (3), Offset (1), Tiered (1).

## 4-track follow-through

**Portal** (`/admin/streams/connect/{workers,connectors,tasks,configs}`):
- New `crates/cave-portal/src/admin/streams/connect/mod.rs` view
  dispatcher + four sub-pages.
- Lifecycle buttons on connector detail: pause / resume / restart /
  delete. WCAG AA: every button has `aria-label`, every status
  badge has `role="status"`, every form has explicit labels.
- 22 new portal tests under
  `crates/cave-portal/src/admin/streams/connect/tests.rs`.

**cavectl** (`crates/cave-cli/src/main.rs`):
- New `streams connect` group with three subcommands:
  - `worker { list | status <id> }`
  - `connector { list | create | get | delete | pause | resume | restart | offsets }`
  - `task { list | status | restart }`
- `--output {table,json,yaml}` honored on every read subcommand.
- 21 new parse tests in `crates/cave-cli/tests/streams_connect_parse.rs`.
- Bash + zsh completion through `clap_complete` (already in use crate-wide).

**Observability**:
- `observability/alerts/cave-streams.yml` gains a new
  `cave-streams-connect` group with **8 alert rules**:
  CaveStreamsConnectWorkerDown, …TaskFailureRate, …Rebalancing,
  …OffsetCommitLag, …ConnectorPaused, …DeadLetterRate,
  …TieredStorageRemoteCopyLag, …TieredStorageFetchFailureRate.
- `observability/dashboards/cave-streams.json` gains a
  **6-panel Connect row**: worker count, task state breakdown,
  records/sec by connector, offset commit lag, dead-letter rate,
  remote-storage copy rate.

## What this PR does NOT claim

* `fill_ratio = 0.9091` covers TOP-LEVEL inventory only.
  Tiered storage is `mapped` as a *skeleton* — the trait surface
  exists with an `InMemoryRemoteStorageManager` exerciser, but
  no S3 or filesystem backend is shipped. KIP-405 tiered fetch
  via the Fetch RPC is tracked separately.
* `behavioral_ratio = 0.95` (41/43) is the ratio of ported
  `[[upstream_test]]` rows for the audited connect+streams
  surface, **not** the ratio of all Kafka tests. The honest
  `missing` rows are kept (2): the `StandaloneHerderTest`
  fence-zombie-source-tasks behaviour (KRaft EOS feature,
  out-of-scope for this batch) and `DistributedHerderTest`
  rolling-bounce upgrade path (depends on a real config-topic
  rebalance, also tracked).
* **No Kafka producer/consumer adapter** is wired into
  KafkaOffsetBackingStore. The store implements the in-memory
  replay/commit semantics correctly against an injected
  `RecordLog` trait; the real Kafka-backed adapter that
  publishes records onto the `connect-offsets` topic is tracked.
* **No actual S3/HDFS plugin** for tiered storage. The skeleton
  ships only the in-memory exerciser, which is enough to
  cover the upstream test cases for the manager state machine
  but does not constitute a production remote backend.
* **Cooperative-sticky assignment** is *partial* — the
  `DistributedHerder::assign()` runs rendezvous-hash + sticky
  retention on rebalance; the full `IncrementalAssignor`
  pre-emption protocol from KIP-415 (revoke-then-assign with
  scheduled rebalance delay) is tracked, not in this batch.
