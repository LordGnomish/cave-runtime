// SPDX-License-Identifier: AGPL-3.0-or-later
//! Kafka Connect Worker runtime — the per-process side of the
//! Connect framework. Mirrors `connect/runtime/` from upstream
//! (`Worker.java`, `WorkerTask`, `OffsetStorage*`,
//! `DistributedHerder`). cave-streams' existing
//! [`crate::connect`] module owns the *state model* (Connector,
//! Task, ConnectCluster API); this module owns the *runtime*
//! (lifecycle, polling loops, offset commit, task assignment).
//!
//! ## Module layout
//!
//! * [`task_runtime`]      — `TaskRuntime` trait + source/sink
//!   poll() / put() lifecycle.
//! * [`offset_store`]      — durable offset commits keyed by
//!   `(connector, partition)`; mirrors
//!   `OffsetStorageReader/Writer`.
//! * [`assignment`]        — distributed task assignment table;
//!   mirrors `DistributedHerder.assign()`.
//! * [`worker`]            — `Worker` struct that owns task
//!   handles + drives the runtime loops.
//! * [`connector_runtime`] — `ConnectorRuntime` — start/stop a
//!   connector and emit its task generation.
//! * [`assignor_incremental`] — KIP-415 cooperative-incremental
//!   Connect assignor.
//! * [`worker_coordinator`]   — Connect-side group-coordinator state
//!   machine: JoinGroup/SyncGroup/Heartbeat with eager vs.
//!   incremental subprotocol negotiation.
//! * [`offset_backing_kafka`] — Real Kafka-topic-backed
//!   OffsetBackingStore (compacted offsets topic adapter).
//! * [`plugin_registry`]      — Plugin classloader isolation
//!   equivalent for native Rust plugins.
//! * [`rest_extension`]       — Connect REST Extension API + filter
//!   chain.
//! * [`smt`]                  — Single Message Transform registry
//!   + 10 built-in transforms.

pub mod assignment;
pub mod assignor_incremental;
pub mod connector_runtime;
pub mod distributed_herder;
pub mod kafka_offset_backing_store;
pub mod offset_backing_kafka;
pub mod offset_store;
pub mod plugin_registry;
pub mod rest_extension;
pub mod smt;
pub mod standalone_herder;
pub mod task_runtime;
pub mod worker;
pub mod worker_coordinator;

pub use assignment::{AssignmentTable, Rebalance, WorkerId};
pub use assignor_incremental::{
    ConnectAssignmentDelta, IncrementalConnectAssignor, IncrementalConnectorAssignment,
    PreviousAssignment,
};
pub use connector_runtime::{ConnectorRuntime, ConnectorSpec};
pub use distributed_herder::{DistributedHerder, HerderState, MemberId};
pub use kafka_offset_backing_store::{KafkaOffsetBackingStore, OffsetRecord, RecordLog};
pub use offset_backing_kafka::{
    InMemoryOffsetTopicClient, KafkaBackedOffsetStore, KafkaOffsetTopicClient,
    OffsetTopicRecord,
};
pub use offset_store::{OffsetBackingStore, OffsetKey, OffsetStore, OffsetValue};
pub use plugin_registry::{ConnectorFactory, PluginEntry, PluginRegistry, PluginVersion};
pub use rest_extension::{
    ConnectRestExtension, ExtensionContext, RestExtensionFilter, RestExtensionRegistry,
    RestFilterChain, RestRequestCtx,
};
pub use smt::{
    global_smt_registry, RecordEnvelope as SmtRecordEnvelope, Smt, SmtChain, SmtRegistry,
    Value as SmtValue,
};
pub use standalone_herder::{ConnectorInfo, HerderError, StandaloneHerder, TargetState, TaskInfo};
pub use task_runtime::{
    SinkRecord, SinkTask, SourceRecord, SourceTask, TaskHandle, TaskKind, TaskRuntime, TaskStatus,
};
pub use worker::{Worker, WorkerConfig};
pub use worker_coordinator::{
    CoordinatorEvent, CoordinatorState, JoinGroupRequest, JoinGroupResponse, RebalanceMode,
    SyncGroupRequest, SyncGroupResponse, WorkerCoordinator,
};
