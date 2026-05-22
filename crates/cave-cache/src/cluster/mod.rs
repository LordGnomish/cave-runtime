// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Redis Cluster — slot routing, gossip, migration, epoch tracking.
//!
//! Ports `src/cluster.c` and `src/cluster_slot_stats.c` from upstream
//! Redis. The local module surface is split:
//!
//! * [`state`] — the original `ClusterState` + `ClusterNode` + CRC16
//!   slot computation that existed pre-batch.
//! * [`slots`] — the 16,384-slot routing table, ownership and
//!   MOVED/ASK redirect computation, plus per-slot stats.
//! * [`gossip`] — message envelope and an in-process message log for
//!   PING/PONG/MEET/FAIL exchange. The on-wire bus protocol of real
//!   Redis runs on its own TCP listener; the cave-cache topology uses
//!   a single REST listener for the data plane, so the gossip layer
//!   here is the *state machine* — what each message does to the
//!   cluster view — without the literal port-16380 listener.
//! * [`migration`] — IMPORTING / MIGRATING slot transitions with the
//!   key-handoff progress tracking that CLUSTER SETSLOT performs.
//! * [`epoch`] — configuration epoch + currentEpoch monotonic counters
//!   matching the upstream tiebreaker semantics.
//!
//! Honest scope:
//! * No on-wire cluster-bus serializer. The gossip layer surfaces the
//!   *messages* and the cluster effect, not the raw byte protocol.
//! * No failover voting at the data plane — cave-cluster's Raft
//!   already handles the equivalent at a higher layer.

pub mod epoch;
pub mod failover;
pub mod gossip;
pub mod migration;
pub mod slots;
pub mod state;

pub use epoch::EpochCounter;
pub use failover::{FailoverMode, FailoverPhase, FailoverState};
pub use gossip::{GossipBus, GossipMessage, GossipMessageKind};
pub use migration::{MigrationLedger, MigrationState, SlotMigration};
pub use slots::{RedirectKind, SlotMap, SlotOwnership, SlotStats};
pub use state::{ClusterNode, ClusterState, ClusterStatus, crc16, generate_node_id, hash_slot};
