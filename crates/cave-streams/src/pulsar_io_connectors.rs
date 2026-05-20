// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulsar IO connector framework — parallel to Kafka Connect.
//!
//! upstream: apache/pulsar — pulsar-io/.../{Source, Sink, FunctionWorker,
//! ConnectorContext, MessageRouter}
//!
//! Pulsar IO mirrors Kafka Connect at the broker level: Sources pull
//! data from external systems and write to a topic; Sinks read from a
//! topic and push to an external system. The lifecycle is:
//!
//!   1. user POSTs ConnectorConfig (yaml/json describing class +
//!      parallelism + config + topic);
//!   2. FunctionWorker spawns N instance handles;
//!   3. each instance opens a producer (Source) or consumer (Sink);
//!   4. messages flow through `process()` with at-least-once or
//!      effectively-once semantics.
//!
//! This module ports the broker-side state machine (config validation,
//! parallelism plan, runtime status, ack/nack accounting) plus the
//! `Source`/`Sink` traits. Real connection to external systems is left
//! to plugin crates implementing the traits.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorKind {
    Source,
    Sink,
}

#[derive(Debug, Clone)]
pub struct ConnectorConfig {
    pub name: String,
    pub kind: ConnectorKind,
    pub class_name: String,
    pub topic_name: String,
    pub parallelism: u32,
    pub processing_guarantees: ProcessingGuarantees,
    pub configs: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessingGuarantees {
    AtMostOnce,
    AtLeastOnce,
    EffectivelyOnce,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    EmptyName,
    EmptyClass,
    EmptyTopic,
    ZeroParallelism,
    UnknownProcessingGuarantee(String),
}

impl ConnectorConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.name.is_empty() {
            return Err(ConfigError::EmptyName);
        }
        if self.class_name.is_empty() {
            return Err(ConfigError::EmptyClass);
        }
        if self.topic_name.is_empty() {
            return Err(ConfigError::EmptyTopic);
        }
        if self.parallelism == 0 {
            return Err(ConfigError::ZeroParallelism);
        }
        Ok(())
    }
}

pub fn parse_processing_guarantee(raw: &str) -> Result<ProcessingGuarantees, ConfigError> {
    match raw.to_ascii_uppercase().as_str() {
        "ATMOST_ONCE" | "ATMOSTONCE" => Ok(ProcessingGuarantees::AtMostOnce),
        "ATLEAST_ONCE" | "ATLEASTONCE" => Ok(ProcessingGuarantees::AtLeastOnce),
        "EFFECTIVELY_ONCE" | "EFFECTIVELYONCE" => Ok(ProcessingGuarantees::EffectivelyOnce),
        other => Err(ConfigError::UnknownProcessingGuarantee(other.to_string())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceState {
    Initializing,
    Running,
    Paused,
    Failed(String),
    Stopped,
}

impl Default for InstanceState {
    fn default() -> Self {
        InstanceState::Initializing
    }
}

#[derive(Default, Debug, Clone)]
pub struct InstanceStatus {
    pub id: u32,
    pub state: InstanceState,
    pub messages_processed: u64,
    pub messages_failed: u64,
    pub last_error: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct ConnectorRuntime {
    pub config: Option<ConnectorConfig>,
    pub instances: Vec<InstanceStatus>,
}

impl ConnectorRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install a connector config. Returns a per-instance plan (one
    /// `InstanceStatus` per parallelism slot, all `Initializing`).
    pub fn install(&mut self, cfg: ConnectorConfig) -> Result<Vec<u32>, ConfigError> {
        cfg.validate()?;
        let parallelism = cfg.parallelism;
        self.config = Some(cfg);
        self.instances = (0..parallelism)
            .map(|i| InstanceStatus {
                id: i,
                ..Default::default()
            })
            .collect();
        Ok(self.instances.iter().map(|s| s.id).collect())
    }

    pub fn start(&mut self) {
        for i in self.instances.iter_mut() {
            if matches!(
                i.state,
                InstanceState::Initializing | InstanceState::Stopped | InstanceState::Paused
            ) {
                i.state = InstanceState::Running;
            }
        }
    }

    pub fn pause(&mut self) {
        for i in self.instances.iter_mut() {
            if matches!(i.state, InstanceState::Running) {
                i.state = InstanceState::Paused;
            }
        }
    }

    pub fn fail(&mut self, instance_id: u32, msg: &str) {
        if let Some(slot) = self.instances.iter_mut().find(|s| s.id == instance_id) {
            slot.state = InstanceState::Failed(msg.to_string());
            slot.last_error = Some(msg.to_string());
        }
    }

    pub fn record_processed(&mut self, instance_id: u32, count: u64) {
        if let Some(slot) = self.instances.iter_mut().find(|s| s.id == instance_id) {
            slot.messages_processed += count;
        }
    }

    pub fn record_failed(&mut self, instance_id: u32, count: u64) {
        if let Some(slot) = self.instances.iter_mut().find(|s| s.id == instance_id) {
            slot.messages_failed += count;
        }
    }

    pub fn aggregate_processed(&self) -> u64 {
        self.instances.iter().map(|s| s.messages_processed).sum()
    }
}

// ─── Traits ─────────────────────────────────────────────────────────────

/// Source connector — pulls bytes from an external system and writes
/// them to a Pulsar topic.
pub trait PulsarSource: Send + Sync {
    fn open(&mut self, ctx: &SourceContext) -> Result<(), String>;
    fn poll(&mut self) -> Vec<SourceRecord>;
    fn ack(&mut self, record: &SourceRecord) -> Result<(), String>;
    fn close(&mut self);
}

#[derive(Debug, Clone)]
pub struct SourceContext {
    pub instance_id: u32,
    pub topic_name: String,
    pub configs: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SourceRecord {
    pub key: Option<String>,
    pub value: Vec<u8>,
    pub event_time_ms: i64,
    pub properties: HashMap<String, String>,
    pub partition_key: Option<String>,
    pub sequence_id: Option<u64>,
}

/// Sink connector — reads from a Pulsar topic and writes to an
/// external system.
pub trait PulsarSink: Send + Sync {
    fn open(&mut self, ctx: &SinkContext) -> Result<(), String>;
    fn write(&mut self, record: &SinkRecord) -> Result<(), String>;
    fn flush(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn close(&mut self);
}

#[derive(Debug, Clone)]
pub struct SinkContext {
    pub instance_id: u32,
    pub topic_name: String,
    pub configs: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SinkRecord {
    pub key: Option<String>,
    pub value: Vec<u8>,
    pub event_time_ms: i64,
    pub message_id: (u64, u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, kind: ConnectorKind, parallelism: u32) -> ConnectorConfig {
        ConnectorConfig {
            name: name.into(),
            kind,
            class_name: "io.example.Connector".into(),
            topic_name: "persistent://t/n/topic".into(),
            parallelism,
            processing_guarantees: ProcessingGuarantees::AtLeastOnce,
            configs: HashMap::new(),
        }
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut c = cfg("", ConnectorKind::Source, 1);
        c.name = String::new();
        assert_eq!(c.validate(), Err(ConfigError::EmptyName));
    }

    #[test]
    fn validate_rejects_zero_parallelism() {
        assert_eq!(
            cfg("c", ConnectorKind::Source, 0).validate(),
            Err(ConfigError::ZeroParallelism)
        );
    }

    #[test]
    fn install_spawns_instances_equal_to_parallelism() {
        let mut rt = ConnectorRuntime::new();
        let ids = rt.install(cfg("c", ConnectorKind::Source, 4)).unwrap();
        assert_eq!(ids, vec![0, 1, 2, 3]);
        assert_eq!(rt.instances.len(), 4);
    }

    #[test]
    fn start_transitions_init_to_running() {
        let mut rt = ConnectorRuntime::new();
        rt.install(cfg("c", ConnectorKind::Source, 2)).unwrap();
        rt.start();
        assert!(
            rt.instances
                .iter()
                .all(|i| i.state == InstanceState::Running)
        );
    }

    #[test]
    fn pause_only_affects_running_instances() {
        let mut rt = ConnectorRuntime::new();
        rt.install(cfg("c", ConnectorKind::Source, 2)).unwrap();
        rt.fail(1, "boom");
        rt.start();
        rt.pause();
        assert_eq!(rt.instances[0].state, InstanceState::Paused);
        assert!(matches!(rt.instances[1].state, InstanceState::Failed(_)));
    }

    #[test]
    fn fail_records_error_message() {
        let mut rt = ConnectorRuntime::new();
        rt.install(cfg("c", ConnectorKind::Source, 1)).unwrap();
        rt.fail(0, "kaboom");
        assert!(matches!(rt.instances[0].state, InstanceState::Failed(ref m) if m == "kaboom"));
        assert_eq!(rt.instances[0].last_error.as_deref(), Some("kaboom"));
    }

    #[test]
    fn record_processed_aggregates_across_instances() {
        let mut rt = ConnectorRuntime::new();
        rt.install(cfg("c", ConnectorKind::Source, 3)).unwrap();
        rt.record_processed(0, 10);
        rt.record_processed(1, 5);
        rt.record_processed(2, 3);
        assert_eq!(rt.aggregate_processed(), 18);
    }

    #[test]
    fn parse_processing_guarantee_matches_known_strings() {
        assert_eq!(
            parse_processing_guarantee("ATLEAST_ONCE").unwrap(),
            ProcessingGuarantees::AtLeastOnce
        );
        assert_eq!(
            parse_processing_guarantee("effectivelyonce").unwrap(),
            ProcessingGuarantees::EffectivelyOnce
        );
    }

    #[test]
    fn parse_processing_guarantee_rejects_unknown() {
        assert!(parse_processing_guarantee("EVENTUAL").is_err());
    }

    // ─── trait sanity (in-mem source/sink) ───────────────────────────

    struct CountingSource {
        emitted: u32,
        open_called: bool,
        close_called: bool,
    }

    impl PulsarSource for CountingSource {
        fn open(&mut self, _ctx: &SourceContext) -> Result<(), String> {
            self.open_called = true;
            Ok(())
        }
        fn poll(&mut self) -> Vec<SourceRecord> {
            self.emitted += 1;
            vec![SourceRecord {
                key: Some(format!("k-{}", self.emitted)),
                value: vec![self.emitted as u8],
                event_time_ms: self.emitted as i64,
                properties: HashMap::new(),
                partition_key: None,
                sequence_id: Some(self.emitted as u64),
            }]
        }
        fn ack(&mut self, _record: &SourceRecord) -> Result<(), String> {
            Ok(())
        }
        fn close(&mut self) {
            self.close_called = true;
        }
    }

    #[test]
    fn pulsar_source_lifecycle_open_poll_ack_close() {
        let mut src = CountingSource {
            emitted: 0,
            open_called: false,
            close_called: false,
        };
        let ctx = SourceContext {
            instance_id: 0,
            topic_name: "t".into(),
            configs: HashMap::new(),
        };
        src.open(&ctx).unwrap();
        let recs = src.poll();
        assert_eq!(recs.len(), 1);
        src.ack(&recs[0]).unwrap();
        src.close();
        assert!(src.open_called);
        assert!(src.close_called);
    }
}
