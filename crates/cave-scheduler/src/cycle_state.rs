// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CycleState — per-scheduling-cycle scratch space shared between plugins.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/cycle_state.go
//!
//! A `CycleState` carries plugin-specific data computed in PreFilter/PreScore
//! through to Filter/Score/Reserve/PreBind etc. of the same cycle. Each plugin
//! owns its own keys; values are downcast on read.
//!
//! Behaviour parity:
//! - `write(key, val)` stores any `Any + Send + Sync` value
//! - `read(key)` clones and downcasts; returns `None` on miss / wrong type
//! - `delete(key)` removes a key
//! - skip lists for Filter / Score plugins (PreFilter→Filter and
//!   PreScore→Score `Skip` propagation)
//! - record-plugin-metrics toggle for diagnostic mode

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

type Slot = Box<dyn Any + Send + Sync>;

/// Shared per-cycle key-value store.
pub struct CycleState {
    data: Mutex<HashMap<String, Slot>>,
    record_plugin_metrics: Mutex<bool>,
    skip_filter_plugins: Mutex<Vec<String>>,
    skip_score_plugins: Mutex<Vec<String>>,
}

impl CycleState {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
            record_plugin_metrics: Mutex::new(false),
            skip_filter_plugins: Mutex::new(Vec::new()),
            skip_score_plugins: Mutex::new(Vec::new()),
        }
    }

    fn data(&self) -> MutexGuard<'_, HashMap<String, Slot>> {
        self.data.lock().expect("CycleState poisoned")
    }

    /// Write a value under `key`. Replaces any existing value.
    pub fn write<T: Any + Send + Sync>(&self, key: impl Into<String>, value: T) {
        self.data().insert(key.into(), Box::new(value));
    }

    pub fn has(&self, key: &str) -> bool {
        self.data().contains_key(key)
    }

    pub fn delete(&self, key: &str) {
        self.data().remove(key);
    }

    /// Read a clone of the value at `key`, downcast to `T`.
    pub fn read<T: Any + Send + Sync + Clone>(&self, key: &str) -> Option<T> {
        self.data().get(key).and_then(|slot| slot.downcast_ref::<T>().cloned())
    }

    /// Apply a closure to a mutable reference to the value at `key`.
    pub fn modify<T, R>(&self, key: &str, f: impl FnOnce(&mut T) -> R) -> Option<R>
    where
        T: Any + Send + Sync,
    {
        let mut data = self.data();
        let slot = data.get_mut(key)?;
        let target = slot.downcast_mut::<T>()?;
        Some(f(target))
    }

    pub fn set_record_plugin_metrics(&self, enabled: bool) {
        *self.record_plugin_metrics.lock().expect("poisoned") = enabled;
    }

    pub fn should_record_plugin_metrics(&self) -> bool {
        *self.record_plugin_metrics.lock().expect("poisoned")
    }

    /// Mark a Filter plugin as skipped for the rest of the cycle. The
    /// framework consults this list before invoking each Filter plugin.
    pub fn mark_filter_skipped(&self, plugin: impl Into<String>) {
        self.skip_filter_plugins.lock().expect("poisoned").push(plugin.into());
    }

    pub fn should_skip_filter(&self, plugin: &str) -> bool {
        self.skip_filter_plugins.lock().expect("poisoned").iter().any(|p| p == plugin)
    }

    pub fn mark_score_skipped(&self, plugin: impl Into<String>) {
        self.skip_score_plugins.lock().expect("poisoned").push(plugin.into());
    }

    pub fn should_skip_score(&self, plugin: &str) -> bool {
        self.skip_score_plugins.lock().expect("poisoned").iter().any(|p| p == plugin)
    }
}

impl Default for CycleState {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<String> = self.data().keys().cloned().collect();
        f.debug_struct("CycleState").field("keys", &keys).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct PreFilterPodInfo {
        cpu: u64,
        mem: u64,
    }

    #[test]
    fn write_and_read_round_trip() {
        let s = CycleState::new();
        s.write("foo", PreFilterPodInfo { cpu: 500, mem: 1024 });
        let got: PreFilterPodInfo = s.read("foo").unwrap();
        assert_eq!(got, PreFilterPodInfo { cpu: 500, mem: 1024 });
    }

    #[test]
    fn read_missing_key_returns_none() {
        let s = CycleState::new();
        assert!(s.read::<PreFilterPodInfo>("missing").is_none());
    }

    #[test]
    fn read_wrong_type_returns_none() {
        let s = CycleState::new();
        s.write("k", 42u64);
        assert!(s.read::<String>("k").is_none());
    }

    #[test]
    fn delete_removes_key() {
        let s = CycleState::new();
        s.write("k", 1u64);
        assert!(s.has("k"));
        s.delete("k");
        assert!(!s.has("k"));
    }

    #[test]
    fn write_overwrites() {
        let s = CycleState::new();
        s.write("k", 1u64);
        s.write("k", 2u64);
        assert_eq!(s.read::<u64>("k"), Some(2));
    }

    #[test]
    fn modify_mutates_in_place() {
        let s = CycleState::new();
        s.write("k", 10u64);
        let returned = s.modify::<u64, _>("k", |v| {
            *v += 5;
            *v
        });
        assert_eq!(returned, Some(15));
        assert_eq!(s.read::<u64>("k"), Some(15));
    }

    #[test]
    fn modify_missing_returns_none() {
        let s = CycleState::new();
        let r = s.modify::<u64, _>("k", |v| *v);
        assert_eq!(r, None);
    }

    #[test]
    fn record_plugin_metrics_toggle() {
        let s = CycleState::new();
        assert!(!s.should_record_plugin_metrics());
        s.set_record_plugin_metrics(true);
        assert!(s.should_record_plugin_metrics());
        s.set_record_plugin_metrics(false);
        assert!(!s.should_record_plugin_metrics());
    }

    #[test]
    fn skip_filter_plugins_round_trip() {
        let s = CycleState::new();
        assert!(!s.should_skip_filter("X"));
        s.mark_filter_skipped("X");
        assert!(s.should_skip_filter("X"));
        assert!(!s.should_skip_filter("Y"));
    }

    #[test]
    fn skip_score_plugins_round_trip() {
        let s = CycleState::new();
        s.mark_score_skipped("Score");
        assert!(s.should_skip_score("Score"));
        assert!(!s.should_skip_score("Other"));
    }

    #[test]
    fn cycle_state_is_send_sync() {
        fn require<T: Send + Sync>() {}
        require::<CycleState>();
    }

    #[test]
    fn shared_across_threads() {
        use std::sync::Arc;
        use std::thread;
        let s = Arc::new(CycleState::new());
        let s1 = s.clone();
        let h = thread::spawn(move || s1.write("k", 99u64));
        h.join().unwrap();
        assert_eq!(s.read::<u64>("k"), Some(99));
    }

    #[test]
    fn debug_lists_keys() {
        let s = CycleState::new();
        s.write("alpha", 1u64);
        s.write("beta", 2u64);
        let dbg = format!("{:?}", s);
        assert!(dbg.contains("alpha"));
        assert!(dbg.contains("beta"));
    }

    #[test]
    fn complex_value_round_trip() {
        #[derive(Clone, PartialEq, Debug)]
        struct ScoreCache {
            entries: std::collections::HashMap<String, i64>,
        }
        let s = CycleState::new();
        let mut cache = ScoreCache { entries: HashMap::new() };
        cache.entries.insert("nodeA".into(), 42);
        s.write("noderesources/scoreCache", cache.clone());
        let got: ScoreCache = s.read("noderesources/scoreCache").unwrap();
        assert_eq!(got, cache);
    }

    #[test]
    fn skip_filter_marks_persist_within_cycle() {
        let s = CycleState::new();
        s.mark_filter_skipped("NodeName");
        s.mark_filter_skipped("NodePorts");
        assert!(s.should_skip_filter("NodeName"));
        assert!(s.should_skip_filter("NodePorts"));
        assert!(!s.should_skip_filter("NodeAffinity"));
    }
}
