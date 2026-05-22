// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KRaft mode — Kafka's ZooKeeper-replacement metadata quorum.
//!
//! Upstream: `apache/kafka` raft/ + metadata/ + controller/
//! (KIP-500 + KIP-595 + KIP-631). cave-streams runs a single
//! Rust broker today; this module adds the metadata-state-machine
//! surface that KRaft mode needs so the broker can self-host its
//! cluster metadata instead of leaning on an external store.
//!
//! ## Layout
//!
//! * [`metadata`] — `MetadataRecord` types (TopicRecord,
//!   PartitionRecord, BrokerRegistration, ConfigRecord), each
//!   modelling one entry that lands in the compacted metadata
//!   log.
//! * [`metadata_log`] — append-only + compacted log of
//!   `MetadataRecord`s. Single source of truth for cluster
//!   metadata; offset + epoch tagged.
//! * [`quorum_controller`] — state machine that owns the
//!   metadata log. Accepts requests (`CreateTopic`,
//!   `RegisterBroker`, `UpdateConfig`), validates them, and
//!   emits the corresponding `MetadataRecord` if accepted.
//! * [`epoch`] — controller-epoch monotonic counter + the
//!   leader/voter set type the raft layer hands the controller
//!   on election.
//!
//! ## Honest limitations
//!
//! * **No replication transport.** The state machine is in
//!   place but the actual Raft consensus (vote / append /
//!   heartbeat) is delegated to a future `RaftTransport` trait —
//!   see [`quorum_controller::QuorumController::new`]. cave-etcd
//!   already has a working raft implementation; wiring it in is
//!   tracked but not landed here.
//! * **No on-disk snapshots.** KIP-630 compacted snapshots are
//!   tracked-not-shipped. The metadata log compacts in-memory
//!   by record-key, which is the semantics-correct behavior for
//!   a single-node controller.
//! * **No KRaft RPC endpoints.** KIP-595 introduced
//!   `Vote`/`BeginQuorumEpoch`/`EndQuorumEpoch`/`Fetch` over the
//!   Kafka wire. Those are added by the existing
//!   `kafka_wire`/`kafka_protocol` layer once the transport
//!   plugs in. Library-only for now.

pub mod epoch;
pub mod metadata;
pub mod metadata_log;
pub mod quorum_controller;
pub mod rpc;

pub use epoch::{ControllerEpoch, VoterSet};
pub use metadata::{BrokerRegistration, MetadataKey, MetadataRecord, PartitionRecord, TopicRecord};
pub use metadata_log::MetadataLog;
pub use quorum_controller::{ControllerRequest, ControllerResponse, QuorumController};
pub use rpc::{
    BeginQuorumEpochRequest, BeginQuorumEpochResponse, DescribeQuorumRequest,
    DescribeQuorumResponse, EndQuorumEpochRequest, EndQuorumEpochResponse, KraftHandler,
    VoteRequest, VoteResponse,
};
