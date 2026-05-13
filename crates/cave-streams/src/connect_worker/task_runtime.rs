//! Task lifecycle types — the records a SourceTask emits and a
//! SinkTask consumes, plus the trait every connector
//! implementation hooks into.
//!
//! Mirrors `org.apache.kafka.connect.runtime.WorkerTask` +
//! `SourceTask` / `SinkTask` from upstream.

use std::collections::BTreeMap;

use crate::error::{StreamsError, StreamsResult};

/// What kind of side a task sits on. Source produces records
/// into a Kafka topic; Sink consumes records from a Kafka topic
/// into an external system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
    Source,
    Sink,
}

/// Per-task lifecycle status — visible to the Worker for
/// reporting and for the Herder's rebalance decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Task is created but the polling loop hasn't run yet.
    Pending,
    /// Polling loop is active.
    Running,
    /// Stopped via `Worker::stop_task`; clean shutdown.
    Stopped,
    /// `poll()` or `put()` returned `Err`; the task is dead.
    Failed,
}

/// One record produced by a `SourceTask::poll()`. The
/// `source_partition` / `source_offset` pair lets the offset
/// store resume from where this task left off after restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecord {
    /// Logical source identifier (e.g. table name for a JDBC
    /// connector) — opaque, used only as offset-store key.
    pub source_partition: BTreeMap<String, String>,
    /// Where within `source_partition` this record sits —
    /// e.g. `{"offset": "1234"}`.
    pub source_offset: BTreeMap<String, String>,
    /// Destination Kafka topic.
    pub topic: String,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

/// One record consumed from Kafka by a `SinkTask`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkRecord {
    pub topic: String,
    pub partition: i32,
    pub kafka_offset: u64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

/// Trait every source connector implements. `poll()` is called
/// in a loop; returning an empty vec is a back-pressure signal
/// that the runtime should sleep before retrying.
pub trait SourceTask: Send + Sync + 'static {
    /// Called once at task start. Default impl no-op.
    fn start(&mut self) -> StreamsResult<()> {
        Ok(())
    }
    /// Pull a batch of records from the external source.
    fn poll(&mut self) -> StreamsResult<Vec<SourceRecord>>;
    /// Called once at task shutdown. Default impl no-op.
    fn stop(&mut self) -> StreamsResult<()> {
        Ok(())
    }
}

/// Trait every sink connector implements. `put()` receives a
/// batch of records; the runtime takes care of offset commit.
pub trait SinkTask: Send + Sync + 'static {
    fn start(&mut self) -> StreamsResult<()> {
        Ok(())
    }
    /// Apply a batch of records to the external system.
    fn put(&mut self, records: Vec<SinkRecord>) -> StreamsResult<()>;
    /// Called when the runtime commits offsets. Connectors that
    /// need to flush at the same moment override this.
    fn flush(&mut self) -> StreamsResult<()> {
        Ok(())
    }
    fn stop(&mut self) -> StreamsResult<()> {
        Ok(())
    }
}

/// A live handle to a task running inside the Worker.
pub struct TaskHandle {
    /// Stable id assigned by the Worker — `<connector>:<n>`.
    pub id: String,
    pub kind: TaskKind,
    pub status: TaskStatus,
    /// Last error captured if `status == Failed`.
    pub last_error: Option<String>,
    /// Monotonic counter for diagnostic output: how many
    /// `poll()` (Source) or `put()` (Sink) invocations have
    /// returned `Ok` so far.
    pub successful_iterations: u64,
    /// Total records emitted (Source) or consumed (Sink).
    pub record_count: u64,
}

impl TaskHandle {
    pub fn new(id: impl Into<String>, kind: TaskKind) -> Self {
        Self {
            id: id.into(),
            kind,
            status: TaskStatus::Pending,
            last_error: None,
            successful_iterations: 0,
            record_count: 0,
        }
    }
}

/// One iteration of a task. Hides the source/sink distinction
/// so the Worker loop can be generic.
pub trait TaskRuntime: Send + Sync {
    /// Tick the runtime once. Returns `Ok(records_processed)`
    /// or `Err(StreamsError)`. Empty vec from a source signals
    /// idle.
    fn tick(&mut self) -> StreamsResult<u64>;
    fn kind(&self) -> TaskKind;
}

/// `TaskRuntime` adapter for `SourceTask` — its `tick()` calls
/// `poll()` and returns the record count.
pub struct SourceRuntime {
    pub task: Box<dyn SourceTask>,
    pub last_offsets: BTreeMap<BTreeMap<String, String>, BTreeMap<String, String>>,
    pub emitted: u64,
}

impl SourceRuntime {
    pub fn new(task: Box<dyn SourceTask>) -> Self {
        Self {
            task,
            last_offsets: BTreeMap::new(),
            emitted: 0,
        }
    }
}

impl TaskRuntime for SourceRuntime {
    fn tick(&mut self) -> StreamsResult<u64> {
        let records = self.task.poll()?;
        let n = records.len() as u64;
        for r in records {
            self.last_offsets.insert(r.source_partition, r.source_offset);
        }
        self.emitted += n;
        Ok(n)
    }
    fn kind(&self) -> TaskKind {
        TaskKind::Source
    }
}

/// Adapter for `SinkTask` — the Worker feeds it a batch of records
/// drawn from a Kafka consumer (in tests a `SinkRuntime` is
/// fed records manually).
pub struct SinkRuntime {
    pub task: Box<dyn SinkTask>,
    pub queued: std::collections::VecDeque<SinkRecord>,
    pub consumed: u64,
}

impl SinkRuntime {
    pub fn new(task: Box<dyn SinkTask>) -> Self {
        Self {
            task,
            queued: std::collections::VecDeque::new(),
            consumed: 0,
        }
    }
    pub fn enqueue(&mut self, r: SinkRecord) {
        self.queued.push_back(r);
    }
    pub fn enqueue_batch(&mut self, rs: Vec<SinkRecord>) {
        for r in rs {
            self.queued.push_back(r);
        }
    }
}

impl TaskRuntime for SinkRuntime {
    fn tick(&mut self) -> StreamsResult<u64> {
        if self.queued.is_empty() {
            return Ok(0);
        }
        let batch: Vec<SinkRecord> = self.queued.drain(..).collect();
        let n = batch.len() as u64;
        self.task.put(batch)?;
        self.consumed += n;
        Ok(n)
    }
    fn kind(&self) -> TaskKind {
        TaskKind::Sink
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test source that yields a fixed sequence then idles.
    struct CannedSource {
        remaining: Vec<SourceRecord>,
        polls: u32,
    }
    impl SourceTask for CannedSource {
        fn poll(&mut self) -> StreamsResult<Vec<SourceRecord>> {
            self.polls += 1;
            Ok(std::mem::take(&mut self.remaining))
        }
    }

    /// Test source that always errors.
    struct ErroringSource;
    impl SourceTask for ErroringSource {
        fn poll(&mut self) -> StreamsResult<Vec<SourceRecord>> {
            Err(StreamsError::Internal("source boom".into()))
        }
    }

    /// Test sink that records every batch.
    struct RecordingSink {
        captured: Vec<SinkRecord>,
        flushes: u32,
    }
    impl SinkTask for RecordingSink {
        fn put(&mut self, records: Vec<SinkRecord>) -> StreamsResult<()> {
            self.captured.extend(records);
            Ok(())
        }
        fn flush(&mut self) -> StreamsResult<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    fn rec(topic: &str, off: &str) -> SourceRecord {
        let mut sp = BTreeMap::new();
        sp.insert("source".into(), topic.into());
        let mut so = BTreeMap::new();
        so.insert("offset".into(), off.into());
        SourceRecord {
            source_partition: sp,
            source_offset: so,
            topic: topic.into(),
            key: vec![],
            value: vec![1, 2, 3],
        }
    }

    #[test]
    fn task_handle_new_starts_pending() {
        let h = TaskHandle::new("c:0", TaskKind::Source);
        assert_eq!(h.status, TaskStatus::Pending);
        assert_eq!(h.successful_iterations, 0);
    }

    #[test]
    fn source_runtime_tick_returns_record_count() {
        let task = Box::new(CannedSource {
            remaining: vec![rec("orders", "1"), rec("orders", "2")],
            polls: 0,
        });
        let mut rt = SourceRuntime::new(task);
        let n = rt.tick().unwrap();
        assert_eq!(n, 2);
        assert_eq!(rt.emitted, 2);
    }

    #[test]
    fn source_runtime_records_last_offset_per_partition() {
        let task = Box::new(CannedSource {
            remaining: vec![rec("orders", "1"), rec("orders", "5"), rec("payments", "9")],
            polls: 0,
        });
        let mut rt = SourceRuntime::new(task);
        rt.tick().unwrap();
        assert_eq!(rt.last_offsets.len(), 2);
        // Partition "orders" should keep the LAST offset.
        let orders = rt
            .last_offsets
            .iter()
            .find(|(k, _)| k.get("source") == Some(&"orders".to_string()))
            .unwrap();
        assert_eq!(orders.1.get("offset"), Some(&"5".to_string()));
    }

    #[test]
    fn source_runtime_propagates_error() {
        let mut rt = SourceRuntime::new(Box::new(ErroringSource));
        let err = rt.tick();
        assert!(err.is_err());
        assert_eq!(rt.emitted, 0);
    }

    #[test]
    fn source_runtime_idle_returns_zero() {
        let task = Box::new(CannedSource {
            remaining: vec![],
            polls: 0,
        });
        let mut rt = SourceRuntime::new(task);
        let n = rt.tick().unwrap();
        assert_eq!(n, 0);
        assert_eq!(rt.emitted, 0);
    }

    #[test]
    fn sink_runtime_enqueue_and_tick_drains_queue() {
        let task = Box::new(RecordingSink {
            captured: vec![],
            flushes: 0,
        });
        let mut rt = SinkRuntime::new(task);
        for i in 0..3 {
            rt.enqueue(SinkRecord {
                topic: "orders".into(),
                partition: 0,
                kafka_offset: i,
                key: vec![],
                value: vec![i as u8],
            });
        }
        let n = rt.tick().unwrap();
        assert_eq!(n, 3);
        assert!(rt.queued.is_empty());
        assert_eq!(rt.consumed, 3);
    }

    #[test]
    fn sink_runtime_tick_with_empty_queue_returns_zero() {
        let mut rt = SinkRuntime::new(Box::new(RecordingSink {
            captured: vec![],
            flushes: 0,
        }));
        let n = rt.tick().unwrap();
        assert_eq!(n, 0);
        assert_eq!(rt.consumed, 0);
    }

    #[test]
    fn task_runtime_kind_reports_correctly() {
        let src = SourceRuntime::new(Box::new(CannedSource {
            remaining: vec![],
            polls: 0,
        }));
        assert_eq!(src.kind(), TaskKind::Source);
        let snk = SinkRuntime::new(Box::new(RecordingSink {
            captured: vec![],
            flushes: 0,
        }));
        assert_eq!(snk.kind(), TaskKind::Sink);
    }
}
