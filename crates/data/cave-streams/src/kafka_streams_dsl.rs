// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka Streams — high-level DSL (`org.apache.kafka.streams.kstream`).
//!
//! upstream: apache/kafka — streams/src/main/java/org/apache/kafka/streams/kstream
//! (StreamsBuilder / KStream / KTable / KGroupedStream / Materialized /
//! TimeWindows)
//!
//! This is the fluent, code-defined DSL that authors stream-processing
//! topologies — the counterpart to the low-level Processor API in
//! [`crate::kafka_streams_processor`].  Like upstream, a `StreamsBuilder`
//! accumulates an operator graph; `build()` freezes it into an executable
//! application.  Execution mirrors `TopologyTestDriver`: [`StreamsApp::pipe_input`]
//! pushes one record through the graph, [`StreamsApp::drain_output`] collects
//! what reached a sink, and [`StreamsApp::store_get`] queries a materialized
//! state store.
//!
//! The DSL is closure-based to match the Java `Predicate`/`ValueMapper`/
//! `KeyValueMapper`/`Aggregator` functional interfaces.  Each operator appends
//! a node whose parent is the upstream node; records flow depth-first through
//! children, and an operator may emit zero, one, or many downstream records —
//! exactly the upstream semantics (a `filter` drops, a `flatMap` fans out).

pub use crate::kafka_streams_processor::Record;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::rc::Rc;

// ─── Operator graph ──────────────────────────────────────────────────────────

/// Boxed predicate — the DSL counterpart of `org.apache.kafka.streams.kstream.Predicate`.
/// Exposed for `KStream::branch`, which takes a heterogeneous list of them.
pub type DslPredicate = Box<dyn Fn(&Record) -> bool>;

type Predicate = Box<dyn Fn(&Record) -> bool>;
type ValueMapper = Box<dyn Fn(&[u8]) -> Vec<u8>>;
type RecordMapper = Box<dyn Fn(&Record) -> Record>;
type ValueFlatMapper = Box<dyn Fn(&[u8]) -> Vec<Vec<u8>>>;
type RecordFlatMapper = Box<dyn Fn(&Record) -> Vec<Record>>;
type KeySelector = Box<dyn Fn(&Record) -> Vec<u8>>;
type Peeker = Box<dyn Fn(&Record)>;
type Initializer = Box<dyn Fn() -> Vec<u8>>;
/// `(key, value, current_aggregate) -> new_aggregate`
type Aggregator = Box<dyn Fn(&[u8], &[u8], &[u8]) -> Vec<u8>>;
/// `(current_aggregate, new_value) -> new_aggregate`
type Reducer = Box<dyn Fn(&[u8], &[u8]) -> Vec<u8>>;

enum NodeKind {
    Source { topic: String },
    Filter(Predicate),
    MapValues(ValueMapper),
    Map(RecordMapper),
    FlatMapValues(ValueFlatMapper),
    FlatMap(RecordFlatMapper),
    SelectKey(KeySelector),
    Peek(Peeker),
    /// Pass-through node with no transform — a join point for `merge` and a
    /// landing node for terminal operators like `foreach`.
    Passthrough,
    /// Route a record to the first child whose predicate matches (the
    /// predicate at index `i` guards child `i`); drop if none match.
    Branch(Vec<Predicate>),
    /// Persist the record to an intermediate topic, then continue downstream.
    Through { topic: String },
    /// Stateful aggregation: `store[key] = agg(key, value, store.get(key)
    /// .unwrap_or_else(init))`.  Emits the new aggregate as a changelog record.
    Aggregate {
        store: String,
        init: Initializer,
        agg: Aggregator,
    },
    /// Stateful reduce: the first value per key initializes the store; later
    /// values fold via `reducer(current, value)`.  Emits a changelog record.
    Reduce { store: String, reducer: Reducer },
    /// Windowed aggregation.  A record at timestamp `t` is assigned to every
    /// window `[start, start+size)` containing `t` (one for tumbling, several
    /// for overlapping hopping windows).  Per `(key, window_start)` the store
    /// holds `agg(key, value, store.get_or(init))`; each window emits a
    /// changelog record.
    WindowedAggregate {
        store: String,
        size_ms: i64,
        advance_ms: i64,
        init: Initializer,
        agg: Aggregator,
    },
    Sink { topic: String },
}

/// Composite state-store key for a windowed aggregate: `key` bytes, a NUL
/// separator, then the window start as big-endian `i64`.
fn window_store_key(key: &[u8], window_start: i64) -> Vec<u8> {
    let mut k = Vec::with_capacity(key.len() + 9);
    k.extend_from_slice(key);
    k.push(0);
    k.extend_from_slice(&window_start.to_be_bytes());
    k
}

/// Window starts whose `[start, start+size)` interval contains `ts`, ascending.
/// A start is valid iff `start <= ts < start + size` and `start` is a multiple
/// of `advance`.
fn windows_for(ts: i64, size_ms: i64, advance_ms: i64) -> Vec<i64> {
    let mut starts = Vec::new();
    if size_ms <= 0 || advance_ms <= 0 || ts < 0 {
        return starts;
    }
    let mut start = (ts / advance_ms) * advance_ms; // largest multiple <= ts
    while start + size_ms > ts && start >= 0 {
        starts.push(start);
        start -= advance_ms;
    }
    starts.reverse(); // ascending window-start order
    starts
}

struct Node {
    kind: NodeKind,
    children: Vec<usize>,
}

#[derive(Default)]
struct BuildState {
    nodes: Vec<Node>,
    /// source topic → node index
    sources: HashMap<String, usize>,
}

impl BuildState {
    fn push(&mut self, kind: NodeKind, parent: Option<usize>) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(Node {
            kind,
            children: Vec::new(),
        });
        if let Some(p) = parent {
            self.nodes[p].children.push(idx);
        }
        idx
    }
}

// ─── Builder + handles ───────────────────────────────────────────────────────

/// Authors a stream-processing topology.  Mirrors
/// `org.apache.kafka.streams.StreamsBuilder`.
pub struct StreamsBuilder {
    inner: Rc<RefCell<BuildState>>,
}

impl Default for StreamsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamsBuilder {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(BuildState::default())),
        }
    }

    /// Open a [`KStream`] reading from `topic` (`StreamsBuilder.stream`).
    pub fn stream(&self, topic: &str) -> KStream {
        let node = {
            let mut st = self.inner.borrow_mut();
            let idx = st.push(
                NodeKind::Source {
                    topic: topic.to_string(),
                },
                None,
            );
            st.sources.insert(topic.to_string(), idx);
            idx
        };
        KStream {
            inner: self.inner.clone(),
            node,
        }
    }

    /// Freeze the topology into an executable application.
    pub fn build(self) -> StreamsApp {
        let st = std::mem::take(&mut *self.inner.borrow_mut());
        StreamsApp {
            nodes: st.nodes,
            sources: st.sources,
            output: HashMap::new(),
            stores: HashMap::new(),
        }
    }
}

/// A record stream.  Mirrors `org.apache.kafka.streams.kstream.KStream`.
pub struct KStream {
    inner: Rc<RefCell<BuildState>>,
    node: usize,
}

impl KStream {
    fn chain(&self, kind: NodeKind) -> KStream {
        let node = self.inner.borrow_mut().push(kind, Some(self.node));
        KStream {
            inner: self.inner.clone(),
            node,
        }
    }

    /// Keep only records for which `pred` is true (`KStream.filter`).
    pub fn filter<F: Fn(&Record) -> bool + 'static>(&self, pred: F) -> KStream {
        self.chain(NodeKind::Filter(Box::new(pred)))
    }

    /// Drop records for which `pred` is true (`KStream.filterNot`).
    pub fn filter_not<F: Fn(&Record) -> bool + 'static>(&self, pred: F) -> KStream {
        self.chain(NodeKind::Filter(Box::new(move |r| !pred(r))))
    }

    /// Transform the value, preserving key/timestamp (`KStream.mapValues`).
    pub fn map_values<F: Fn(&[u8]) -> Vec<u8> + 'static>(&self, f: F) -> KStream {
        self.chain(NodeKind::MapValues(Box::new(f)))
    }

    /// Transform key and/or value (`KStream.map`).
    pub fn map<F: Fn(&Record) -> Record + 'static>(&self, f: F) -> KStream {
        self.chain(NodeKind::Map(Box::new(f)))
    }

    /// Fan one value into many, preserving key (`KStream.flatMapValues`).
    pub fn flat_map_values<F: Fn(&[u8]) -> Vec<Vec<u8>> + 'static>(&self, f: F) -> KStream {
        self.chain(NodeKind::FlatMapValues(Box::new(f)))
    }

    /// Fan one record into many records (`KStream.flatMap`).
    pub fn flat_map<F: Fn(&Record) -> Vec<Record> + 'static>(&self, f: F) -> KStream {
        self.chain(NodeKind::FlatMap(Box::new(f)))
    }

    /// Re-key without touching the value (`KStream.selectKey`).
    pub fn select_key<F: Fn(&Record) -> Vec<u8> + 'static>(&self, f: F) -> KStream {
        self.chain(NodeKind::SelectKey(Box::new(f)))
    }

    /// Observe each record as a side effect, passing it through (`KStream.peek`).
    pub fn peek<F: Fn(&Record) + 'static>(&self, f: F) -> KStream {
        self.chain(NodeKind::Peek(Box::new(f)))
    }

    /// Terminal: run a side effect per record, forwarding nothing
    /// (`KStream.foreach`).
    pub fn foreach<F: Fn(&Record) + 'static>(&self, f: F) {
        // A peek whose child is a dead-end passthrough — terminal, no sink.
        self.chain(NodeKind::Peek(Box::new(f)));
    }

    /// Terminal: write every record to `topic` (`KStream.to`).
    pub fn to(&self, topic: &str) {
        self.chain(NodeKind::Sink {
            topic: topic.to_string(),
        });
    }

    /// Split into N branches — a record goes to the first branch whose
    /// predicate matches (mutually exclusive) and is dropped if none do.
    /// Returns one [`KStream`] per predicate, in order (`KStream.branch`).
    pub fn branch(&self, predicates: Vec<DslPredicate>) -> Vec<KStream> {
        let n = predicates.len();
        let mut st = self.inner.borrow_mut();
        let branch_node = st.push(NodeKind::Branch(predicates), Some(self.node));
        // One passthrough head per predicate, as ordered children of the
        // branch node — child index i is guarded by predicate i.
        let mut heads = Vec::with_capacity(n);
        for _ in 0..n {
            let head = st.push(NodeKind::Passthrough, Some(branch_node));
            heads.push(KStream {
                inner: self.inner.clone(),
                node: head,
            });
        }
        heads
    }

    /// Merge this stream with `other` — downstream sees records from both
    /// (`KStream.merge`).  Both streams must come from the same builder.
    pub fn merge(&self, other: &KStream) -> KStream {
        let node = {
            let mut st = self.inner.borrow_mut();
            let node = st.push(NodeKind::Passthrough, Some(self.node));
            // Also a child of the other parent.
            st.nodes[other.node].children.push(node);
            node
        };
        KStream {
            inner: self.inner.clone(),
            node,
        }
    }

    /// Write each record to an intermediate `topic`, then continue the stream
    /// (`KStream.through`).
    pub fn through(&self, topic: &str) -> KStream {
        self.chain(NodeKind::Through {
            topic: topic.to_string(),
        })
    }

    /// Group by the existing key (`KStream.groupByKey`).
    pub fn group_by_key(&self) -> KGroupedStream {
        KGroupedStream {
            inner: self.inner.clone(),
            node: self.node,
        }
    }

    /// Re-key with `f`, then group by the new key (`KStream.groupBy`).
    pub fn group_by<F: Fn(&Record) -> Vec<u8> + 'static>(&self, f: F) -> KGroupedStream {
        let s = self.select_key(f);
        KGroupedStream {
            inner: s.inner,
            node: s.node,
        }
    }
}

/// A stream grouped by key, awaiting an aggregation.  Mirrors
/// `org.apache.kafka.streams.kstream.KGroupedStream`.
pub struct KGroupedStream {
    inner: Rc<RefCell<BuildState>>,
    node: usize,
}

impl KGroupedStream {
    fn chain(&self, kind: NodeKind) -> KTable {
        let node = self.inner.borrow_mut().push(kind, Some(self.node));
        KTable {
            inner: self.inner.clone(),
            node,
        }
    }

    /// Count records per key into `store`; the running tally is a
    /// decimal-encoded `Long` (`KGroupedStream.count`).
    pub fn count(&self, store: &str) -> KTable {
        self.chain(NodeKind::Aggregate {
            store: store.to_string(),
            init: Box::new(|| b"0".to_vec()),
            agg: Box::new(|_k, _v, current| {
                let cur: i64 = String::from_utf8_lossy(current).parse().unwrap_or(0);
                (cur + 1).to_string().into_bytes()
            }),
        })
    }

    /// Reduce values per key into `store` (`KGroupedStream.reduce`).
    pub fn reduce<F: Fn(&[u8], &[u8]) -> Vec<u8> + 'static>(&self, store: &str, reducer: F) -> KTable {
        self.chain(NodeKind::Reduce {
            store: store.to_string(),
            reducer: Box::new(reducer),
        })
    }

    /// Aggregate values per key into `store` with an initializer + aggregator
    /// (`KGroupedStream.aggregate`).
    pub fn aggregate<I, A>(&self, store: &str, init: I, agg: A) -> KTable
    where
        I: Fn() -> Vec<u8> + 'static,
        A: Fn(&[u8], &[u8], &[u8]) -> Vec<u8> + 'static,
    {
        self.chain(NodeKind::Aggregate {
            store: store.to_string(),
            init: Box::new(init),
            agg: Box::new(agg),
        })
    }

    /// Window the grouped stream by event time (`KGroupedStream.windowedBy`).
    pub fn windowed_by(&self, windows: TimeWindows) -> TimeWindowedKStream {
        TimeWindowedKStream {
            inner: self.inner.clone(),
            node: self.node,
            windows,
        }
    }
}

/// A fixed-size, fixed-advance time window definition.  Mirrors
/// `org.apache.kafka.streams.kstream.TimeWindows`.  `advance == size` gives
/// tumbling windows; `advance < size` gives overlapping hopping windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindows {
    size_ms: i64,
    advance_ms: i64,
}

impl TimeWindows {
    /// A tumbling window of `size_ms` (`TimeWindows.ofSizeWithNoGrace`).
    pub fn of_size_ms(size_ms: i64) -> Self {
        Self {
            size_ms,
            advance_ms: size_ms,
        }
    }

    /// Make the window hop by `advance_ms` (`TimeWindows.advanceBy`).
    pub fn advance_by_ms(mut self, advance_ms: i64) -> Self {
        self.advance_ms = advance_ms;
        self
    }

    pub fn size_ms(&self) -> i64 {
        self.size_ms
    }

    pub fn advance_ms(&self) -> i64 {
        self.advance_ms
    }
}

/// A grouped stream windowed by time, awaiting a windowed aggregation.
/// Mirrors `org.apache.kafka.streams.kstream.TimeWindowedKStream`.
pub struct TimeWindowedKStream {
    inner: Rc<RefCell<BuildState>>,
    node: usize,
    windows: TimeWindows,
}

impl TimeWindowedKStream {
    fn chain(&self, kind: NodeKind) -> KTable {
        let node = self.inner.borrow_mut().push(kind, Some(self.node));
        KTable {
            inner: self.inner.clone(),
            node,
        }
    }

    /// Count records per `(key, window)` (`TimeWindowedKStream.count`).
    pub fn count(&self, store: &str) -> KTable {
        self.chain(NodeKind::WindowedAggregate {
            store: store.to_string(),
            size_ms: self.windows.size_ms,
            advance_ms: self.windows.advance_ms,
            init: Box::new(|| b"0".to_vec()),
            agg: Box::new(|_k, _v, current| {
                let cur: i64 = String::from_utf8_lossy(current).parse().unwrap_or(0);
                (cur + 1).to_string().into_bytes()
            }),
        })
    }

    /// Aggregate per `(key, window)` (`TimeWindowedKStream.aggregate`).
    pub fn aggregate<I, A>(&self, store: &str, init: I, agg: A) -> KTable
    where
        I: Fn() -> Vec<u8> + 'static,
        A: Fn(&[u8], &[u8], &[u8]) -> Vec<u8> + 'static,
    {
        self.chain(NodeKind::WindowedAggregate {
            store: store.to_string(),
            size_ms: self.windows.size_ms,
            advance_ms: self.windows.advance_ms,
            init: Box::new(init),
            agg: Box::new(agg),
        })
    }
}

/// A changelog-backed table.  Mirrors `org.apache.kafka.streams.kstream.KTable`.
/// Internally a KTable's nodes carry changelog records (one per update), so
/// table operators reuse the stream node kinds.
pub struct KTable {
    inner: Rc<RefCell<BuildState>>,
    node: usize,
}

impl KTable {
    fn chain(&self, kind: NodeKind) -> KTable {
        let node = self.inner.borrow_mut().push(kind, Some(self.node));
        KTable {
            inner: self.inner.clone(),
            node,
        }
    }

    /// View the changelog as a record stream (`KTable.toStream`).
    pub fn to_stream(&self) -> KStream {
        KStream {
            inner: self.inner.clone(),
            node: self.node,
        }
    }

    /// Filter the changelog (`KTable.filter`).
    pub fn filter<F: Fn(&Record) -> bool + 'static>(&self, pred: F) -> KTable {
        self.chain(NodeKind::Filter(Box::new(pred)))
    }

    /// Transform changelog values (`KTable.mapValues`).
    pub fn map_values<F: Fn(&[u8]) -> Vec<u8> + 'static>(&self, f: F) -> KTable {
        self.chain(NodeKind::MapValues(Box::new(f)))
    }

    /// Terminal: write the changelog to `topic` (`KTable.toStream().to`).
    pub fn to(&self, topic: &str) {
        self.chain(NodeKind::Sink {
            topic: topic.to_string(),
        });
    }
}

// ─── Executable application ──────────────────────────────────────────────────

/// A frozen, executable topology.  Counterpart to `TopologyTestDriver`.
pub struct StreamsApp {
    nodes: Vec<Node>,
    sources: HashMap<String, usize>,
    output: HashMap<String, Vec<Record>>,
    /// Materialized state stores (changelog-backed key→value).
    stores: HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>,
}

/// What a single node does to one inbound record, computed under an immutable
/// borrow of the node and applied once the borrow is released.
enum Step {
    /// Forward these records to every child.
    Forward(Vec<Record>),
    /// Forward one record to a single child by local index (branch routing);
    /// `None` drops it.
    ForwardChild(Option<usize>, Record),
    /// Append to the named output topic (sink); forwards nothing.
    Sink { topic: String, rec: Record },
    /// Append to an intermediate topic, then forward to every child.
    Through { topic: String, rec: Record },
    /// Write the new aggregate into `store` under `key`, then forward the
    /// changelog record (`key`, new aggregate) to every child.
    StoreUpdate {
        store: String,
        key: Vec<u8>,
        changelog: Record,
    },
    /// One store write + changelog forward per matched window.
    WindowUpdates {
        store: String,
        updates: Vec<(Vec<u8>, Record)>,
    },
}

impl StreamsApp {
    /// Push one record into `topic` and run it through the topology.
    pub fn pipe_input(&mut self, topic: &str, key: &[u8], value: &[u8], ts: i64) {
        let Some(&src) = self.sources.get(topic) else {
            return;
        };
        self.run(src, Record::new(key, value, ts));
    }

    /// Process `rec` at node `idx`, then forward to children depth-first in
    /// emission order — matching upstream's `ProcessorContext.forward`.
    fn run(&mut self, idx: usize, rec: Record) {
        // Compute the step + child list under a scoped borrow, then drop it
        // before recursing (recursion needs `&mut self`).
        let (step, children) = {
            let node = &self.nodes[idx];
            let children = node.children.clone();
            let step = match &node.kind {
                NodeKind::Source { .. } | NodeKind::Passthrough => Step::Forward(vec![rec]),
                NodeKind::Filter(p) => {
                    if p(&rec) {
                        Step::Forward(vec![rec])
                    } else {
                        Step::Forward(vec![])
                    }
                }
                NodeKind::MapValues(f) => {
                    let mut r = rec;
                    r.value = f(&r.value);
                    Step::Forward(vec![r])
                }
                NodeKind::Map(f) => Step::Forward(vec![f(&rec)]),
                NodeKind::FlatMapValues(f) => {
                    let out = f(&rec.value)
                        .into_iter()
                        .map(|v| {
                            let mut r = rec.clone();
                            r.value = v;
                            r
                        })
                        .collect();
                    Step::Forward(out)
                }
                NodeKind::FlatMap(f) => Step::Forward(f(&rec)),
                NodeKind::SelectKey(f) => {
                    let mut r = rec;
                    r.key = f(&r);
                    Step::Forward(vec![r])
                }
                NodeKind::Peek(f) => {
                    f(&rec);
                    Step::Forward(vec![rec])
                }
                NodeKind::Branch(preds) => {
                    let chosen = preds.iter().position(|p| p(&rec));
                    Step::ForwardChild(chosen, rec)
                }
                NodeKind::Through { topic } => Step::Through {
                    topic: topic.clone(),
                    rec,
                },
                NodeKind::Aggregate { store, init, agg } => {
                    let cur = self
                        .stores
                        .get(store)
                        .and_then(|s| s.get(&rec.key))
                        .cloned()
                        .unwrap_or_else(|| init());
                    let newv = agg(&rec.key, &rec.value, &cur);
                    let mut changelog = rec;
                    changelog.value = newv;
                    Step::StoreUpdate {
                        store: store.clone(),
                        key: changelog.key.clone(),
                        changelog,
                    }
                }
                NodeKind::Reduce { store, reducer } => {
                    let newv = match self.stores.get(store).and_then(|s| s.get(&rec.key)) {
                        Some(cur) => reducer(cur, &rec.value),
                        None => rec.value.clone(), // first value initializes
                    };
                    let mut changelog = rec;
                    changelog.value = newv;
                    Step::StoreUpdate {
                        store: store.clone(),
                        key: changelog.key.clone(),
                        changelog,
                    }
                }
                NodeKind::WindowedAggregate {
                    store,
                    size_ms,
                    advance_ms,
                    init,
                    agg,
                } => {
                    let mut updates = Vec::new();
                    for start in windows_for(rec.timestamp_ms, *size_ms, *advance_ms) {
                        let composite = window_store_key(&rec.key, start);
                        let cur = self
                            .stores
                            .get(store)
                            .and_then(|s| s.get(&composite))
                            .cloned()
                            .unwrap_or_else(|| init());
                        let newv = agg(&rec.key, &rec.value, &cur);
                        // Changelog record is keyed by the windowed key.
                        let changelog = Record::new(&composite, &newv, rec.timestamp_ms);
                        updates.push((composite, changelog));
                    }
                    Step::WindowUpdates {
                        store: store.clone(),
                        updates,
                    }
                }
                NodeKind::Sink { topic } => Step::Sink {
                    topic: topic.clone(),
                    rec,
                },
            };
            (step, children)
        };

        match step {
            Step::Forward(recs) => {
                for r in recs {
                    for &c in &children {
                        self.run(c, r.clone());
                    }
                }
            }
            Step::ForwardChild(Some(i), rec) => {
                if let Some(&c) = children.get(i) {
                    self.run(c, rec);
                }
            }
            Step::ForwardChild(None, _rec) => {}
            Step::Through { topic, rec } => {
                self.output.entry(topic).or_default().push(rec.clone());
                for &c in &children {
                    self.run(c, rec.clone());
                }
            }
            Step::StoreUpdate {
                store,
                key,
                changelog,
            } => {
                self.stores
                    .entry(store)
                    .or_default()
                    .insert(key, changelog.value.clone());
                for &c in &children {
                    self.run(c, changelog.clone());
                }
            }
            Step::WindowUpdates { store, updates } => {
                for (composite, changelog) in updates {
                    self.stores
                        .entry(store.clone())
                        .or_default()
                        .insert(composite, changelog.value.clone());
                    for &c in &children {
                        self.run(c, changelog.clone());
                    }
                }
            }
            Step::Sink { topic, rec } => {
                self.output.entry(topic).or_default().push(rec);
            }
        }
    }

    /// Remove and return everything written to `topic` so far.
    pub fn drain_output(&mut self, topic: &str) -> Vec<Record> {
        self.output.remove(topic).unwrap_or_default()
    }

    /// Query a materialized state store by key.
    pub fn store_get(&self, store: &str, key: &[u8]) -> Option<Vec<u8>> {
        self.stores.get(store).and_then(|s| s.get(key).cloned())
    }

    /// Query a windowed state store by `(key, window_start)`.
    pub fn windowed_store_get(
        &self,
        store: &str,
        key: &[u8],
        window_start_ms: i64,
    ) -> Option<Vec<u8>> {
        let composite = window_store_key(key, window_start_ms);
        self.stores.get(store).and_then(|s| s.get(&composite).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_registers_source() {
        let b = StreamsBuilder::new();
        b.stream("orders").to("sink");
        let app = b.build();
        assert!(app.sources.contains_key("orders"));
    }

    #[test]
    fn empty_store_get_is_none() {
        let b = StreamsBuilder::new();
        b.stream("in").to("out");
        let app = b.build();
        assert_eq!(app.store_get("missing", b"k"), None);
    }
}
