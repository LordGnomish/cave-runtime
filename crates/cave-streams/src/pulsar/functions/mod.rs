// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-functions/api-java/src/main/java/org/apache/pulsar/functions/api/Function.java
//   pulsar-functions/instance/src/main/java/org/apache/pulsar/functions/instance/JavaInstance.java
//   pulsar-functions/worker/src/main/java/org/apache/pulsar/functions/worker/FunctionRuntimeManager.java

//! Pulsar Functions runtime — SKELETON.
//!
//! This module establishes the [`Function`] trait, the
//! [`FunctionInstance`] state machine, and the [`FunctionWorker`]
//! registry that knows how to start, stop, and look up function
//! instances by name.  **No process isolation is implemented** —
//! every function runs inline on the same thread that drove the
//! input message.
//!
//! Phase 2 (out of scope for this batch):
//! - Wasm runtime (wasmtime / wasmer)
//! - Process-isolated runtime (podman container per instance)
//! - Java-byte-code class-loader runtime (the upstream default;
//!   requires a JVM and is not on cave-runtime's path)
//!
//! These gaps are documented in the manifest under
//! `[[unmapped]] apache/pulsar:pulsar-functions/runtime-process` and
//! `[[unmapped]] apache/pulsar:pulsar-functions/runtime-wasm`.

pub mod worker;

pub use worker::{FunctionWorker, WorkerError};

use crate::error::StreamsResult;
use std::sync::{Arc, Mutex};

/// A Pulsar Functions message — opaque key/value payload + the source
/// topic name.  Mirrors `org.apache.pulsar.functions.api.Record` minus
/// the broker plumbing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub topic: String,
    pub key: Option<Vec<u8>>,
    pub value: Vec<u8>,
}

impl Message {
    pub fn new(topic: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        Self {
            topic: topic.into(),
            key: None,
            value: value.into(),
        }
    }

    pub fn with_key(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.key = Some(key.into());
        self
    }
}

/// Pulsar Functions Function trait — a stateless transformer
/// `Message -> 0..N Messages`.  Pulsar's Java API has `process(Input,
/// Context)`; the `Context` (counters, secrets, state) is deferred to
/// Phase 2.
pub trait Function: Send + Sync {
    fn process(&self, input: Message) -> StreamsResult<Vec<Message>>;
}

/// Lifecycle of a function instance.  Matches
/// `org.apache.pulsar.functions.proto.Function.Status.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceState {
    Init,
    Running,
    Stopped,
    Failed,
}

/// A function instance — one logical execution of a [`Function`].
pub struct FunctionInstance {
    pub name: String,
    pub instance_id: u32,
    func: Arc<dyn Function>,
    state: Mutex<InstanceState>,
    /// Failure trace for the most recent unhandled error.
    failure: Mutex<Option<String>>,
    /// Counters — successful invocations + failed invocations.
    counters: Mutex<InstanceCounters>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct InstanceCounters {
    pub processed: u64,
    pub failures: u64,
    pub emitted: u64,
}

impl FunctionInstance {
    pub fn new(
        name: impl Into<String>,
        instance_id: u32,
        func: Arc<dyn Function>,
    ) -> Self {
        Self {
            name: name.into(),
            instance_id,
            func,
            state: Mutex::new(InstanceState::Init),
            failure: Mutex::new(None),
            counters: Mutex::new(InstanceCounters::default()),
        }
    }

    pub fn state(&self) -> InstanceState {
        *self.state.lock().unwrap()
    }

    pub fn failure(&self) -> Option<String> {
        self.failure.lock().unwrap().clone()
    }

    pub fn counters(&self) -> InstanceCounters {
        *self.counters.lock().unwrap()
    }

    /// `start()` — Init/Stopped/Failed → Running.  No-op when already
    /// running.
    pub fn start(&self) {
        let mut st = self.state.lock().unwrap();
        if !matches!(*st, InstanceState::Running) {
            *st = InstanceState::Running;
            *self.failure.lock().unwrap() = None;
        }
    }

    /// `stop()` — Running → Stopped.
    pub fn stop(&self) {
        let mut st = self.state.lock().unwrap();
        *st = InstanceState::Stopped;
    }

    /// Push one message through `Function.process`.  Updates counters
    /// and transitions to `Failed` on error (matches upstream
    /// FunctionStatsManager).
    pub fn invoke(&self, input: Message) -> StreamsResult<Vec<Message>> {
        let st = *self.state.lock().unwrap();
        if st != InstanceState::Running {
            return Err(crate::error::StreamsError::Internal(format!(
                "function {:?}/{} not running (state={:?})",
                self.name, self.instance_id, st
            )));
        }
        match self.func.process(input) {
            Ok(out) => {
                let mut c = self.counters.lock().unwrap();
                c.processed += 1;
                c.emitted += out.len() as u64;
                Ok(out)
            }
            Err(e) => {
                let mut c = self.counters.lock().unwrap();
                c.failures += 1;
                *self.failure.lock().unwrap() = Some(e.to_string());
                *self.state.lock().unwrap() = InstanceState::Failed;
                Err(e)
            }
        }
    }

    /// Reset counters + failure trace (Functions admin
    /// "restart instance").
    pub fn reset(&self) {
        *self.counters.lock().unwrap() = InstanceCounters::default();
        *self.failure.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::StreamsError;

    struct UppercaseFn;
    impl Function for UppercaseFn {
        fn process(&self, input: Message) -> StreamsResult<Vec<Message>> {
            let upper: Vec<u8> = input.value.iter().map(|b| b.to_ascii_uppercase()).collect();
            Ok(vec![Message::new("out", upper)])
        }
    }

    struct FailingFn;
    impl Function for FailingFn {
        fn process(&self, _input: Message) -> StreamsResult<Vec<Message>> {
            Err(StreamsError::Internal("boom".into()))
        }
    }

    struct FanoutFn;
    impl Function for FanoutFn {
        fn process(&self, input: Message) -> StreamsResult<Vec<Message>> {
            Ok((0..3)
                .map(|i| Message::new("out", format!("{}-{i}", String::from_utf8_lossy(&input.value))))
                .collect())
        }
    }

    #[test]
    fn test_instance_starts_in_init_state() {
        // cite: pulsar 4.2.0 Function.Status.State.Init initial value
        // ensemble = fn-001
        let inst = FunctionInstance::new("uc", 0, Arc::new(UppercaseFn));
        assert_eq!(inst.state(), InstanceState::Init);
    }

    #[test]
    fn test_instance_start_transitions_to_running() {
        // cite: pulsar 4.2.0 FunctionRuntimeManager.startFunction
        // ensemble = fn-002
        let inst = FunctionInstance::new("uc", 0, Arc::new(UppercaseFn));
        inst.start();
        assert_eq!(inst.state(), InstanceState::Running);
    }

    #[test]
    fn test_instance_invoke_when_not_running_errors() {
        // cite: pulsar 4.2.0 invoke before start rejected
        // ensemble = fn-003
        let inst = FunctionInstance::new("uc", 0, Arc::new(UppercaseFn));
        let err = inst.invoke(Message::new("in", b"hi".to_vec()));
        assert!(err.is_err());
    }

    #[test]
    fn test_instance_invoke_running_returns_transformed_output() {
        // cite: pulsar 4.2.0 Function.process called when Running
        // ensemble = fn-004
        let inst = FunctionInstance::new("uc", 0, Arc::new(UppercaseFn));
        inst.start();
        let out = inst.invoke(Message::new("in", b"abc".to_vec())).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].value, b"ABC");
    }

    #[test]
    fn test_instance_invoke_failure_transitions_to_failed() {
        // cite: pulsar 4.2.0 unhandled error → Failed state
        // ensemble = fn-005
        let inst = FunctionInstance::new("bad", 0, Arc::new(FailingFn));
        inst.start();
        let err = inst.invoke(Message::new("in", b"x".to_vec()));
        assert!(err.is_err());
        assert_eq!(inst.state(), InstanceState::Failed);
        assert!(inst.failure().unwrap().contains("boom"));
    }

    #[test]
    fn test_instance_fanout_function_emits_multiple() {
        // cite: pulsar 4.2.0 Function returning multiple records (fan-out)
        // ensemble = fn-006
        let inst = FunctionInstance::new("fan", 0, Arc::new(FanoutFn));
        inst.start();
        let out = inst.invoke(Message::new("in", b"hi".to_vec())).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn test_instance_counters_track_processed_and_emitted() {
        // cite: pulsar 4.2.0 FunctionStatsManager metrics
        // ensemble = fn-007
        let inst = FunctionInstance::new("fan", 0, Arc::new(FanoutFn));
        inst.start();
        inst.invoke(Message::new("in", b"a".to_vec())).unwrap();
        inst.invoke(Message::new("in", b"b".to_vec())).unwrap();
        let c = inst.counters();
        assert_eq!(c.processed, 2);
        assert_eq!(c.emitted, 6);
        assert_eq!(c.failures, 0);
    }

    #[test]
    fn test_instance_stop_transitions_to_stopped() {
        // cite: pulsar 4.2.0 FunctionRuntimeManager.stopFunction
        // ensemble = fn-008
        let inst = FunctionInstance::new("uc", 0, Arc::new(UppercaseFn));
        inst.start();
        inst.stop();
        assert_eq!(inst.state(), InstanceState::Stopped);
    }

    #[test]
    fn test_instance_restart_clears_failure() {
        // cite: pulsar 4.2.0 admin restart() clears failure trace
        // ensemble = fn-009
        let inst = FunctionInstance::new("bad", 0, Arc::new(FailingFn));
        inst.start();
        let _ = inst.invoke(Message::new("in", b"x".to_vec()));
        assert_eq!(inst.state(), InstanceState::Failed);
        inst.reset();
        inst.start();
        assert_eq!(inst.state(), InstanceState::Running);
        assert!(inst.failure().is_none());
    }
}
