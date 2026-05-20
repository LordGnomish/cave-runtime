// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka Streams — low-level Processor API.
//!
//! upstream: apache/kafka — streams/src/main/java/org/apache/kafka/streams/processor
//! (Processor / ProcessorContext / StateStore / Punctuator)
//!
//! The high-level Streams DSL is already covered by `streams_api.rs`.
//! This module ports the **low-level Processor API**: a Topology of
//! sources → processors → sinks, named state stores, and wall-clock /
//! stream-time punctuation callbacks.
//!
//! Threading is single-threaded per partition — that matches Kafka
//! Streams' guarantee that all callbacks on a given processor observe
//! a consistent state-store view. We expose it as an in-memory engine
//! the broker can drive directly.

use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub timestamp_ms: i64,
    pub headers: HashMap<String, Vec<u8>>,
}

impl Record {
    pub fn new(key: &[u8], value: &[u8], ts: i64) -> Self {
        Self {
            key: key.to_vec(),
            value: value.to_vec(),
            timestamp_ms: ts,
            headers: HashMap::new(),
        }
    }
}

/// One row in a named state store. We keep stores as key→value maps.
#[derive(Default, Debug, Clone)]
pub struct StateStore {
    pub name: String,
    map: HashMap<Vec<u8>, Vec<u8>>,
}

impl StateStore {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            map: HashMap::new(),
        }
    }
    pub fn put(&mut self, k: &[u8], v: &[u8]) {
        self.map.insert(k.to_vec(), v.to_vec());
    }
    pub fn get(&self, k: &[u8]) -> Option<&[u8]> {
        self.map.get(k).map(Vec::as_slice)
    }
    pub fn delete(&mut self, k: &[u8]) -> Option<Vec<u8>> {
        self.map.remove(k)
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn keys(&self) -> Vec<Vec<u8>> {
        self.map.keys().cloned().collect()
    }
}

/// Context the processor uses to forward records, schedule punctuators,
/// and access its connected state stores.
pub struct ProcessorContext {
    pub current_node: String,
    pub forwarded: Vec<(String, Record)>,
    pub commits: u32,
    pub scheduled_punctuators: Vec<ScheduledPunctuator>,
    pub stream_time_ms: i64,
}

impl ProcessorContext {
    pub fn new() -> Self {
        Self {
            current_node: String::new(),
            forwarded: Vec::new(),
            commits: 0,
            scheduled_punctuators: Vec::new(),
            stream_time_ms: 0,
        }
    }
    pub fn forward(&mut self, child: &str, record: Record) {
        self.forwarded.push((child.to_string(), record));
    }
    pub fn commit(&mut self) {
        self.commits += 1;
    }
    pub fn schedule(&mut self, p: ScheduledPunctuator) {
        self.scheduled_punctuators.push(p);
    }
}

impl Default for ProcessorContext {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PunctuationKind {
    /// Fires on observed event-time progress.
    StreamTime,
    /// Fires on wall-clock progress.
    WallClock,
}

#[derive(Debug, Clone)]
pub struct ScheduledPunctuator {
    pub kind: PunctuationKind,
    pub interval_ms: i64,
    pub last_fired_ms: i64,
    pub name: String,
}

impl ScheduledPunctuator {
    pub fn new(kind: PunctuationKind, interval_ms: i64, name: &str) -> Self {
        Self {
            kind,
            interval_ms,
            last_fired_ms: 0,
            name: name.to_string(),
        }
    }

    /// Returns true if the punctuator should fire at `now_ms` (wall-clock
    /// or stream-time) given its last firing.
    pub fn should_fire(&mut self, now_ms: i64) -> bool {
        if now_ms - self.last_fired_ms >= self.interval_ms {
            self.last_fired_ms = now_ms;
            true
        } else {
            false
        }
    }
}

/// Single processor node: `init` wires up state stores, `process`
/// handles one record, `close` releases resources.
pub trait Processor: Send {
    fn init(&mut self, _ctx: &mut ProcessorContext) {}
    fn process(&mut self, record: Record, ctx: &mut ProcessorContext, stores: &mut StoreRegistry);
    fn close(&mut self) {}
    fn name(&self) -> &str;
    fn connected_stores(&self) -> &[String] {
        &[]
    }
}

#[derive(Default)]
pub struct StoreRegistry {
    pub stores: HashMap<String, StateStore>,
}

impl StoreRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, s: StateStore) {
        self.stores.insert(s.name.clone(), s);
    }
    pub fn get_mut(&mut self, name: &str) -> Option<&mut StateStore> {
        self.stores.get_mut(name)
    }
}

#[derive(Default)]
pub struct Topology {
    /// node_name → (parent_names, processor)
    nodes: Vec<TopologyNode>,
    pub stores: StoreRegistry,
    pub source_topics: HashMap<String, String>, // topic → source node name
    pub sink_topics: HashMap<String, String>,   // node name → topic
}

struct TopologyNode {
    name: String,
    parents: Vec<String>,
    processor: Box<dyn Processor>,
}

impl Topology {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_source(&mut self, name: &str, topic: &str) {
        self.source_topics
            .insert(topic.to_string(), name.to_string());
        self.nodes.push(TopologyNode {
            name: name.to_string(),
            parents: Vec::new(),
            processor: Box::new(SourceNode {
                name: name.to_string(),
            }),
        });
    }

    pub fn add_processor<P: Processor + 'static>(&mut self, name: &str, parents: &[&str], p: P) {
        // Validate parents exist.
        let known: HashSet<String> = self.nodes.iter().map(|n| n.name.clone()).collect();
        for parent in parents {
            assert!(
                known.contains(*parent),
                "parent {} unknown in topology",
                parent
            );
        }
        self.nodes.push(TopologyNode {
            name: name.to_string(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            processor: Box::new(p),
        });
    }

    pub fn add_sink(&mut self, name: &str, parents: &[&str], topic: &str) {
        let parents_vec: Vec<String> = parents.iter().map(|s| s.to_string()).collect();
        self.sink_topics.insert(name.to_string(), topic.to_string());
        self.nodes.push(TopologyNode {
            name: name.to_string(),
            parents: parents_vec,
            processor: Box::new(SinkNode {
                name: name.to_string(),
            }),
        });
    }

    pub fn add_state_store(&mut self, name: &str) {
        self.stores.add(StateStore::new(name));
    }

    /// Push a record through the topology starting at `source_topic`.
    /// Returns the records that landed on sink nodes (each tagged with
    /// the sink topic).
    pub fn process(&mut self, source_topic: &str, record: Record) -> Vec<(String, Record)> {
        let mut sink_output: Vec<(String, Record)> = Vec::new();
        let source_node = match self.source_topics.get(source_topic).cloned() {
            Some(n) => n,
            None => return sink_output,
        };
        // BFS through children.
        let mut frontier: Vec<(String, Record)> = vec![(source_node, record)];
        while let Some((node_name, rec)) = frontier.pop() {
            // If sink, capture and continue.
            if let Some(topic) = self.sink_topics.get(&node_name).cloned() {
                sink_output.push((topic, rec));
                continue;
            }
            // Find direct children (whose parents include node_name).
            let children: Vec<usize> = self
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.parents.iter().any(|p| p == &node_name))
                .map(|(i, _)| i)
                .collect();
            // Allow processor to forward.
            let mut ctx = ProcessorContext::new();
            ctx.current_node = node_name.clone();
            ctx.stream_time_ms = rec.timestamp_ms;
            if let Some(node) = self.nodes.iter_mut().find(|n| n.name == node_name) {
                node.processor
                    .process(rec.clone(), &mut ctx, &mut self.stores);
            }
            // If processor didn't forward, propagate `rec` to every child unchanged.
            if ctx.forwarded.is_empty() {
                for ci in children {
                    let child_name = self.nodes[ci].name.clone();
                    frontier.push((child_name, rec.clone()));
                }
            } else {
                for (child, child_rec) in ctx.forwarded {
                    frontier.push((child, child_rec));
                }
            }
        }
        sink_output
    }
}

struct SourceNode {
    name: String,
}
impl Processor for SourceNode {
    fn process(&mut self, _r: Record, _ctx: &mut ProcessorContext, _s: &mut StoreRegistry) {}
    fn name(&self) -> &str {
        &self.name
    }
}

struct SinkNode {
    name: String,
}
impl Processor for SinkNode {
    fn process(&mut self, _r: Record, _ctx: &mut ProcessorContext, _s: &mut StoreRegistry) {}
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── StateStore ──────────────────────────────────────────────

    #[test]
    fn store_put_get_delete_roundtrip() {
        let mut s = StateStore::new("counts");
        s.put(b"a", b"1");
        assert_eq!(s.get(b"a"), Some(&[b'1'][..]));
        assert_eq!(s.delete(b"a"), Some(vec![b'1']));
        assert!(s.get(b"a").is_none());
    }

    #[test]
    fn store_len_reflects_inserts() {
        let mut s = StateStore::new("s");
        assert!(s.is_empty());
        s.put(b"k", b"v");
        assert_eq!(s.len(), 1);
    }

    // ─── ScheduledPunctuator ─────────────────────────────────────

    #[test]
    fn punctuator_fires_on_interval_elapsed() {
        let mut p = ScheduledPunctuator::new(PunctuationKind::WallClock, 1_000, "tick");
        assert!(p.should_fire(1_000)); // first time the interval has elapsed
        assert!(!p.should_fire(1_500));
        assert!(p.should_fire(2_500)); // 1000 ms after last firing at 1000
    }

    #[test]
    fn punctuator_kind_stream_time() {
        let p = ScheduledPunctuator::new(PunctuationKind::StreamTime, 100, "tick");
        assert_eq!(p.kind, PunctuationKind::StreamTime);
    }

    // ─── Topology — counting processor ───────────────────────────

    struct CountingProcessor {
        name_: String,
        store_name: String,
    }
    impl Processor for CountingProcessor {
        fn name(&self) -> &str {
            &self.name_
        }
        fn connected_stores(&self) -> &[String] {
            std::slice::from_ref(&self.store_name)
        }
        fn process(
            &mut self,
            record: Record,
            ctx: &mut ProcessorContext,
            stores: &mut StoreRegistry,
        ) {
            let store = stores.get_mut(&self.store_name).unwrap();
            let prev: u64 = store
                .get(&record.key)
                .map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0u8; 8])))
                .unwrap_or(0);
            let next = prev + 1;
            store.put(&record.key, &next.to_le_bytes());
            ctx.forward(
                "sink",
                Record::new(&record.key, &next.to_le_bytes(), record.timestamp_ms),
            );
        }
    }

    #[test]
    fn topology_routes_record_source_to_processor_to_sink() {
        let mut topo = Topology::new();
        topo.add_source("source", "input-topic");
        topo.add_state_store("counts");
        topo.add_processor(
            "counter",
            &["source"],
            CountingProcessor {
                name_: "counter".into(),
                store_name: "counts".into(),
            },
        );
        topo.add_sink("sink", &["counter"], "output-topic");

        let out = topo.process("input-topic", Record::new(b"alpha", b"x", 100));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "output-topic");
        // store now holds count=1 for "alpha"
        let s = topo.stores.stores.get("counts").unwrap();
        let raw = s.get(b"alpha").unwrap();
        let count = u64::from_le_bytes(raw.try_into().unwrap());
        assert_eq!(count, 1);
    }

    #[test]
    fn topology_unknown_source_topic_yields_no_output() {
        let mut topo = Topology::new();
        topo.add_source("source", "input");
        let out = topo.process("other-topic", Record::new(b"k", b"v", 0));
        assert!(out.is_empty());
    }

    #[test]
    fn topology_repeated_records_accumulate_state() {
        let mut topo = Topology::new();
        topo.add_source("source", "input");
        topo.add_state_store("counts");
        topo.add_processor(
            "counter",
            &["source"],
            CountingProcessor {
                name_: "counter".into(),
                store_name: "counts".into(),
            },
        );
        topo.add_sink("sink", &["counter"], "output");

        topo.process("input", Record::new(b"k", b"v", 0));
        topo.process("input", Record::new(b"k", b"v", 100));
        topo.process("input", Record::new(b"k", b"v", 200));

        let raw = topo.stores.stores["counts"].get(b"k").unwrap();
        let count = u64::from_le_bytes(raw.try_into().unwrap());
        assert_eq!(count, 3);
    }

    #[test]
    fn context_forward_records_to_child() {
        let mut ctx = ProcessorContext::new();
        ctx.forward("child", Record::new(b"k", b"v", 0));
        assert_eq!(ctx.forwarded.len(), 1);
        assert_eq!(ctx.forwarded[0].0, "child");
    }

    #[test]
    fn context_commit_bumps_counter() {
        let mut ctx = ProcessorContext::new();
        ctx.commit();
        ctx.commit();
        assert_eq!(ctx.commits, 2);
    }

    #[test]
    fn context_schedule_records_punctuator() {
        let mut ctx = ProcessorContext::new();
        ctx.schedule(ScheduledPunctuator::new(
            PunctuationKind::WallClock,
            1000,
            "tick",
        ));
        assert_eq!(ctx.scheduled_punctuators.len(), 1);
    }
}
