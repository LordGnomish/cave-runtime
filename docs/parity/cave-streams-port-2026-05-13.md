# cave-streams parity — 2026-05-13 sweep (KRaft mode)

**Upstreams:** `apache/kafka v4.2.0` (Apache-2.0, primary) +
`apache/pulsar v4.2.0` (Apache-2.0, secondary).
**Delta from previous audit:** `2026-05-12` snapshot at `fill_ratio = 0.7907`.

## What this sweep landed

A new `crates/cave-streams/src/kraft/` module porting Kafka's
KRaft metadata-quorum surface (KIP-500 + KIP-595 + KIP-631).
Five files, ~1180 LOC + 32 unit tests:

| File | Role |
|------|------|
| `mod.rs`              | module root + re-exports |
| `epoch.rs`            | `ControllerEpoch` monotonic counter + `VoterSet` (elect / step-down / quorum size) |
| `metadata.rs`         | `MetadataRecord` enum (Topic / TopicRemoved / Partition / Broker / BrokerUnregistered / Config — last with `value=None` as tombstone), `ClusterMetadata` materialised view with cascade-on-topic-removed |
| `metadata_log.rs`     | append-only log with by-key compaction; offset + high-water-mark + last-epoch tracking |
| `quorum_controller.rs`| state machine accepting `ControllerRequest::{CreateTopic, DeleteTopic, RegisterBroker, UnregisterBroker, SetConfig}` — validates pre-conditions (duplicate-topic, replication-factor vs live brokers, empty fields), emits records, returns `ControllerResponse::{Ok, Rejected, NotLeader}` |

**32 new unit tests pass** in `cave-streams --lib`:
- `ControllerEpoch` monotonicity, `VoterSet` quorum math + elect / stale-rejection / step-down (6)
- `MetadataKey` distinguishes variants, tombstone detection, `ClusterMetadata::apply` for topic/partition/config, topic-removed cascades to partitions, config-value-None deletes (5)
- `MetadataLog` monotonic offsets, atomic batch ordering, compaction drops predecessor, tombstone removes live entry, snapshot reflects latest leader, `last_epoch` (6)
- `QuorumController` not-leader rejection, force-leader path, CreateTopic emits topic+N partitions with round-robin leadership over 3 live brokers, rejects empty-name / duplicate / insufficient-RF, DeleteTopic emits tombstone, register / unregister broker, SetConfig set-then-unset, empty-component rejection (11)
- Combined integration coverage of the state machine + log (4)

## Counts

| Bucket   | 2026-05-12 | 2026-05-13 |
|----------|-----------:|-----------:|
| Mapped   | 18 | **19** |
| Skipped  | 16 | 16 |
| Unmapped | 9 | **8** |
| **Total** | 43 | 43 |
| **fill_ratio** | 0.7907 | **0.8140** |

## What changed in the inventory

* `[[mapped]]` gained `apache/kafka:raft/ + metadata/ + controller/`,
  pointing to the five new files.
* `[[unmapped]]` Kafka raft entry removed.

## What this PR does NOT claim

* `fill_ratio = 0.8140` does NOT mean cave-streams is 81% of a
  production Kafka/Pulsar. It claims 81% of the upstream's
  top-level packages are either covered or honestly out of
  scope.
* **No replication transport.** The state machine + compacted
  metadata log are in place but the actual Raft consensus is
  delegated to a future `RaftTransport` trait. cave-etcd
  already has a working raft implementation; wiring it through
  the new `QuorumController` is tracked but not landed here.
* **No on-disk snapshots.** KIP-630 compacted snapshots are
  tracked-not-shipped. The metadata log compacts in-memory
  by record-key, which is the semantics-correct behavior for
  a single-node controller.
* **No KRaft RPC endpoints over the Kafka wire.** KIP-595
  introduced `Vote` / `BeginQuorumEpoch` / `EndQuorumEpoch` /
  `Fetch` for the controller quorum. Those are added by the
  existing `kafka_wire`/`kafka_protocol` layer once the
  transport plugs in. Library-only for now.
* The broker still uses its existing in-memory metadata; the
  new `kraft` module is **parallel surface** — production
  wiring (`Broker::metadata` reading the `QuorumController`'s
  `snapshot()`) is its own follow-up paket.
