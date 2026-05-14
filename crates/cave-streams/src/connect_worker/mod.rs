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
//!
//! ## Honest scope
//!
//! * No actual TCP poll/sink (kafka producer/consumer adapter is
//!   the broker side); the runtime is exercised via in-memory
//!   `SourceTask` / `SinkTask` impls in tests.
//! * No SMT (Single Message Transform) chain — the existing
//!   `connect::Transform` config is preserved but transforms are
//!   not applied to records in this runtime.
//! * No DLQ (dead-letter queue) routing — runtime errors fail
//!   the task rather than rerouting.

pub mod assignment;
pub mod connector_runtime;
pub mod distributed_herder;
pub mod kafka_offset_backing_store;
pub mod offset_store;
pub mod standalone_herder;
pub mod task_runtime;
pub mod worker;

pub use assignment::{AssignmentTable, Rebalance, WorkerId};
pub use connector_runtime::{ConnectorRuntime, ConnectorSpec};
pub use distributed_herder::{DistributedHerder, HerderState, MemberId};
pub use kafka_offset_backing_store::{KafkaOffsetBackingStore, OffsetRecord, RecordLog};
pub use offset_store::{OffsetBackingStore, OffsetKey, OffsetStore, OffsetValue};
pub use standalone_herder::{ConnectorInfo, HerderError, StandaloneHerder, TargetState, TaskInfo};
pub use task_runtime::{
    SinkRecord, SinkTask, SourceRecord, SourceTask, TaskHandle, TaskKind, TaskRuntime, TaskStatus,
};
pub use worker::{Worker, WorkerConfig};
