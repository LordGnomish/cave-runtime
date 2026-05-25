// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runtime handler registry — KEP-585 / RuntimeClass support.
//!
//! Models the CRI v1 `RuntimeHandler` message, which advertises which
//! container runtimes (runc, runsc, kata, …) are available to the kubelet
//! along with the per-runtime feature flags. The kubelet picks one by name
//! through `PodSandboxConfig.runtime_handler` (driven by the
//! RuntimeClass.handler field).
//!
//! Upstream:
//! - containerd: `pkg/cri/server/runtime_handler.go`
//! - kubernetes API: `k8s.io/cri-api/pkg/apis/runtime/v1.RuntimeHandler`

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

/// Per-runtime feature flags advertised in `RuntimeHandler.features`.
///
/// Mirrors `runtime.v1.RuntimeHandlerFeatures` (CRI v1 message).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHandlerFeatures {
    /// `recursive_read_only_mounts` — runtime can apply RRO bind mounts (KEP-3857).
    pub recursive_read_only_mounts: bool,
    /// `user_namespaces` — runtime supports per-pod user namespaces (KEP-127).
    pub user_namespaces: bool,
}

/// A single runtime handler advertised to the kubelet.
///
/// `name` matches `PodSandboxConfig.runtime_handler` and the
/// `handler` field of a Kubernetes RuntimeClass object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHandler {
    pub name: String,
    pub features: RuntimeHandlerFeatures,
}

impl RuntimeHandler {
    /// Build the canonical `runc` handler (default OCI runtime, no extra features).
    pub fn runc() -> Self {
        Self {
            name: "runc".into(),
            features: RuntimeHandlerFeatures {
                recursive_read_only_mounts: true,
                user_namespaces: true,
            },
        }
    }

    /// Build the `runsc` (gVisor) handler — sandboxed runtime.
    pub fn runsc() -> Self {
        Self {
            name: "runsc".into(),
            features: RuntimeHandlerFeatures {
                recursive_read_only_mounts: false,
                user_namespaces: false,
            },
        }
    }

    /// Build the `kata` (Kata Containers) handler — VM-based runtime.
    pub fn kata() -> Self {
        Self {
            name: "kata".into(),
            features: RuntimeHandlerFeatures {
                recursive_read_only_mounts: true,
                user_namespaces: false,
            },
        }
    }
}

/// In-process registry of runtime handlers. Thread-safe; the kubelet may
/// query it concurrently from `RuntimeService.Status`.
#[derive(Debug)]
pub struct RuntimeHandlerRegistry {
    handlers: RwLock<BTreeMap<String, RuntimeHandler>>,
    default_name: RwLock<Option<String>>,
}

impl Default for RuntimeHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeHandlerRegistry {
    /// Empty registry — no handlers and no default.
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(BTreeMap::new()),
            default_name: RwLock::new(None),
        }
    }

    /// Registry preloaded with the standard handlers (`runc`, `runsc`, `kata`),
    /// with `runc` selected as default.
    pub fn with_defaults() -> Self {
        let r = Self::new();
        r.register(RuntimeHandler::runc());
        r.register(RuntimeHandler::runsc());
        r.register(RuntimeHandler::kata());
        r.set_default("runc").expect("runc was just registered");
        r
    }

    /// Register or replace a handler. Returns the previous entry if any.
    pub fn register(&self, handler: RuntimeHandler) -> Option<RuntimeHandler> {
        let mut map = self.handlers.write().unwrap();
        map.insert(handler.name.clone(), handler)
    }

    /// Remove a handler by name. If it was the default, the default is cleared.
    pub fn unregister(&self, name: &str) -> Option<RuntimeHandler> {
        let removed = self.handlers.write().unwrap().remove(name);
        if removed.is_some() {
            let mut def = self.default_name.write().unwrap();
            if def.as_deref() == Some(name) {
                *def = None;
            }
        }
        removed
    }

    /// Look up a handler by name.
    pub fn lookup(&self, name: &str) -> Option<RuntimeHandler> {
        self.handlers.read().unwrap().get(name).cloned()
    }

    /// True if `name` is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.handlers.read().unwrap().contains_key(name)
    }

    /// Number of registered handlers.
    pub fn len(&self) -> usize {
        self.handlers.read().unwrap().len()
    }

    /// True if no handlers are registered.
    pub fn is_empty(&self) -> bool {
        self.handlers.read().unwrap().is_empty()
    }

    /// All registered handlers, sorted by name.
    pub fn list(&self) -> Vec<RuntimeHandler> {
        self.handlers.read().unwrap().values().cloned().collect()
    }

    /// Designate one of the registered handlers as the default. Used when
    /// `PodSandboxConfig.runtime_handler` is empty.
    pub fn set_default(&self, name: &str) -> Result<(), String> {
        if !self.contains(name) {
            return Err(format!("runtime handler not registered: {}", name));
        }
        *self.default_name.write().unwrap() = Some(name.to_string());
        Ok(())
    }

    /// The handler currently marked default, if any.
    pub fn default_handler(&self) -> Option<RuntimeHandler> {
        let name = self.default_name.read().unwrap().clone()?;
        self.lookup(&name)
    }

    /// Resolve the handler that should be used for a sandbox.
    ///
    /// `requested` corresponds to `PodSandboxConfig.runtime_handler` (and
    /// ultimately `RuntimeClass.handler`). Empty string → fall back to the
    /// registry default. Unknown name → `Err`.
    pub fn select_for_sandbox(&self, requested: &str) -> Result<RuntimeHandler, String> {
        if requested.is_empty() {
            return self
                .default_handler()
                .ok_or_else(|| "no default runtime handler configured".to_string());
        }
        self.lookup(requested)
            .ok_or_else(|| format!("runtime handler not found: {}", requested))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RuntimeHandler builders ───────────────────────────────────────────────

    #[test]
    fn runc_builder_advertises_userns_and_rro() {
        let h = RuntimeHandler::runc();
        assert_eq!(h.name, "runc");
        assert!(h.features.recursive_read_only_mounts);
        assert!(h.features.user_namespaces);
    }

    #[test]
    fn runsc_has_no_optional_features() {
        let h = RuntimeHandler::runsc();
        assert_eq!(h.name, "runsc");
        assert!(!h.features.recursive_read_only_mounts);
        assert!(!h.features.user_namespaces);
    }

    #[test]
    fn kata_supports_rro_but_not_userns() {
        let h = RuntimeHandler::kata();
        assert_eq!(h.name, "kata");
        assert!(h.features.recursive_read_only_mounts);
        assert!(!h.features.user_namespaces);
    }

    // ── Registry CRUD ─────────────────────────────────────────────────────────

    #[test]
    fn empty_registry_has_no_handlers() {
        let r = RuntimeHandlerRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(r.list().is_empty());
        assert!(r.default_handler().is_none());
    }

    #[test]
    fn with_defaults_registers_three_handlers() {
        let r = RuntimeHandlerRegistry::with_defaults();
        assert_eq!(r.len(), 3);
        assert!(r.contains("runc"));
        assert!(r.contains("runsc"));
        assert!(r.contains("kata"));
    }

    #[test]
    fn register_returns_previous_value() {
        let r = RuntimeHandlerRegistry::new();
        assert!(r.register(RuntimeHandler::runc()).is_none());
        let mut updated = RuntimeHandler::runc();
        updated.features.user_namespaces = false;
        let prev = r.register(updated).unwrap();
        assert!(prev.features.user_namespaces);
    }

    #[test]
    fn lookup_returns_registered_handler() {
        let r = RuntimeHandlerRegistry::new();
        r.register(RuntimeHandler::runc());
        let h = r.lookup("runc").unwrap();
        assert_eq!(h.name, "runc");
    }

    #[test]
    fn lookup_unknown_handler_returns_none() {
        let r = RuntimeHandlerRegistry::with_defaults();
        assert!(r.lookup("notarealthing").is_none());
    }

    #[test]
    fn unregister_removes_handler() {
        let r = RuntimeHandlerRegistry::with_defaults();
        let removed = r.unregister("kata").unwrap();
        assert_eq!(removed.name, "kata");
        assert!(!r.contains("kata"));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn unregister_unknown_returns_none() {
        let r = RuntimeHandlerRegistry::new();
        assert!(r.unregister("ghost").is_none());
    }

    #[test]
    fn list_is_sorted_by_name() {
        let r = RuntimeHandlerRegistry::with_defaults();
        let names: Vec<String> = r.list().into_iter().map(|h| h.name).collect();
        assert_eq!(names, vec!["kata", "runc", "runsc"]);
    }

    // ── Default handler ───────────────────────────────────────────────────────

    #[test]
    fn set_default_requires_registered_name() {
        let r = RuntimeHandlerRegistry::new();
        assert!(r.set_default("runc").is_err());
        r.register(RuntimeHandler::runc());
        assert!(r.set_default("runc").is_ok());
    }

    #[test]
    fn default_handler_returns_marked_handler() {
        let r = RuntimeHandlerRegistry::with_defaults();
        assert_eq!(r.default_handler().unwrap().name, "runc");
    }

    #[test]
    fn default_handler_changes_when_set_default_called() {
        let r = RuntimeHandlerRegistry::with_defaults();
        r.set_default("kata").unwrap();
        assert_eq!(r.default_handler().unwrap().name, "kata");
    }

    #[test]
    fn unregister_default_clears_default() {
        let r = RuntimeHandlerRegistry::with_defaults();
        r.unregister("runc");
        assert!(r.default_handler().is_none());
    }

    // ── select_for_sandbox ────────────────────────────────────────────────────

    #[test]
    fn select_empty_falls_back_to_default() {
        let r = RuntimeHandlerRegistry::with_defaults();
        let h = r.select_for_sandbox("").unwrap();
        assert_eq!(h.name, "runc");
    }

    #[test]
    fn select_empty_with_no_default_errors() {
        let r = RuntimeHandlerRegistry::new();
        let err = r.select_for_sandbox("").unwrap_err();
        assert!(err.contains("no default"));
    }

    #[test]
    fn select_named_returns_that_handler() {
        let r = RuntimeHandlerRegistry::with_defaults();
        let h = r.select_for_sandbox("kata").unwrap();
        assert_eq!(h.name, "kata");
    }

    #[test]
    fn select_unknown_returns_error_naming_handler() {
        let r = RuntimeHandlerRegistry::with_defaults();
        let err = r.select_for_sandbox("missing").unwrap_err();
        assert!(err.contains("missing"));
        assert!(err.contains("not found"));
    }

    // ── Serialization ────────────────────────────────────────────────────────

    #[test]
    fn runtime_handler_roundtrips_through_json() {
        let h = RuntimeHandler::runc();
        let json = serde_json::to_string(&h).unwrap();
        let back: RuntimeHandler = serde_json::from_str(&json).unwrap();
        assert_eq!(h, back);
    }

    #[test]
    fn runtime_handler_features_roundtrips_through_json() {
        let f = RuntimeHandlerFeatures {
            recursive_read_only_mounts: true,
            user_namespaces: false,
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: RuntimeHandlerFeatures = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn registry_concurrent_register_is_safe() {
        use std::sync::Arc;
        let r = Arc::new(RuntimeHandlerRegistry::new());
        let mut handles = vec![];
        for i in 0..10 {
            let r = r.clone();
            handles.push(std::thread::spawn(move || {
                r.register(RuntimeHandler {
                    name: format!("h-{}", i),
                    features: RuntimeHandlerFeatures::default(),
                });
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(r.len(), 10);
    }
}
