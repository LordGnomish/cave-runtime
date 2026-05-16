// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// core/src/main/scala/kafka/server/group/GroupCoordinator.scala
// core/src/main/scala/kafka/coordinator/group/ConsumerGroupCoordinator.scala
// clients/src/main/java/org/apache/kafka/common/message/ConsumerGroupHeartbeatRequest.json
// clients/src/main/java/org/apache/kafka/common/message/ConsumerGroupHeartbeatResponse.json
//
//! KIP-848 — Next-Generation Consumer Rebalance Protocol.
//!
//! In KIP-848 the broker computes the target assignment itself and
//! streams it to clients via the `ConsumerGroupHeartbeat` (CGH) RPC;
//! there is no more JoinGroup / SyncGroup phase. Clients converge to
//! the target assignment incrementally, similar to cooperative
//! rebalance but driven entirely by the coordinator.
//!
//! Wire RPCs (API keys 68 / 69):
//!   * `ConsumerGroupHeartbeatRequest`  — member → coordinator
//!   * `ConsumerGroupHeartbeatResponse` — coordinator → member
//!
//! Persistence model (records written to `__consumer_offsets`):
//!   * [`ConsumerGroupRecord`]        — group-level metadata (epoch, partition_metadata_keys)
//!   * [`MemberRecord`]               — per-member subscription + epoch
//!   * [`TargetAssignmentRecord`]     — coordinator-computed target per member
//!
//! Client opt-in: `protocol_version` field on the heartbeat. `1` opts
//! into KIP-848. Anything `<= 0` keeps the legacy classic-rebalance
//! path (see `crate::consumer_group::GroupCoordinator`).

pub mod assignor;
pub mod coordinator;
pub mod records;
pub mod wire;

pub use assignor::{TargetAssignmentBuilder, UniformAssignor};
pub use coordinator::{ConsumerGroupCoordinator, ConsumerGroupSummary};
pub use records::{
    ConsumerGroupRecord, MemberRecord, MemberSubscription, TargetAssignmentRecord, TopicPartitions,
};
pub use wire::{ConsumerGroupHeartbeatRequest, ConsumerGroupHeartbeatResponse, HeartbeatErrorCode};
