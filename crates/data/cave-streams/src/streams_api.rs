// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Streams API — stateless and stateful stream transformations.
//!
//! Provides a fluent DSL for defining stream processing pipelines:
//!
//! ```text
//! StreamPipeline::from("orders")
//!     .filter(|r| r.value_is("status", "PAID"))
//!     .map(|r| r.add_header("enriched", "true"))
//!     .group_by_key(|r| r.header("customer_id"))
//!     .count(None)
//!     .to("order-counts")
//! ```
//!
//! Pipelines are described as a [`StreamPipelineConfig`] (serialisable) and
//! stored in the registry.  Execution happens in a background Tokio task.

use crate::error::{StreamError, StreamResult};
use crate::models::{
    AggregationType, PipelineState, StreamOperation, StreamPipelineConfig,
};
use crate::storage::StreamStorage;
use uuid::Uuid;

// ─── Pipeline builder ─────────────────────────────────────────────────────────

/// Fluent builder that accumulates operations and produces a
/// [`StreamPipelineConfig`].
pub struct StreamPipelineBuilder {
    name: String,
    source_topic: String,
    sink_topic: Option<String>,
    operations: Vec<StreamOperation>,
}

impl StreamPipelineBuilder {
    /// Start a pipeline that reads from `source_topic`.
    pub fn from(source_topic: impl Into<String>) -> Self {
        Self {
            name: format!("pipeline-{}", Uuid::new_v4()),
            source_topic: source_topic.into(),
            sink_topic: None,
            operations: Vec::new(),
        }
    }

    /// Set a human-readable name.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    // ── Stateless operations ──────────────────────────────────────────────────

    /// Transform each record value via an expression.
    pub fn map(mut self, expression: impl Into<String>) -> Self {
        self.operations.push(StreamOperation::Map {
            expression: expression.into(),
        });
        self
    }

    /// Keep only records that match a predicate expression.
    pub fn filter(mut self, predicate: impl Into<String>) -> Self {
        self.operations.push(StreamOperation::Filter {
            predicate: predicate.into(),
        });
        self
    }

    /// Expand each record into zero or more output records.
    pub fn flat_map(mut self, expression: impl Into<String>) -> Self {
        self.operations.push(StreamOperation::FlatMap {
            expression: expression.into(),
        });
        self
    }

    // ── Key grouping ──────────────────────────────────────────────────────────

    /// Re-key the stream for subsequent stateful operations.
    pub fn group_by_key(mut self, key_expression: impl Into<String>) -> Self {
        self.operations.push(StreamOperation::GroupBy {
            key_expression: key_expression.into(),
        });
        self
    }

    // ── Stateful operations ───────────────────────────────────────────────────

    /// Count records per key, optionally within a tumbling window.
    pub fn count(mut self, window_ms: Option<i64>) -> Self {
        self.operations.push(StreamOperation::Count { window_ms });
        self
    }

    /// Aggregate records per key using a built-in aggregation function.
    pub fn aggregate(mut self, agg: AggregationType, window_ms: Option<i64>) -> Self {
        self.operations.push(StreamOperation::Aggregate {
            aggregation: agg,
            window_ms,
        });
        self
    }

    /// Reduce records per key using a custom folding expression.
    pub fn reduce(mut self, expression: impl Into<String>, window_ms: Option<i64>) -> Self {
        self.operations.push(StreamOperation::Reduce {
            expression: expression.into(),
            window_ms,
        });
        self
    }

    /// Windowed join with another stream.
    pub fn join(
        mut self,
        right_topic: impl Into<String>,
        window_ms: i64,
        join_key: impl Into<String>,
    ) -> Self {
        self.operations.push(StreamOperation::Join {
            right_topic: right_topic.into(),
            window_ms,
            join_key: join_key.into(),
        });
        self
    }

    // ── Sink ──────────────────────────────────────────────────────────────────

    /// Write output to a sink topic.
    pub fn to(mut self, sink_topic: impl Into<String>) -> Self {
        self.sink_topic = Some(sink_topic.into());
        self
    }

    /// Finalise the builder into a storable config.
    pub fn build(self) -> StreamPipelineConfig {
        StreamPipelineConfig {
            id: Uuid::new_v4(),
            name: self.name,
            source_topic: self.source_topic,
            sink_topic: self.sink_topic,
            operations: self.operations,
            state: PipelineState::Created,
        }
    }
}

// ─── Pipeline registry ────────────────────────────────────────────────────────

/// Manages pipeline configs and their lifecycle in storage.
pub struct PipelineRegistry<S: StreamStorage> {
    storage: S,
}

impl<S: StreamStorage> PipelineRegistry<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    pub fn create(&self, cfg: StreamPipelineConfig) -> StreamResult<StreamPipelineConfig> {
        self.storage.create_pipeline(cfg.clone())?;
        Ok(cfg)
    }

    pub fn get(&self, id: Uuid) -> StreamResult<StreamPipelineConfig> {
        self.storage
            .get_pipeline(id)?
            .ok_or_else(|| StreamError::PipelineNotFound(id.to_string()))
    }

    pub fn list(&self) -> StreamResult<Vec<StreamPipelineConfig>> {
        self.storage.list_pipelines()
    }

    pub fn start(&self, id: Uuid) -> StreamResult<()> {
        let mut cfg = self.get(id)?;
        cfg.state = PipelineState::Running;
        self.storage.update_pipeline(cfg)
    }

    pub fn pause(&self, id: Uuid) -> StreamResult<()> {
        let mut cfg = self.get(id)?;
        cfg.state = PipelineState::Paused;
        self.storage.update_pipeline(cfg)
    }

    pub fn stop(&self, id: Uuid) -> StreamResult<()> {
        let mut cfg = self.get(id)?;
        cfg.state = PipelineState::Stopped;
        self.storage.update_pipeline(cfg)
    }

    pub fn delete(&self, id: Uuid) -> StreamResult<()> {
        self.storage.delete_pipeline(id)
    }
}

// ─── In-process executor ──────────────────────────────────────────────────────

/// Execute a pipeline config synchronously against a storage backend.
///
/// In production this would run in a background Tokio task.  This function
/// processes one "batch" of records from the source topic and writes results
/// to the sink topic.
pub fn execute_batch<S: StreamStorage>(
    cfg: &StreamPipelineConfig,
    storage: &S,
    fetch_offset: i64,
    max_records: usize,
) -> StreamResult<ExecutionResult> {
    // Read the source topic (partition 0 for simplicity).
    let source_records =
        storage.fetch_from_partition(&cfg.source_topic, 0, fetch_offset, max_records)?;

    let mut output: Vec<crate::models::Record> = source_records;

    // Apply each operation in order.
    for op in &cfg.operations {
        output = apply_operation(op, output)?;
    }

    // Write to sink if configured.
    let mut offsets_written = Vec::new();
    if let Some(ref sink) = cfg.sink_topic {
        for record in &output {
            let mut sink_record = record.clone();
            sink_record.topic = sink.clone();
            let offset = storage.append_to_partition(sink, 0, sink_record)?;
            offsets_written.push(offset);
        }
    }

    Ok(ExecutionResult {
        records_processed: output.len(),
        records_emitted: offsets_written.len(),
        offsets_written,
    })
}

fn apply_operation(
    op: &StreamOperation,
    records: Vec<crate::models::Record>,
) -> StreamResult<Vec<crate::models::Record>> {
    match op {
        StreamOperation::Map { expression } => {
            // In production: evaluate `expression` as a template/jq/CEL expression.
            // Here we simply add a header recording the map step.
            Ok(records
                .into_iter()
                .map(|mut r| {
                    r.headers.push(crate::models::Header {
                        key: "cave.map".into(),
                        value: expression.as_bytes().to_vec(),
                    });
                    r
                })
                .collect())
        }

        StreamOperation::Filter { predicate } => {
            // In production: evaluate predicate expression.
            // Here we keep records whose value contains the predicate string.
            Ok(records
                .into_iter()
                .filter(|r| {
                    if predicate == "*" || predicate.is_empty() {
                        return true;
                    }
                    r.value
                        .as_deref()
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .map(|s| s.contains(predicate.as_str()))
                        .unwrap_or(false)
                })
                .collect())
        }

        StreamOperation::FlatMap { expression } => {
            // Simplified: each record is emitted once (no splitting logic).
            Ok(records
                .into_iter()
                .map(|mut r| {
                    r.headers.push(crate::models::Header {
                        key: "cave.flatmap".into(),
                        value: expression.as_bytes().to_vec(),
                    });
                    r
                })
                .collect())
        }

        StreamOperation::GroupBy { key_expression } => {
            // Re-key: set key to the result of evaluating key_expression.
            Ok(records
                .into_iter()
                .map(|mut r| {
                    if r.key.is_none() {
                        r.key = Some(key_expression.as_bytes().to_vec());
                    }
                    r
                })
                .collect())
        }

        StreamOperation::Count { .. } => {
            // Emit a single record with the count as a JSON value.
            let count = records.len();
            if count == 0 {
                return Ok(Vec::new());
            }
            let mut out = records.into_iter().next().unwrap();
            out.value = Some(format!(r#"{{"count":{count}}}"#).into_bytes());
            Ok(vec![out])
        }

        StreamOperation::Aggregate { aggregation, .. } => {
            aggregate_records(records, aggregation)
        }

        StreamOperation::Reduce { expression, .. } => {
            // Simplified: pass through with a reduce header.
            Ok(records
                .into_iter()
                .map(|mut r| {
                    r.headers.push(crate::models::Header {
                        key: "cave.reduce".into(),
                        value: expression.as_bytes().to_vec(),
                    });
                    r
                })
                .collect())
        }

        StreamOperation::Join { right_topic, .. } => {
            // Pass-through with join metadata header.
            Ok(records
                .into_iter()
                .map(|mut r| {
                    r.headers.push(crate::models::Header {
                        key: "cave.join.right".into(),
                        value: right_topic.as_bytes().to_vec(),
                    });
                    r
                })
                .collect())
        }
    }
}

fn aggregate_records(
    records: Vec<crate::models::Record>,
    agg: &AggregationType,
) -> StreamResult<Vec<crate::models::Record>> {
    if records.is_empty() {
        return Ok(Vec::new());
    }

    let values: Vec<i64> = records
        .iter()
        .filter_map(|r| {
            r.value
                .as_deref()
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|s| s.parse::<i64>().ok())
        })
        .collect();

    let result: Option<i64> = match agg {
        AggregationType::Sum => Some(values.iter().sum()),
        AggregationType::Min => values.iter().copied().reduce(i64::min),
        AggregationType::Max => values.iter().copied().reduce(i64::max),
        AggregationType::Average => {
            if values.is_empty() {
                None
            } else {
                Some(values.iter().sum::<i64>() / values.len() as i64)
            }
        }
        AggregationType::First => values.first().copied(),
        AggregationType::Last => values.last().copied(),
        AggregationType::Collect => {
            let json = serde_json::to_string(&values).unwrap_or_default();
            let mut out = records.into_iter().next().unwrap();
            out.value = Some(json.into_bytes());
            return Ok(vec![out]);
        }
    };

    let mut out = records.into_iter().next().unwrap();
    out.value = result.map(|v| v.to_string().into_bytes());
    Ok(vec![out])
}

// ─── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub records_processed: usize,
    pub records_emitted: usize,
    pub offsets_written: Vec<i64>,
}
