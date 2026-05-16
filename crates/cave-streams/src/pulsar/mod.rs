// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors

//! Apache Pulsar parity surface — segmented storage, subscription policies,
//! topic compaction, message deduplication, Functions runtime skeleton,
//! geo-replication, and the extended schema registry.
//!
//! Per ADR-RUNTIME-STREAMING-CONSOLIDATION-001 the Pulsar and Kafka
//! surfaces live in a single crate (`cave-streams`).  This module is
//! the Pulsar-only home; the Pulsar wire protocol + admin REST live in
//! the crate root (`pulsar_wire.rs`, `pulsar_admin.rs`, ...).
//!
//! Upstream reference: Apache Pulsar v4.2.0
//! (`1940aebc6ade10050399cd65f870353eedf80008`).
//!
//! Submodules:
//! - [`ledger`] — BookKeeper-style append-only ledger + multi-bookie
//!   ensemble with (E, Qw, Qa) quorum semantics.
//! - [`managed_ledger`] — `ManagedLedger` over a chain of ledgers
//!   (`org.apache.bookkeeper.mledger.ManagedLedger`).
//! - [`subscription`] — Exclusive / Shared / Failover / Key_Shared
//!   policy traits + `KeySharedMode::AutoSplit` and `Sticky`.
//! - [`compaction`] — Periodic topic compactor + `Reader.read_compacted`
//!   view.
//! - [`dedup`] — Producer-name + sequence-id dedup ledger that bolts
//!   into `pulsar_dispatch.rs` via the [`dedup::DedupHook`] trait.
//! - [`functions`] — Pulsar Functions skeleton (`Function` trait +
//!   `FunctionInstance` state machine + `FunctionWorker` registry).
//! - [`replicator`] — Per-(source-topic, remote-cluster) `Replicator`
//!   producing to remote with `__replication` cursor checkpoint.
//! - [`schema`] — Schema-type expansions: Avro / JSON / Protobuf /
//!   Key-Value, with BACKWARD / FORWARD / FULL compatibility checks.

pub mod compaction;
pub mod dedup;
pub mod functions;
pub mod ledger;
pub mod managed_ledger;
pub mod replicator;
pub mod schema;
pub mod subscription;
