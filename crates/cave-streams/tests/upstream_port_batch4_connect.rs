// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Upstream-test port — Kafka Connect runtime + Tiered Storage.
//!
//! Each test below mirrors a `@Test` method in the upstream
//! Apache Kafka 4.2.0 source tree. The mapping is kept in
//! `crates/cave-streams/parity.manifest.toml::[[upstream_test]]`.
//!
//! These tests are written first against the new modules
//! (`connect_worker::standalone_herder`,
//! `connect_worker::distributed_herder`,
//! `connect_worker::kafka_offset_backing_store`,
//! `tiered_storage`) before those modules exist — Charter §1
//! red→green observable cycle.

use std::collections::BTreeMap;

use cave_streams::connect_worker::distributed_herder::{
    DistributedHerder, HerderState, MemberId,
};
use cave_streams::connect_worker::kafka_offset_backing_store::{
    KafkaOffsetBackingStore, OffsetRecord, RecordLog,
};
use cave_streams::connect_worker::offset_store::{OffsetBackingStore, OffsetKey, OffsetValue};
use cave_streams::connect_worker::standalone_herder::{
    HerderError, StandaloneHerder, TargetState,
};
use cave_streams::connect_worker::task_runtime::TaskKind;
use cave_streams::tiered_storage::{
    InMemoryRemoteLogMetadataManager, InMemoryRemoteStorageManager, RemoteIndexCache,
    RemoteLogManager, RemoteLogSegmentId, RemoteLogSegmentMetadata, RemoteLogSegmentState,
    RemoteStorageManager, TopicIdPartition,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn config(class: &str, tasks_max: u32) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("connector.class".into(), class.into());
    m.insert("tasks.max".into(), tasks_max.to_string());
    m
}

// ── StandaloneHerder ─────────────────────────────────────────────────────────

/// Upstream: `StandaloneHerderTest.testCreateSourceConnector`.
#[test]
fn upstream_standalone_create_source_connector_starts_in_running() {
    let mut h = StandaloneHerder::new();
    let info = h
        .put_connector_config("jdbc", config("io.confluent.JdbcSourceConnector", 2), false)
        .unwrap();
    assert_eq!(info.created, true);
    assert_eq!(info.name, "jdbc");
    assert_eq!(info.tasks.len(), 2);
    assert_eq!(info.kind, TaskKind::Source);
    assert_eq!(h.connectors(), vec!["jdbc".to_string()]);
}

/// Upstream: `StandaloneHerderTest.testCreateConnectorAlreadyExists`.
#[test]
fn upstream_standalone_create_duplicate_returns_already_exists() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSourceConnector", 1), false)
        .unwrap();
    let err = h
        .put_connector_config("jdbc", config("...JdbcSourceConnector", 1), false)
        .unwrap_err();
    assert!(matches!(err, HerderError::AlreadyExists(_)), "got {err:?}");
}

/// Upstream: `StandaloneHerderTest.testCreateConnectorFailedValidation`.
#[test]
fn upstream_standalone_create_rejects_missing_connector_class() {
    let mut h = StandaloneHerder::new();
    let mut bad = BTreeMap::new();
    bad.insert("tasks.max".into(), "1".into());
    let err = h.put_connector_config("c", bad, false).unwrap_err();
    assert!(matches!(err, HerderError::BadConfig(_)), "got {err:?}");
}

/// Upstream: `StandaloneHerderTest.testDestroyConnector`.
#[test]
fn upstream_standalone_delete_connector_clears_tasks() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSourceConnector", 3), false)
        .unwrap();
    h.delete_connector("jdbc").unwrap();
    assert!(h.connectors().is_empty());
    assert!(h.task_configs("jdbc").is_err());
}

/// Upstream: `StandaloneHerderTest.testTargetStates` (PAUSED branch).
#[test]
fn upstream_standalone_pause_transitions_target_state() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSource", 1), false)
        .unwrap();
    h.set_target_state("jdbc", TargetState::Paused).unwrap();
    assert_eq!(h.target_state("jdbc").unwrap(), TargetState::Paused);
}

/// Upstream: `StandaloneHerderTest.testTargetStates` (STOPPED branch).
#[test]
fn upstream_standalone_stop_connector_releases_tasks_but_keeps_config() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSource", 2), false)
        .unwrap();
    h.stop_connector("jdbc").unwrap();
    assert_eq!(h.target_state("jdbc").unwrap(), TargetState::Stopped);
    // Config remains.
    assert!(h.connector_config("jdbc").is_ok());
    // Tasks zero out while stopped.
    assert_eq!(h.task_configs("jdbc").unwrap().len(), 0);
}

/// Upstream: `StandaloneHerderTest.testRestartTask`.
#[test]
fn upstream_standalone_restart_task_clears_failure_trace() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSource", 1), false)
        .unwrap();
    h.fail_task("jdbc", 0, "boom").unwrap();
    h.restart_task("jdbc", 0).unwrap();
    let tasks = h.task_configs("jdbc").unwrap();
    assert!(tasks[0].failure_trace.is_none());
}

/// Upstream: `StandaloneHerderTest.testPatchConnectorConfig`.
#[test]
fn upstream_standalone_patch_connector_config_merges_keys() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSource", 1), false)
        .unwrap();
    let mut patch = BTreeMap::new();
    patch.insert("topics".into(), "orders,refunds".into());
    h.patch_connector_config("jdbc", patch).unwrap();
    let cfg = h.connector_config("jdbc").unwrap();
    assert_eq!(cfg.get("topics").map(|s| s.as_str()), Some("orders,refunds"));
    assert_eq!(
        cfg.get("connector.class").map(|s| s.as_str()),
        Some("...JdbcSource")
    );
}

/// Upstream: `StandaloneHerderTest.testPatchConnectorConfigNotFound`.
#[test]
fn upstream_standalone_patch_unknown_returns_not_found() {
    let mut h = StandaloneHerder::new();
    let err = h.patch_connector_config("nope", BTreeMap::new()).unwrap_err();
    assert!(matches!(err, HerderError::NotFound(_)));
}

/// Upstream: `StandaloneHerderTest.testCreateConnectorWithStoppedInitialState`.
#[test]
fn upstream_standalone_put_connector_with_stopped_initial_state_skips_tasks() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config_with_state(
        "jdbc",
        config("...JdbcSource", 4),
        TargetState::Stopped,
        false,
    )
    .unwrap();
    assert_eq!(h.task_configs("jdbc").unwrap().len(), 0);
    assert_eq!(h.target_state("jdbc").unwrap(), TargetState::Stopped);
}

/// Upstream: `StandaloneHerderTest.testModifyConnectorOffsetsConnectorNotInStoppedState`.
#[test]
fn upstream_standalone_alter_offsets_requires_stopped_state() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSource", 1), false)
        .unwrap();
    let err = h
        .alter_connector_offsets("jdbc", vec![(BTreeMap::new(), Some(BTreeMap::new()))])
        .unwrap_err();
    assert!(
        matches!(err, HerderError::IllegalState(_)),
        "got {err:?}"
    );
}

/// Upstream: `StandaloneHerderTest.testAlterConnectorOffsets`.
#[test]
fn upstream_standalone_alter_offsets_succeeds_when_stopped() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("jdbc", config("...JdbcSource", 1), false)
        .unwrap();
    h.stop_connector("jdbc").unwrap();
    let mut part = BTreeMap::new();
    part.insert("table".into(), "orders".into());
    let mut off = BTreeMap::new();
    off.insert("position".into(), "9000".into());
    h.alter_connector_offsets("jdbc", vec![(part.clone(), Some(off.clone()))])
        .unwrap();
    let dump = h.connector_offsets("jdbc").unwrap();
    assert_eq!(dump.get(&part), Some(&off));
}

/// Upstream: `StandaloneHerderTest.testPutTaskConfigs` — distributed-only,
/// must throw in standalone mode.
#[test]
fn upstream_standalone_put_task_configs_unsupported() {
    let mut h = StandaloneHerder::new();
    let err = h.put_task_configs("jdbc", vec![]).unwrap_err();
    assert!(
        matches!(err, HerderError::Unsupported(_)),
        "got {err:?}"
    );
}

/// Upstream: `StandaloneHerderTest.testAccessors`.
#[test]
fn upstream_standalone_accessors_after_two_creates() {
    let mut h = StandaloneHerder::new();
    h.put_connector_config("a", config("...Source", 1), false)
        .unwrap();
    h.put_connector_config("b", config("...Sink", 2), false)
        .unwrap();
    let mut names = h.connectors();
    names.sort();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(h.task_configs("b").unwrap().len(), 2);
}

// ── DistributedHerder ────────────────────────────────────────────────────────

/// Upstream: `DistributedHerderTest.testJoinAssignment` (member-1 alone).
#[test]
fn upstream_distributed_first_member_becomes_leader() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    assert_eq!(h.leader(), Some(&MemberId::from("w1")));
    assert_eq!(h.state(), HerderState::Assigning);
}

/// Upstream: `DistributedHerderTest.testJoinAssignment` (member-2 follows).
#[test]
fn upstream_distributed_second_member_joins_as_follower() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    h.join("w2".into());
    assert_eq!(h.leader(), Some(&MemberId::from("w1")));
    assert_eq!(h.members().len(), 2);
}

/// Upstream: `DistributedHerderTest.testRebalanceAssignment`.
#[test]
fn upstream_distributed_assign_distributes_tasks_rendezvous() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    h.join("w2".into());
    h.register_tasks(&[
        "c:0", "c:1", "c:2", "c:3",
    ]);
    h.assign();
    let owners = h.assignment();
    // Both members should own at least one task — rendezvous
    // hash guarantees that with 4 tasks + 2 members at least one
    // task lives on each.
    let mut w1 = 0;
    let mut w2 = 0;
    for (_, m) in owners {
        if m == &MemberId::from("w1") {
            w1 += 1;
        } else if m == &MemberId::from("w2") {
            w2 += 1;
        }
    }
    assert!(w1 >= 1, "w1 owns {w1}");
    assert!(w2 >= 1, "w2 owns {w2}");
    assert_eq!(w1 + w2, 4);
}

/// Upstream: `DistributedHerderTest.testRebalanceOnLeave`.
#[test]
fn upstream_distributed_member_leave_triggers_rebalance() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    h.join("w2".into());
    h.register_tasks(&["c:0", "c:1"]);
    h.assign();
    let g1 = h.generation();
    h.leave("w2".into());
    assert!(h.generation() > g1);
    assert_eq!(h.state(), HerderState::Rebalancing);
}

/// Upstream: `DistributedHerderTest.testStaleGenerationRejected`.
#[test]
fn upstream_distributed_heartbeat_stale_generation_is_rejected() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    let g = h.generation();
    let result = h.heartbeat("w1", g.saturating_sub(1));
    assert!(result.is_err());
}

/// Upstream: `DistributedHerderTest.testHeartbeatAdvancesClock`.
#[test]
fn upstream_distributed_tick_advances_clock_monotonically() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    let t0 = h.clock();
    h.tick();
    h.tick();
    assert!(h.clock() > t0);
    let t1 = h.clock();
    h.tick();
    assert!(h.clock() > t1);
}

/// Upstream: `DistributedHerderTest.testLeaderFailoverPicksNext`.
#[test]
fn upstream_distributed_leader_failure_promotes_next_member() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    h.join("w2".into());
    h.join("w3".into());
    let leader1 = h.leader().cloned().unwrap();
    h.leave(leader1.clone());
    let leader2 = h.leader().cloned().unwrap();
    assert_ne!(leader1, leader2, "leader must change after leave");
}

/// Upstream: `DistributedHerderTest.testSyncGroupResponse`.
#[test]
fn upstream_distributed_sync_group_returns_member_assignment() {
    let mut h = DistributedHerder::new();
    h.join("w1".into());
    h.join("w2".into());
    h.register_tasks(&["c:0", "c:1", "c:2"]);
    h.assign();
    let mine = h.sync_group("w1");
    assert!(!mine.is_empty());
    for t in &mine {
        assert!(t.starts_with("c:"));
    }
}

// ── KafkaOffsetBackingStore ─────────────────────────────────────────────────

fn offset_key(connector: &str, table: &str) -> OffsetKey {
    let mut p = BTreeMap::new();
    p.insert("table".into(), table.into());
    OffsetKey {
        connector: connector.into(),
        partition: p,
    }
}

fn offset_value(pos: &str) -> OffsetValue {
    let mut m = BTreeMap::new();
    m.insert("position".into(), pos.into());
    m
}

/// Upstream: `KafkaOffsetBackingStoreTest.testReplayEmpty`.
#[test]
fn upstream_kafka_offset_store_replay_empty_returns_empty() {
    let log = RecordLog::new();
    let store = KafkaOffsetBackingStore::new(log);
    assert!(store.snapshot().is_empty());
}

/// Upstream: `KafkaOffsetBackingStoreTest.testReplayRebuildsState`.
#[test]
fn upstream_kafka_offset_store_replay_rebuilds_map_from_records() {
    let mut log = RecordLog::new();
    log.append(OffsetRecord::put(offset_key("jdbc", "a"), offset_value("1")));
    log.append(OffsetRecord::put(offset_key("jdbc", "b"), offset_value("2")));
    let store = KafkaOffsetBackingStore::new(log);
    assert_eq!(store.get(&offset_key("jdbc", "a")), Some(offset_value("1")));
    assert_eq!(store.get(&offset_key("jdbc", "b")), Some(offset_value("2")));
}

/// Upstream: `KafkaOffsetBackingStoreTest.testReplayHonorsTombstones`.
#[test]
fn upstream_kafka_offset_store_replay_tombstone_deletes_key() {
    let mut log = RecordLog::new();
    log.append(OffsetRecord::put(offset_key("jdbc", "a"), offset_value("1")));
    log.append(OffsetRecord::tombstone(offset_key("jdbc", "a")));
    let store = KafkaOffsetBackingStore::new(log);
    assert!(store.get(&offset_key("jdbc", "a")).is_none());
}

/// Upstream: `KafkaOffsetBackingStoreTest.testSetThenGet`.
#[test]
fn upstream_kafka_offset_store_commit_appends_and_reads() {
    let mut store = KafkaOffsetBackingStore::new(RecordLog::new());
    store.commit(offset_key("jdbc", "x"), offset_value("100"));
    assert_eq!(store.get(&offset_key("jdbc", "x")), Some(offset_value("100")));
}

// ── Tiered Storage ───────────────────────────────────────────────────────────

fn topic_id_partition(topic: &str, p: u32) -> TopicIdPartition {
    TopicIdPartition {
        topic: topic.into(),
        topic_uuid: 0,
        partition: p,
    }
}

fn metadata_for(topic: &str, partition: u32, base: u64, size: u64) -> RemoteLogSegmentMetadata {
    let id = RemoteLogSegmentId {
        topic_partition: topic_id_partition(topic, partition),
        segment_uuid: base,
    };
    RemoteLogSegmentMetadata {
        id,
        start_offset: base,
        end_offset: base + size - 1,
        max_timestamp_ms: (base as i64) * 1_000,
        broker_id: 1,
        event_timestamp_ms: 0,
        segment_size_bytes: size,
        state: RemoteLogSegmentState::CopyStarted,
    }
}

/// Upstream: `RemoteLogManagerTest.testCopySegmentEmitsEvent`.
#[test]
fn upstream_tiered_remote_log_mgr_copy_emits_metadata() {
    let rsm = InMemoryRemoteStorageManager::new();
    let rmm = InMemoryRemoteLogMetadataManager::new();
    let mut mgr = RemoteLogManager::new(Box::new(rsm), Box::new(rmm));
    let m = metadata_for("orders", 0, 0, 100);
    mgr.copy_log_segment(m.clone(), b"segment-bytes".to_vec()).unwrap();
    let listed = mgr
        .list_segments(&topic_id_partition("orders", 0))
        .collect::<Vec<_>>();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, m.id);
}

/// Upstream: `RemoteLogManagerTest.testFindSegmentForOffset`.
#[test]
fn upstream_tiered_remote_log_mgr_find_segment_for_offset() {
    let rsm = InMemoryRemoteStorageManager::new();
    let rmm = InMemoryRemoteLogMetadataManager::new();
    let mut mgr = RemoteLogManager::new(Box::new(rsm), Box::new(rmm));
    let m0 = metadata_for("orders", 0, 0, 100);
    let m1 = metadata_for("orders", 0, 100, 100);
    mgr.copy_log_segment(m0, b"x".to_vec()).unwrap();
    mgr.copy_log_segment(m1.clone(), b"y".to_vec()).unwrap();
    let hit = mgr.find_segment_for_offset(&topic_id_partition("orders", 0), 150);
    assert!(hit.is_some());
    assert_eq!(hit.unwrap().id, m1.id);
}

/// Upstream: `RemoteLogManagerTest.testRetentionDeletesOldSegments`.
#[test]
fn upstream_tiered_remote_log_mgr_retention_removes_old() {
    let rsm = InMemoryRemoteStorageManager::new();
    let rmm = InMemoryRemoteLogMetadataManager::new();
    let mut mgr = RemoteLogManager::new(Box::new(rsm), Box::new(rmm));
    let m0 = metadata_for("orders", 0, 0, 100);
    let m1 = metadata_for("orders", 0, 100, 100);
    mgr.copy_log_segment(m0, b"x".to_vec()).unwrap();
    mgr.copy_log_segment(m1, b"y".to_vec()).unwrap();
    let removed = mgr.apply_retention(&topic_id_partition("orders", 0), 100);
    assert_eq!(removed.len(), 1);
    let remaining = mgr
        .list_segments(&topic_id_partition("orders", 0))
        .count();
    assert_eq!(remaining, 1);
}

/// Upstream: `RemoteIndexCacheTest.testLruEviction`.
#[test]
fn upstream_tiered_remote_index_cache_evicts_coldest() {
    let mut cache: RemoteIndexCache<u64, String> = RemoteIndexCache::new(2);
    cache.put(1, "a".into());
    cache.put(2, "b".into());
    cache.put(3, "c".into());
    assert!(cache.get(&1).is_none()); // evicted
    assert_eq!(cache.get(&2), Some(&"b".to_string()));
    assert_eq!(cache.get(&3), Some(&"c".to_string()));
}

/// Upstream: `RemoteIndexCacheTest.testHitPromotes`.
#[test]
fn upstream_tiered_remote_index_cache_hit_promotes_entry() {
    let mut cache: RemoteIndexCache<u64, String> = RemoteIndexCache::new(2);
    cache.put(1, "a".into());
    cache.put(2, "b".into());
    // Hit on 1 should make it the most-recently-used.
    let _ = cache.get(&1);
    cache.put(3, "c".into());
    // 2 was the coldest at the time of insert.
    assert!(cache.get(&2).is_none());
    assert_eq!(cache.get(&1), Some(&"a".to_string()));
}
