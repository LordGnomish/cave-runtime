// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-functions/worker/src/main/java/org/apache/pulsar/functions/worker/FunctionRuntimeManager.java

//! Function worker — registry of `(name, instance_id)` instances + the
//! orchestration verbs the Functions admin API exposes.

use super::{Function, FunctionInstance, InstanceState, Message};
use crate::error::StreamsResult;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerError {
    AlreadyExists(String),
    NotFound(String),
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerError::AlreadyExists(n) => write!(f, "function {n:?} already exists"),
            WorkerError::NotFound(n) => write!(f, "function {n:?} not found"),
        }
    }
}

impl std::error::Error for WorkerError {}

impl From<WorkerError> for crate::error::StreamsError {
    fn from(e: WorkerError) -> Self {
        crate::error::StreamsError::Internal(e.to_string())
    }
}

/// Worker — owns the per-instance state.
pub struct FunctionWorker {
    instances: Mutex<BTreeMap<String, Vec<Arc<FunctionInstance>>>>,
}

impl Default for FunctionWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionWorker {
    pub fn new() -> Self {
        Self {
            instances: Mutex::new(BTreeMap::new()),
        }
    }

    /// Register `parallelism` instances of `func` under `name`.
    pub fn register(
        &self,
        name: impl Into<String>,
        parallelism: u32,
        func: Arc<dyn Function>,
    ) -> Result<(), WorkerError> {
        let name = name.into();
        let mut inst = self.instances.lock().unwrap();
        if inst.contains_key(&name) {
            return Err(WorkerError::AlreadyExists(name));
        }
        let v: Vec<Arc<FunctionInstance>> = (0..parallelism)
            .map(|i| Arc::new(FunctionInstance::new(name.clone(), i, Arc::clone(&func))))
            .collect();
        inst.insert(name, v);
        Ok(())
    }

    pub fn deregister(&self, name: &str) -> Result<(), WorkerError> {
        let mut inst = self.instances.lock().unwrap();
        if inst.remove(name).is_some() {
            Ok(())
        } else {
            Err(WorkerError::NotFound(name.into()))
        }
    }

    pub fn list(&self) -> Vec<String> {
        self.instances.lock().unwrap().keys().cloned().collect()
    }

    pub fn parallelism(&self, name: &str) -> Option<u32> {
        self.instances
            .lock()
            .unwrap()
            .get(name)
            .map(|v| v.len() as u32)
    }

    /// Start every instance of `name`.
    pub fn start(&self, name: &str) -> Result<(), WorkerError> {
        let inst = self.instances.lock().unwrap();
        let v = inst.get(name).ok_or_else(|| WorkerError::NotFound(name.into()))?;
        for i in v {
            i.start();
        }
        Ok(())
    }

    /// Stop every instance of `name`.
    pub fn stop(&self, name: &str) -> Result<(), WorkerError> {
        let inst = self.instances.lock().unwrap();
        let v = inst.get(name).ok_or_else(|| WorkerError::NotFound(name.into()))?;
        for i in v {
            i.stop();
        }
        Ok(())
    }

    /// Snapshot of states (one entry per instance).
    pub fn instance_states(&self, name: &str) -> Option<Vec<InstanceState>> {
        self.instances
            .lock()
            .unwrap()
            .get(name)
            .map(|v| v.iter().map(|i| i.state()).collect())
    }

    /// Round-robin dispatch one message through `name` — picks an
    /// instance by `instance_id = key.hash() % parallelism`.
    pub fn dispatch(&self, name: &str, msg: Message) -> StreamsResult<Vec<Message>> {
        let inst = self.instances.lock().unwrap();
        let v = inst
            .get(name)
            .ok_or_else(|| crate::error::StreamsError::Internal(format!("function {name:?} not registered")))?;
        if v.is_empty() {
            return Err(crate::error::StreamsError::Internal("no instances".into()));
        }
        let slot = match &msg.key {
            Some(k) => {
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for &b in k {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x100_0000_01b3);
                }
                (h as usize) % v.len()
            }
            None => 0,
        };
        let target = Arc::clone(&v[slot]);
        // Drop the lock before invoking (Function might be slow).
        drop(inst);
        target.invoke(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct IdentityFn;
    impl Function for IdentityFn {
        fn process(&self, input: Message) -> StreamsResult<Vec<Message>> {
            Ok(vec![input])
        }
    }

    #[test]
    fn test_worker_register_returns_alphabetic_listing() {
        // cite: pulsar 4.2.0 FunctionRuntimeManager.listFunctions
        // ensemble = wk-001
        let w = FunctionWorker::new();
        w.register("zeta", 1, Arc::new(IdentityFn)).unwrap();
        w.register("alpha", 1, Arc::new(IdentityFn)).unwrap();
        assert_eq!(w.list(), vec!["alpha", "zeta"]);
    }

    #[test]
    fn test_worker_register_duplicate_errors() {
        // cite: pulsar 4.2.0 FunctionAlreadyExistsException
        // ensemble = wk-002
        let w = FunctionWorker::new();
        w.register("f", 1, Arc::new(IdentityFn)).unwrap();
        let err = w.register("f", 1, Arc::new(IdentityFn));
        assert_eq!(err, Err(WorkerError::AlreadyExists("f".into())));
    }

    #[test]
    fn test_worker_parallelism_returns_instance_count() {
        // cite: pulsar 4.2.0 FunctionConfig.parallelism N → N instances
        // ensemble = wk-003
        let w = FunctionWorker::new();
        w.register("f", 4, Arc::new(IdentityFn)).unwrap();
        assert_eq!(w.parallelism("f"), Some(4));
    }

    #[test]
    fn test_worker_deregister_unknown_errors() {
        // cite: pulsar 4.2.0 FunctionNotFoundException
        // ensemble = wk-004
        let w = FunctionWorker::new();
        let err = w.deregister("nope");
        assert_eq!(err, Err(WorkerError::NotFound("nope".into())));
    }

    #[test]
    fn test_worker_start_transitions_all_instances_to_running() {
        // cite: pulsar 4.2.0 start() applies to every parallelism slot
        // ensemble = wk-005
        let w = FunctionWorker::new();
        w.register("f", 3, Arc::new(IdentityFn)).unwrap();
        w.start("f").unwrap();
        let states = w.instance_states("f").unwrap();
        assert!(states.iter().all(|s| matches!(s, InstanceState::Running)));
    }

    #[test]
    fn test_worker_dispatch_picks_instance_by_key_hash() {
        // cite: pulsar 4.2.0 message routing by key hash
        // ensemble = wk-006
        let w = FunctionWorker::new();
        w.register("f", 2, Arc::new(IdentityFn)).unwrap();
        w.start("f").unwrap();
        let m = Message::new("in", b"hi".to_vec()).with_key(b"k1".to_vec());
        // Should not error — picks one of the running instances.
        let out = w.dispatch("f", m.clone()).unwrap();
        assert_eq!(out[0], m);
    }

    #[test]
    fn test_worker_dispatch_unregistered_errors() {
        // cite: pulsar 4.2.0 dispatch to unknown function errors
        // ensemble = wk-007
        let w = FunctionWorker::new();
        let err = w.dispatch("ghost", Message::new("in", b"x".to_vec()));
        assert!(err.is_err());
    }

    #[test]
    fn test_worker_stop_transitions_all_instances_to_stopped() {
        // cite: pulsar 4.2.0 stop() applies to every parallelism slot
        // ensemble = wk-008
        let w = FunctionWorker::new();
        w.register("f", 2, Arc::new(IdentityFn)).unwrap();
        w.start("f").unwrap();
        w.stop("f").unwrap();
        let states = w.instance_states("f").unwrap();
        assert!(states.iter().all(|s| matches!(s, InstanceState::Stopped)));
    }
}
