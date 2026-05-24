// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Plugin SDK trait — analogue of falcosecurity's plugin SDK
//! (`plugin/api.h`, `plugin/plugin_loader.cpp`). Plugins extend the
//! engine with new event sources (k8s_audit), new fields, or new
//! extraction filters.
//!
//! NOTICE: dlopen/dlsym plugin runtime + binary ABI are out-of-process
//! per ADR-RUNTIME-SANDBOX-NO-FFI-001. cave-falco loads only in-tree
//! pure-Rust plugins through this trait.

use crate::error::Result;
use crate::event::FalcoEvent;
use serde::{Deserialize, Serialize};

/// Plugin capabilities — mirrors `ss_plugin_caps_t`.
pub mod plugin_caps {
    pub const SOURCE: u8        = 0b0001;
    pub const EXTRACTION: u8    = 0b0010;
    pub const PARSING: u8       = 0b0100;
    pub const ASYNC_EVENTS: u8  = 0b1000;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// Bit mask — see `PluginCaps`.
    pub caps_bits: u8,
}

/// Trait every in-tree plugin must implement.
pub trait Plugin: Send + Sync {
    fn info(&self) -> PluginInfo;

    /// Source plugins emit events into the engine queue. Default:
    /// nothing to emit.
    fn next(&self) -> Result<Option<FalcoEvent>> { Ok(None) }

    /// Extraction plugins implement field lookups beyond the built-in
    /// libsinsp fields. Default: no extra fields.
    fn extract(&self, _ev: &FalcoEvent, _field: &str) -> Option<String> { None }
}

/// In-tree registry (no dlopen).
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, p: Box<dyn Plugin>) { self.plugins.push(p); }

    pub fn len(&self) -> usize { self.plugins.len() }

    pub fn is_empty(&self) -> bool { self.plugins.is_empty() }

    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins.iter().map(|p| p.info()).collect()
    }

    /// Round-robin pump — pull one event per plugin into a vec.
    pub fn pump(&self) -> Result<Vec<FalcoEvent>> {
        let mut out = Vec::new();
        for p in &self.plugins {
            if let Some(ev) = p.next()? {
                out.push(ev);
            }
        }
        Ok(out)
    }

    /// Apply extraction plugins to fill any missing field on the event.
    pub fn extract_into(&self, ev: &mut FalcoEvent, field: &str) {
        if ev.fields.contains_key(field) { return; }
        for p in &self.plugins {
            if let Some(v) = p.extract(ev, field) {
                ev.fields.insert(field.into(), v);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::FalcoEvent;
    use std::sync::Mutex;

    struct Counter {
        n: Mutex<u32>,
    }

    impl Plugin for Counter {
        fn info(&self) -> PluginInfo {
            PluginInfo {
                name: "counter".into(),
                version: "0.1".into(),
                description: "test".into(),
                author: "test".into(),
                caps_bits: 0b0001, // SOURCE
            }
        }
        fn next(&self) -> Result<Option<FalcoEvent>> {
            let mut n = self.n.lock().unwrap();
            *n += 1;
            Ok(Some(FalcoEvent::syscall("tick").with("n", n.to_string())))
        }
    }

    struct Extractor;
    impl Plugin for Extractor {
        fn info(&self) -> PluginInfo {
            PluginInfo {
                name: "ext".into(),
                version: "0.1".into(),
                description: "test".into(),
                author: "test".into(),
                caps_bits: 0b0010, // EXTRACTION
            }
        }
        fn extract(&self, _ev: &FalcoEvent, field: &str) -> Option<String> {
            (field == "ext.derived").then(|| "yes".to_string())
        }
    }

    #[test]
    fn empty_registry_is_empty() {
        let r = PluginRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn registered_plugin_appears_in_list() {
        let mut r = PluginRegistry::new();
        r.register(Box::new(Counter { n: Mutex::new(0) }));
        assert_eq!(r.list().len(), 1);
        assert_eq!(r.list()[0].name, "counter");
    }

    #[test]
    fn pump_collects_events_from_source_plugins() {
        let mut r = PluginRegistry::new();
        r.register(Box::new(Counter { n: Mutex::new(0) }));
        let evs = r.pump().unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].fields.get("evt.type").unwrap(), "tick");
    }

    #[test]
    fn extraction_fills_missing_field_only() {
        let mut r = PluginRegistry::new();
        r.register(Box::new(Extractor));
        let mut ev = FalcoEvent::syscall("openat");
        r.extract_into(&mut ev, "ext.derived");
        assert_eq!(ev.fields.get("ext.derived").unwrap(), "yes");
    }

    #[test]
    fn extraction_skips_when_field_already_present() {
        let mut r = PluginRegistry::new();
        r.register(Box::new(Extractor));
        let mut ev = FalcoEvent::syscall("openat").with("ext.derived", "preset");
        r.extract_into(&mut ev, "ext.derived");
        assert_eq!(ev.fields.get("ext.derived").unwrap(), "preset");
    }

    #[test]
    fn info_caps_bits_carry_through() {
        let c = Counter { n: Mutex::new(0) };
        assert_eq!(c.info().caps_bits, 0b0001);
        assert_eq!(Extractor.info().caps_bits, 0b0010);
    }
}
