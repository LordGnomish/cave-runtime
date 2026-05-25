// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Pure-Rust kube-bench plugin marketplace — replaces upstream's
//! Go-plugin `.so` loader with a WASM module catalogue + signed
//! manifest. No `dlopen`, no `unsafe`, no shared-library FFI.
//!
//! NOTICE: upstream is aquasecurity/kube-bench (Apache-2.0). The
//! Go-plugin model relies on `plugin.Open(path)` + `Lookup("Check")`
//! which is unsafe by construction (binary-ABI dispatch). cave-bench
//! ships a WASM-component model instead — modules are loaded by a
//! Wasmtime / Wasmer runtime in a separate orchestrator process; this
//! crate owns only the **descriptor + catalogue** types.

use crate::error::{BenchError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One plugin entry as published in a marketplace manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    /// SHA-256 digest of the WASM binary (hex-lowercase, 64 chars).
    pub wasm_sha256: String,
    /// Frameworks the plugin extends (e.g. ["cis", "nsa"]).
    pub frameworks: Vec<String>,
    /// Charter v2 requirement: plugins MUST be pure-Rust WASM, no
    /// `.so` and no JVM. cave-bench refuses to enroll non-`wasm`
    /// runtimes.
    pub runtime: PluginRuntime,
    /// Optional Ed25519 signature over `wasm_sha256` by the author key.
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginRuntime {
    Wasm,
    /// Reserved (Phase 2) — pure-Rust shared-lib alternative via a
    /// dlmopen-free trait-object plugin registry. Not enrollable today.
    InProcRust,
}

/// In-memory catalogue + ID index. Pure data; no I/O.
#[derive(Debug, Default)]
pub struct PluginMarketplace {
    by_id: BTreeMap<String, PluginEntry>,
}

impl PluginMarketplace {
    pub fn new() -> Self { Self::default() }

    pub fn len(&self) -> usize { self.by_id.len() }
    pub fn is_empty(&self) -> bool { self.by_id.is_empty() }

    /// Insert a plugin. Returns Err if `runtime != Wasm` (Charter v2
    /// rejects shared-library loaders) or if `wasm_sha256` is not a
    /// 64-char hex string, or if `id` already exists.
    pub fn enroll(&mut self, entry: PluginEntry) -> Result<()> {
        if entry.runtime != PluginRuntime::Wasm {
            return Err(BenchError::Internal(format!(
                "plugin '{}' runtime '{:?}' rejected: only WASM allowed (Charter v2 no-FFI)",
                entry.id, entry.runtime
            )));
        }
        if entry.wasm_sha256.len() != 64
            || !entry.wasm_sha256.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(BenchError::Internal(format!(
                "plugin '{}' wasm_sha256 must be 64-char hex", entry.id
            )));
        }
        if self.by_id.contains_key(&entry.id) {
            return Err(BenchError::Internal(format!("plugin '{}' already enrolled", entry.id)));
        }
        self.by_id.insert(entry.id.clone(), entry);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&PluginEntry> { self.by_id.get(id) }

    pub fn list(&self) -> Vec<&PluginEntry> { self.by_id.values().collect() }

    pub fn by_framework(&self, fw: &str) -> Vec<&PluginEntry> {
        self.by_id
            .values()
            .filter(|p| p.frameworks.iter().any(|f| f == fw))
            .collect()
    }

    pub fn remove(&mut self, id: &str) -> Option<PluginEntry> { self.by_id.remove(id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> PluginEntry {
        PluginEntry {
            id: id.into(),
            name: id.into(),
            version: "0.1.0".into(),
            author: "test".into(),
            wasm_sha256: "a".repeat(64),
            frameworks: vec!["cis".into()],
            runtime: PluginRuntime::Wasm,
            signature: None,
        }
    }

    #[test]
    fn new_marketplace_is_empty() {
        let m = PluginMarketplace::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn enroll_wasm_plugin_succeeds() {
        let mut m = PluginMarketplace::new();
        m.enroll(entry("p1")).unwrap();
        assert_eq!(m.len(), 1);
        assert!(m.get("p1").is_some());
    }

    #[test]
    fn enroll_rejects_inproc_rust_runtime() {
        let mut m = PluginMarketplace::new();
        let mut e = entry("p1");
        e.runtime = PluginRuntime::InProcRust;
        let r = m.enroll(e);
        assert!(r.is_err());
        assert!(m.is_empty());
    }

    #[test]
    fn enroll_rejects_bad_sha256_length() {
        let mut m = PluginMarketplace::new();
        let mut e = entry("p1");
        e.wasm_sha256 = "deadbeef".into();
        assert!(m.enroll(e).is_err());
    }

    #[test]
    fn enroll_rejects_non_hex_sha256() {
        let mut m = PluginMarketplace::new();
        let mut e = entry("p1");
        e.wasm_sha256 = "z".repeat(64);
        assert!(m.enroll(e).is_err());
    }

    #[test]
    fn enroll_rejects_duplicate_id() {
        let mut m = PluginMarketplace::new();
        m.enroll(entry("p1")).unwrap();
        assert!(m.enroll(entry("p1")).is_err());
    }

    #[test]
    fn by_framework_filters_correctly() {
        let mut m = PluginMarketplace::new();
        let mut p1 = entry("cis-plugin"); p1.frameworks = vec!["cis".into()];
        let mut p2 = entry("nsa-plugin"); p2.frameworks = vec!["nsa".into()];
        let mut p3 = entry("dual"); p3.frameworks = vec!["cis".into(), "nsa".into()];
        m.enroll(p1).unwrap();
        m.enroll(p2).unwrap();
        m.enroll(p3).unwrap();
        assert_eq!(m.by_framework("cis").len(), 2);
        assert_eq!(m.by_framework("nsa").len(), 2);
        assert_eq!(m.by_framework("mitre").len(), 0);
    }

    #[test]
    fn remove_returns_removed_entry() {
        let mut m = PluginMarketplace::new();
        m.enroll(entry("p1")).unwrap();
        let r = m.remove("p1");
        assert!(r.is_some());
        assert!(m.is_empty());
    }

    #[test]
    fn list_orders_by_id_btreemap_iteration() {
        let mut m = PluginMarketplace::new();
        m.enroll(entry("b")).unwrap();
        m.enroll(entry("a")).unwrap();
        let ids: Vec<&str> = m.list().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn entry_round_trips_via_json() {
        let e = entry("p1");
        let j = serde_json::to_string(&e).unwrap();
        let r: PluginEntry = serde_json::from_str(&j).unwrap();
        assert_eq!(e, r);
    }
}
