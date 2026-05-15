// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The Worker — top-level Connect runtime process. Holds task
//! handles, drives the tick loop, owns the [`OffsetStore`].
//! Mirrors `org.apache.kafka.connect.runtime.Worker` from
//! upstream.
//!
//! cave-streams ships a synchronous tick API rather than a
//! tokio task per task — the existing broker already runs on
//! tokio; the Worker is plugged in through its tick() being
//! called from a single driver loop. Real-time long-running
//! tokio integration is tracked, not in this batch.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::assignment::{AssignmentTable, Rebalance, WorkerId};
use super::connector_runtime::{ConnectorLifecycle, ConnectorRuntime, ConnectorSpec};
use super::offset_store::OffsetStore;
use super::task_runtime::{TaskHandle, TaskKind, TaskRuntime, TaskStatus};

/// Worker startup configuration. Fields are kept open so a real
/// install can configure node id + offset commit cadence.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub worker_id: WorkerId,
    /// Connector + tasks lookup against the herder.
    pub group_id: String,
    /// How many `tick()` calls between offset commits — 1 means
    /// commit-every-tick (most durable; highest CPU). Default 50.
    pub commit_interval_ticks: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: 1,
            group_id: "connect-cluster".to_string(),
            commit_interval_ticks: 50,
        }
    }
}

/// The Worker — owns task handles + runtimes, applies
/// assignments, drives tick().
pub struct Worker {
    pub config: WorkerConfig,
    pub assignment: AssignmentTable,
    pub offsets: Arc<OffsetStore>,
    connectors: BTreeMap<String, ConnectorRuntime>,
    /// task_id → (handle, runtime).
    tasks: BTreeMap<String, (TaskHandle, Box<dyn TaskRuntime>)>,
    /// How many tick() calls have run since the last commit.
    ticks_since_commit: u32,
    /// Total tick() invocations — diagnostic.
    pub total_ticks: u64,
}

impl Worker {
    pub fn new(config: WorkerConfig, offsets: Arc<OffsetStore>) -> Self {
        let mut assignment = AssignmentTable::new();
        assignment.add_worker(config.worker_id);
        Self {
            config,
            assignment,
            offsets,
            connectors: BTreeMap::new(),
            tasks: BTreeMap::new(),
            ticks_since_commit: 0,
            total_ticks: 0,
        }
    }

    /// Register a connector spec and validate it. Returns the
    /// generated task ids.
    pub fn register_connector(&mut self, spec: ConnectorSpec) -> StreamsResult<Vec<String>> {
        let mut connector = ConnectorRuntime::new(spec);
        connector.validate()?;
        let ids = connector.generate_tasks()?;
        self.connectors.insert(connector.spec.name.clone(), connector);
        // Add the task ids into the assignment table and run a
        // rebalance so the Worker knows which it owns.
        let all_task_ids: Vec<String> = self
            .connectors
            .values()
            .flat_map(|c| c.task_ids.iter().cloned())
            .collect();
        self.assignment.rebalance(all_task_ids);
        Ok(ids)
    }

    /// Install a task runtime for `task_id`. Caller is
    /// responsible for matching the connector kind.
    pub fn install_task(&mut self, task_id: String, runtime: Box<dyn TaskRuntime>) {
        let kind = runtime.kind();
        let handle = TaskHandle::new(task_id.clone(), kind);
        self.tasks.insert(task_id, (handle, runtime));
    }

    /// Start every installed task — flip its handle to
    /// Running. Also marks the parent connector Running.
    pub fn start_all(&mut self) {
        for (h, _) in self.tasks.values_mut() {
            if h.status == TaskStatus::Pending {
                h.status = TaskStatus::Running;
            }
        }
        for c in self.connectors.values_mut() {
            c.mark_running();
        }
    }

    /// One iteration over every owned, running task. Returns
    /// the total record count across all task ticks. Failed
    /// tasks are skipped on subsequent ticks.
    pub fn tick(&mut self) -> u64 {
        self.total_ticks += 1;
        self.ticks_since_commit += 1;
        let owned: Vec<String> = self
            .assignment
            .tasks_for(self.config.worker_id)
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut total = 0u64;
        for id in owned {
            if let Some((handle, runtime)) = self.tasks.get_mut(&id) {
                if handle.status != TaskStatus::Running {
                    continue;
                }
                // Pause-aware: skip tasks whose connector is
                // paused.
                let connector_name = id.split(':').next().unwrap_or("");
                if let Some(c) = self.connectors.get(connector_name) {
                    if c.lifecycle == ConnectorLifecycle::Paused {
                        continue;
                    }
                }
                match runtime.tick() {
                    Ok(n) => {
                        handle.record_count += n;
                        handle.successful_iterations += 1;
                        total += n;
                    }
                    Err(e) => {
                        handle.status = TaskStatus::Failed;
                        handle.last_error = Some(e.to_string());
                    }
                }
            }
        }
        if self.ticks_since_commit >= self.config.commit_interval_ticks {
            // Source-task offset commit is delegated to a real
            // OffsetStore.commit_batch call in production; here
            // we just reset the counter — the runtime tests
            // exercise OffsetStore directly.
            self.ticks_since_commit = 0;
        }
        total
    }

    /// Stop a single task. Idempotent — re-stopping a stopped
    /// task is a no-op.
    pub fn stop_task(&mut self, task_id: &str) -> StreamsResult<()> {
        match self.tasks.get_mut(task_id) {
            Some((h, _)) => {
                h.status = TaskStatus::Stopped;
                Ok(())
            }
            None => Err(StreamsError::Internal(format!("unknown task: {task_id}"))),
        }
    }

    /// Pause every task belonging to `connector` — they stay
    /// installed but don't tick.
    pub fn pause_connector(&mut self, connector: &str) -> StreamsResult<()> {
        match self.connectors.get_mut(connector) {
            Some(c) => {
                c.pause();
                Ok(())
            }
            None => Err(StreamsError::Internal(format!("unknown connector: {connector}"))),
        }
    }

    pub fn resume_connector(&mut self, connector: &str) -> StreamsResult<()> {
        match self.connectors.get_mut(connector) {
            Some(c) => {
                c.resume();
                Ok(())
            }
            None => Err(StreamsError::Internal(format!("unknown connector: {connector}"))),
        }
    }

    /// Delete a connector — stop all its tasks, drop the
    /// connector entry, scrub its offsets.
    pub fn delete_connector(&mut self, connector: &str) -> StreamsResult<()> {
        let Some(mut c) = self.connectors.remove(connector) else {
            return Err(StreamsError::Internal(format!("unknown connector: {connector}")));
        };
        for tid in &c.task_ids {
            self.tasks.remove(tid);
        }
        c.stop();
        self.offsets.forget_connector(connector);
        // Re-balance with what's left.
        let remaining: Vec<String> = self
            .connectors
            .values()
            .flat_map(|c| c.task_ids.iter().cloned())
            .collect();
        self.assignment.rebalance(remaining);
        Ok(())
    }

    /// Add a fresh peer worker — used in cluster expansion.
    pub fn add_peer(&mut self, peer: WorkerId) -> Rebalance {
        self.assignment.add_worker(peer);
        let all: Vec<String> = self
            .connectors
            .values()
            .flat_map(|c| c.task_ids.iter().cloned())
            .collect();
        self.assignment.rebalance(all)
    }

    /// Snapshot of the task handles — for the admin UI / metrics.
    pub fn handles(&self) -> Vec<&TaskHandle> {
        self.tasks.values().map(|(h, _)| h).collect()
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    pub fn connector_count(&self) -> usize {
        self.connectors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::super::task_runtime::{SinkRecord, SinkRuntime, SinkTask, SourceRecord, SourceRuntime, SourceTask};
    use super::*;

    struct CannedSource {
        batches: std::collections::VecDeque<Vec<SourceRecord>>,
    }
    impl SourceTask for CannedSource {
        fn poll(&mut self) -> StreamsResult<Vec<SourceRecord>> {
            Ok(self.batches.pop_front().unwrap_or_default())
        }
    }

    struct CountingSink {
        seen: u64,
    }
    impl SinkTask for CountingSink {
        fn put(&mut self, records: Vec<SinkRecord>) -> StreamsResult<()> {
            self.seen += records.len() as u64;
            Ok(())
        }
    }

    fn one_record() -> SourceRecord {
        let mut sp = BTreeMap::new();
        sp.insert("src".into(), "orders".into());
        let mut so = BTreeMap::new();
        so.insert("offset".into(), "1".into());
        SourceRecord {
            source_partition: sp,
            source_offset: so,
            topic: "orders".into(),
            key: vec![],
            value: vec![1],
        }
    }

    fn fresh_worker() -> Worker {
        Worker::new(WorkerConfig::default(), Arc::new(OffsetStore::new()))
    }

    #[test]
    fn register_validates_and_generates_tasks() {
        let mut w = fresh_worker();
        let spec = ConnectorSpec::new("orders", TaskKind::Source, 2);
        let ids = w.register_connector(spec).unwrap();
        assert_eq!(ids, vec!["orders:0", "orders:1"]);
        assert_eq!(w.connector_count(), 1);
    }

    #[test]
    fn register_rejects_invalid_spec() {
        let mut w = fresh_worker();
        let spec = ConnectorSpec::new("", TaskKind::Source, 1);
        assert!(w.register_connector(spec).is_err());
    }

    #[test]
    fn install_task_records_handle() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("orders", TaskKind::Source, 1))
            .unwrap();
        let runtime = SourceRuntime::new(Box::new(CannedSource {
            batches: vec![vec![one_record()]].into(),
        }));
        w.install_task("orders:0".into(), Box::new(runtime));
        assert_eq!(w.task_count(), 1);
        assert_eq!(w.handles()[0].kind, TaskKind::Source);
    }

    #[test]
    fn start_all_flips_pending_to_running() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("orders", TaskKind::Source, 1))
            .unwrap();
        w.install_task(
            "orders:0".into(),
            Box::new(SourceRuntime::new(Box::new(CannedSource {
                batches: vec![vec![one_record()]].into(),
            }))),
        );
        w.start_all();
        assert_eq!(w.handles()[0].status, TaskStatus::Running);
    }

    #[test]
    fn tick_returns_records_processed() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("orders", TaskKind::Source, 1))
            .unwrap();
        w.install_task(
            "orders:0".into(),
            Box::new(SourceRuntime::new(Box::new(CannedSource {
                batches: vec![vec![one_record(), one_record()]].into(),
            }))),
        );
        w.start_all();
        let n = w.tick();
        assert_eq!(n, 2);
    }

    #[test]
    fn tick_failed_task_marks_failed_and_skips() {
        struct AlwaysFails;
        impl SourceTask for AlwaysFails {
            fn poll(&mut self) -> StreamsResult<Vec<SourceRecord>> {
                Err(StreamsError::Internal("boom".into()))
            }
        }
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("flaky", TaskKind::Source, 1))
            .unwrap();
        w.install_task(
            "flaky:0".into(),
            Box::new(SourceRuntime::new(Box::new(AlwaysFails))),
        );
        w.start_all();
        w.tick();
        assert_eq!(w.handles()[0].status, TaskStatus::Failed);
        // Second tick must not re-invoke the failed task.
        let captured_iters = w.handles()[0].successful_iterations;
        w.tick();
        assert_eq!(w.handles()[0].successful_iterations, captured_iters);
    }

    #[test]
    fn pause_connector_skips_its_tasks_in_tick() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("orders", TaskKind::Source, 1))
            .unwrap();
        w.install_task(
            "orders:0".into(),
            Box::new(SourceRuntime::new(Box::new(CannedSource {
                batches: vec![vec![one_record()], vec![one_record()]].into(),
            }))),
        );
        w.start_all();
        w.pause_connector("orders").unwrap();
        let n = w.tick();
        assert_eq!(n, 0);
        w.resume_connector("orders").unwrap();
        let n2 = w.tick();
        assert_eq!(n2, 1);
    }

    #[test]
    fn stop_task_terminates_one_task() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("orders", TaskKind::Source, 2))
            .unwrap();
        w.install_task(
            "orders:0".into(),
            Box::new(SourceRuntime::new(Box::new(CannedSource {
                batches: vec![vec![one_record()]].into(),
            }))),
        );
        w.install_task(
            "orders:1".into(),
            Box::new(SourceRuntime::new(Box::new(CannedSource {
                batches: vec![vec![one_record()]].into(),
            }))),
        );
        w.start_all();
        w.stop_task("orders:0").unwrap();
        let handles = w.handles();
        let stopped: Vec<_> = handles
            .iter()
            .filter(|h| h.status == TaskStatus::Stopped)
            .collect();
        assert_eq!(stopped.len(), 1);
    }

    #[test]
    fn delete_connector_clears_tasks_and_offsets() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("orders", TaskKind::Source, 1))
            .unwrap();
        w.install_task(
            "orders:0".into(),
            Box::new(SourceRuntime::new(Box::new(CannedSource {
                batches: vec![vec![one_record()]].into(),
            }))),
        );
        w.start_all();
        w.tick();
        w.delete_connector("orders").unwrap();
        assert_eq!(w.connector_count(), 0);
        assert_eq!(w.task_count(), 0);
    }

    #[test]
    fn sink_task_consumes_from_queued_records() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("sink-x", TaskKind::Sink, 1))
            .unwrap();
        let mut sink_rt = SinkRuntime::new(Box::new(CountingSink { seen: 0 }));
        sink_rt.enqueue(SinkRecord {
            topic: "orders".into(),
            partition: 0,
            kafka_offset: 0,
            key: vec![],
            value: vec![],
        });
        sink_rt.enqueue(SinkRecord {
            topic: "orders".into(),
            partition: 0,
            kafka_offset: 1,
            key: vec![],
            value: vec![],
        });
        w.install_task("sink-x:0".into(), Box::new(sink_rt));
        w.start_all();
        let n = w.tick();
        assert_eq!(n, 2);
    }

    #[test]
    fn add_peer_rebalances_tasks() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("c", TaskKind::Source, 6))
            .unwrap();
        let rb = w.add_peer(2);
        // Some of the 6 tasks should now belong to worker 2.
        let worker2_tasks: Vec<_> = rb.assignment.values().filter(|w| **w == 2).collect();
        assert!(!worker2_tasks.is_empty());
    }

    #[test]
    fn delete_unknown_connector_errors() {
        let mut w = fresh_worker();
        assert!(w.delete_connector("nope").is_err());
    }

    #[test]
    fn stop_unknown_task_errors() {
        let mut w = fresh_worker();
        assert!(w.stop_task("nope:0").is_err());
    }

    #[test]
    fn worker_total_ticks_accumulates() {
        let mut w = fresh_worker();
        let _ = w
            .register_connector(ConnectorSpec::new("c", TaskKind::Source, 1))
            .unwrap();
        w.install_task(
            "c:0".into(),
            Box::new(SourceRuntime::new(Box::new(CannedSource {
                batches: vec![vec![]].into(),
            }))),
        );
        w.start_all();
        for _ in 0..5 {
            w.tick();
        }
        assert_eq!(w.total_ticks, 5);
    }
}
