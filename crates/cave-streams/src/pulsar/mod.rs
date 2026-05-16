// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//         (v4.2.0 — top-level umbrella module)
//
//! Pulsar advanced subsystem (S3 close-out).
//!
//! New, S3-introduced Pulsar machinery lives in this subdirectory under
//! topic-specific submodules.  Existing flat `pulsar_*.rs` files
//! (`pulsar_wire`, `pulsar_admin`, `pulsar_dispatch`, `pulsar_topic`)
//! remain at the crate root; their migration into this subdirectory is
//! a Phase-2 backlog item — moving them right now would touch a wide
//! re-export surface and is out of scope.
//!
//! Submodules:
//!
//! * [`bookkeeper`] — segmented, write-once, quorum-replicated ledger
//!   modelled after Apache BookKeeper 4.17.1.
//! * [`subscription`] — full Pulsar subscription-type semantics
//!   (Exclusive / Failover / Shared / Key_Shared with `AUTO_SPLIT` and
//!   `STICKY` sub-modes) including nack/redelivery and ack-mode rules
//!   that `pulsar_dispatch` does not cover.
//! * [`functions`] — in-process Pulsar Functions runtime (Thread +
//!   Process), with a deterministic `Context` for testing.
//! * [`geo_replication`] — per-cluster, per-topic replicator with an
//!   independent `__replication.{remote_cluster}` cursor and bounded
//!   retry / alarm metric path.
//! * [`schema`] — Pulsar-specific schema registry: `__schema`
//!   compacted topic per namespace, full compatibility-check matrix
//!   (`FULL`/`BACKWARD`/`FORWARD`/`NONE` plus `_TRANSITIVE` variants),
//!   `KEY_VALUE` composite schemas, and auto schema inference.
//! * [`compaction`] — per-key latest-value topic compaction
//!   (`CompactedTopic::compact`, `CompactedTopicReader`) plus broker-side
//!   message deduplication keyed by `(producer_id, sequence_id)` with
//!   8-byte unsigned sequence-id rollover handling.

pub mod bookkeeper;
pub mod compaction;
pub mod functions;
pub mod geo_replication;
pub mod schema;
pub mod subscription;
