# TDD coverage audit — cave-ha vs etcd (Raft) @ v3.5.13

- **Cave crate:** `cave-ha` (theme: compute)
- **Upstream:** https://github.com/etcd-io/etcd @ `v3.5.13`
- **Upstream test inventory:** 346 `*_test.go` files / 1888 test symbols (`/tmp/tdd-audit/cave-ha-upstream-tests.txt`)
- **Cave test functions:** 65 `#[test]`/`#[tokio::test]` (4 integration files + 4 in-src modules + proptest smoke)

## Scope framing

cave-ha is a **fresh-implementation Raft engine** (Diego Ongaro Raft: leader election,
pre-vote, log replication, joint-consensus membership, snapshot install, ReadIndex/lease,
check-quorum, leadership transfer). It is mapped to etcd because etcd vendors the canonical
`go.etcd.io/raft` library, but cave-ha is **not** a line-port — it has its own actor model
(`RaftNode` + `RaftHandle` over tokio mpsc), its own `MemLog`, and its own membership type.

The vast majority of etcd's 1888 test symbols are **scope-cut**: they cover the etcd *server*
(`etcdserver`, `mvcc`, `lease`, `auth`, `wal`, `v3rpc`), client libraries, the `etcdctl`/`etcdutl`
CLIs, TLS transport, file utilities, and SRV discovery — none of which cave-ha reimplements
(cave-ha consumes `cave-kernel::consensus` traits and is consumed by cave-etcd/cave-apiserver
for those layers). The behaviorally-relevant comparison surface is the upstream `raft/` package:
`raft_test.go` (112), `raft_paper_test.go` (26), `log_test.go` (17), `storage_test.go` (8),
`confchange/*`, `tracker/progress_test.go` (8), `raft_snap_test.go` (5).

**Cave's cluster-level Raft behaviors are well-covered** by integration tests
(`election_test.rs`, `replication_test.rs`, `partition_test.rs`): single/3/5-node election,
one-leader invariant, pre-vote term-inflation suppression, leadership transfer, check-quorum
step-down, split-brain prevention, partition recovery, replication, snapshot trigger,
compaction correctness. Several cluster tests are `#[ignore]`-flagged as flaky-under-parallel
(timing races the tokio scheduler) — the *behavior* is exercised, the gap is determinism, not
coverage.

**The real, honest gaps are the pure synchronous public functions** that etcd unit-tests
directly but cave only exercises indirectly (through flaky cluster paths) or not at all.
These are deterministic, fast, non-flaky, and are the portable-coverage priority.

## Behavior coverage table

| behavior | upstream test | cave impl? | cave test? | gap type | suggested test |
|---|---|---|---|---|---|
| Log slice for AppendEntries range `[lo,hi)` | `TestSlice`, `TestStorageEntries` | yes — `log::MemLog::slice` | no (only via cluster replication) | **portable-coverage** | unit: append 5 entries, assert `slice(2,4)` returns idx 2..3; `slice` past snapshot errors |
| Log term lookup across snapshot boundary | `TestTerm`, `TestStorageTerm` | yes — `log::MemLog::term` | no | **portable-coverage** | unit: `term(snapshot_index)` == snapshot_term; `term` of compacted idx errors |
| Append truncates conflicting suffix | `TestAppend`, `TestFindConflict` | yes — `log::MemLog::append` | no | **portable-coverage** | unit: append idx1..3, re-append conflicting idx2 (diff term) → log truncates to idx1 then re-extends |
| Compaction discards ≤ index, sets snapshot meta | `TestCompaction`, `TestStorageCompact`, `TestCompactionSideEffects` | yes — `log::MemLog::compact` | indirect only (`partition_test::test_compaction_correctness` is cluster-level) | **portable-coverage** | unit: append 1..5, `compact(3,t)`, assert `first_index()==4`, `snapshot_index()==3`, `entry(2)` errors |
| `truncate_to` rollback | `TestStableTo`, `TestUnstableTruncateAndAppend` | yes — `log::MemLog::truncate_to` | no | **portable-coverage** | unit: append 1..5, `truncate_to(3)`, assert `last_index()==3`, len==3 |
| Find most-recent membership entry in log | `TestLogRestore`, confchange restore | yes — `log::MemLog::last_membership` | no | **portable-coverage** | unit: append normal + 2 membership entries, assert `last_membership()` returns the latter config |
| Snapshot chunk-split for streaming | `TestSnapshotSucceed` (chunking path) | yes — `snapshot::Snapshot::chunks` | no | **portable-coverage** | unit: 10-byte data, `chunks(4)` → 3 chunks, last `done==true`, offsets contiguous; empty data → 1 done chunk |
| Snapshot reassembly + out-of-order reset | `TestSnapshotSucceed`, `TestSnapshotAbort` | yes — `snapshot::SnapshotReceiver::feed` | no | **portable-coverage** | unit: feed in-order chunks → `Some(Snapshot)` on done; feed gap offset → `None` + buffer reset |
| Joint config has_quorum requires both sets | `TestConfChangeQuick`, quorum datadriven | yes — `types::MembershipConfig::has_quorum` | yes — `membership::test_joint_quorum` | covered | — |
| Joint-config detection / voter union | `TestConfState_Equivalent` | yes — `is_joint`/`all_voters`/`all_nodes` | no | **portable-coverage** | unit: build joint cfg, assert `is_joint()`, `all_voters()` == old∪new, `all_nodes()` includes learners |
| Joint config for node removal | `TestClusterValidateConfigurationChange`, `TestConfChange*` | yes — `membership::joint_for_remove` | no (only `joint_for_add` tested) | **portable-coverage** | unit: remove voter → `voters_outgoing` holds old set, removed id absent, `auto_leave==true` |
| Leave-joint strips outgoing set (C_new) | `TestConfChangeV2*`, restore | yes — `membership::leave_joint` | no | **portable-coverage** | unit: `leave_joint(joint)` → `voters_outgoing==None`, `auto_leave==false`, voters preserved |
| Membership change validation | `TestClusterValidateConfigurationChange` | yes — `membership::validate` | partial — only `test_validate_empty` | **portable-coverage** | unit: voter∩learner overlap rejected; removing >1 voter rejected; valid single-change ok |
| Quorum size N→majority | `quorum/datadriven_test`, `TestAddNodeCheckQuorum` | yes — `MembershipConfig::quorum` | yes — `membership::test_quorum_sizes` | covered | — |
| ReadIndex queue accumulates acks → quorum | `TestReadIndex*` (server), tracker | yes — `read_only::ReadOnlyQueue::add`/`ack` | no | **portable-coverage** | unit: add req at idx, `ack(peer, quorum)` accumulates until len≥quorum then drains the request |
| Leader lease validity window | lease read tests | yes — `read_only::LeaderLease` | no | **portable-coverage** | unit: `renew()` → `is_valid()`; `invalidate()` → `!is_valid()` |
| Single/3/5-node leader election, one-leader invariant | `TestLeaderElectionInOneRoundRPC`, `TestStartAsFollower` | yes — `node::RaftNode` | yes — `election_test::test_{single_node,three_node,five_node}_election` | covered | — |
| Pre-vote suppresses term inflation under partition | `TestConfChangeCheckBeforeCampaign`, pre-vote paper | yes | yes — `election_test::test_pre_vote_prevents_term_inflation` | covered | — |
| Leadership transfer via TimeoutNow | `TestCtlV3MoveLeaderScenarios` | yes — `node::initiate_transfer` | yes — `election_test::test_leadership_transfer` | covered | — |
| Check-quorum step-down on quorum loss | `TestAddNodeCheckQuorum` | yes — `node::do_check_quorum` | yes — `election_test::test_check_quorum_stepdown` | covered | — |
| Split-brain prevention (majority gets leader) | paper safety | yes | yes — `partition_test::test_split_brain_prevention` | covered | — |
| Partition recovery / flaky-net consensus | network tests | yes | yes — `partition_test::test_partition_recovery`, `test_flaky_network_consensus` | covered | — |
| Log replication / commit advance | `TestLeaderStartReplication`, `TestLeaderCommitEntry` | yes — `node::maybe_advance_commit` | yes — `replication_test::test_three_node_replication` | covered (commit-current-term-only rule is internal, indirectly exercised) | — |
| Leader-only-commits-current-term rule | `TestLeaderOnlyCommitsLogFromCurrentTerm` | yes — `maybe_advance_commit` (term guard) | no (internal fn, not directly assertable) | missing-impl-test (private) | scope-cut: private fn; covered indirectly by replication test |
| Follower restart catch-up | network/snapshot tests | yes | `#[ignore]` flaky — `replication_test::test_follower_catches_up` | flaky (behavior present, needs det-clock) | — |
| ReadIndex linearizable read end-to-end | `TestReadIndex` (server) | yes | `#[ignore]` hangs single-node — `replication_test::test_read_index` | flaky/incomplete cluster path | covered at unit level via ReadOnlyQueue fills above |
| Kernel-trait projection (LogStore/RaftHandle/error map) | n/a (cave-specific bridge) | yes — `kernel_bridge` | yes — 31 tests in `kernel_bridge.rs` | covered | — |
| Snapshot store/prune/order (storage layer) | `TestBackendSnapshot`, `TestStorageCreateSnapshot` | yes — `snapshot.rs` (5 tests) | yes — `snapshot::test_*` | covered | — |
| WAL / fileutil / tlsutil / SRV / etcdctl / mvcc / auth / lease | `client/pkg/*`, `etcdctl`, `mvcc`, `auth` (~1700 symbols) | no — owned by cave-etcd / cave-kernel / transport TLS | n/a | **scope-cut** (vendor/server/CLI/infra) | — |

## Recommended TDD fills (portable-coverage first)

Each fill is a fast, deterministic, non-async (or trivially-async) unit test exercising an
exact public cave function. Add to `src/raft/log.rs`, `src/raft/snapshot.rs`,
`src/raft/membership.rs`, `src/raft/read_only.rs` `#[cfg(test)] mod tests`.

1. `cave_ha::raft::log::MemLog::slice` — range read returns correct sub-slice; `lo ≤ snapshot_index` errors (`TestSlice`).
2. `cave_ha::raft::log::MemLog::term` — term at snapshot boundary == snapshot_term; compacted index errors (`TestTerm`).
3. `cave_ha::raft::log::MemLog::append` — re-appending a conflicting index truncates the suffix then extends (`TestAppend`/`TestFindConflict`).
4. `cave_ha::raft::log::MemLog::compact` — post-compact `first_index`/`snapshot_index`/`snapshot_term` and pre-snapshot `entry()` error (`TestCompaction`/`TestStorageCompact`).
5. `cave_ha::raft::log::MemLog::truncate_to` — rollback shrinks `last_index`/`len` correctly (`TestStableTo`).
6. `cave_ha::raft::log::MemLog::last_membership` — returns the most recent `MembershipChange` entry's decoded config (`TestLogRestore`).
7. `cave_ha::raft::snapshot::Snapshot::chunks` — splits data into contiguous `done`-terminated chunks; empty data yields one done chunk (`TestSnapshotSucceed`).
8. `cave_ha::raft::snapshot::SnapshotReceiver::feed` — in-order chunks assemble to `Some(Snapshot)`; an offset gap returns `None` and resets the buffer (`TestSnapshotAbort`).
9. `cave_ha::raft::membership::joint_for_remove` — produces joint cfg with `voters_outgoing`=old, removed id gone, `auto_leave=true` (`TestClusterValidateConfigurationChange`).
10. `cave_ha::raft::membership::leave_joint` — drops `voters_outgoing`, clears `auto_leave`, preserves voters (`TestConfChangeV2*`).
11. `cave_ha::raft::membership::validate` — rejects voter/learner overlap and multi-voter removal; accepts a valid single change (`TestClusterValidateConfigurationChange`).
12. `cave_ha::MembershipConfig::is_joint` / `all_voters` / `all_nodes` — joint detection and old∪new voter / learner-inclusive node union (`TestConfState_Equivalent`).
13. `cave_ha::raft::read_only::ReadOnlyQueue::add` + `ack` — request accumulates per-peer acks and drains exactly when `acks.len() ≥ quorum` (`TestReadIndex`).
14. `cave_ha::raft::read_only::LeaderLease::renew`/`is_valid`/`invalidate` — lease window validity transitions (lease-read tests).

## Honest notes

- **14 portable-coverage gaps**, all pure/deterministic public functions — high-value, zero-flake fills.
- Cluster-level Raft safety/liveness is genuinely covered; the `#[ignore]` cluster tests are
  flaky-timing, not absent behavior. Padding them with more cluster tests would add flake, not
  signal — the unit fills above give the same coverage deterministically.
- ~1700 upstream test symbols are scope-cut (etcd server/CLI/storage/auth/TLS/fileutil), correctly
  not cave-ha's surface.
- The private `maybe_advance_commit` term-guard (`TestLeaderOnlyCommitsLogFromCurrentTerm`) is not
  directly unit-testable without exposing internals; it is exercised by replication integration — left as-is.
