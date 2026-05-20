// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Admission plugin initializer chain.
//!
//! Upstream: `staging/src/k8s.io/apiserver/pkg/admission/initializer/`
//!
//! Upstream wires admission plugins through a `WantsXxx` capability
//! pattern — a plugin that implements `WantsExternalKubeClientSet` is
//! handed the shared client set; one that implements
//! `WantsSharedInformerFactory` is handed the informer factory; and so
//! on. The central initializer chain walks every plugin and offers it
//! every capability it advertises.
//!
//! The cave port previously constructed plugins manually in
//! `cave-runtime/src/main.rs`, passing each dependency by position. That
//! works but it forces every new plugin to grow another constructor
//! argument and every initialization site to re-thread it. This module
//! ports the upstream `Initializer` chain in a Rust-idiomatic shape:
//! capability traits + a `ChainInitializer` that fans the registered
//! capabilities through every plugin.
//!
//! Tenant invariant: the initializer carries the cluster-wide context
//! that plugins need (shared informers, client builders, audience info)
//! — it must never carry a single tenant_id. Tenant scoping happens
//! per-request in the admission handler.

use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// Capability traits — plugins opt in by implementing one or more of
// these. Mirrors upstream `WantsXxx` interfaces.
// ─────────────────────────────────────────────────────────────────────────────

/// Plugin wants the cluster's shared informer factory handle. The
/// concrete handle is opaque to this module; we pass it as `Arc<dyn …>`.
pub trait WantsSharedInformerFactory: Send + Sync {
    fn set_informer_factory(&self, factory: Arc<dyn SharedInformerFactory>);
}

/// Plugin wants the cluster API client builder.
pub trait WantsClientBuilder: Send + Sync {
    fn set_client_builder(&self, builder: Arc<dyn ClientBuilder>);
}

/// Plugin wants the audience list (used by ServiceAccount token plugin
/// + DPoP plugin to validate audiences).
pub trait WantsAudiences: Send + Sync {
    fn set_audiences(&self, audiences: Vec<String>);
}

/// Plugin wants the authorizer chain (RBAC + Node + FieldRBAC) so it
/// can defer to existing authz before mutating.
pub trait WantsAuthorizer: Send + Sync {
    fn set_authorizer(&self, authz: Arc<dyn Authorizer>);
}

// ─────────────────────────────────────────────────────────────────────────────
// Opaque capability handles. Cave deliberately uses small trait objects
// here so this module does not pull cave-kernel and cave-etcd into a
// new dependency cycle; the real implementations live elsewhere and
// just satisfy these tiny shapes.
// ─────────────────────────────────────────────────────────────────────────────

pub trait SharedInformerFactory: Send + Sync {
    /// Cache key — used in tests to identify the factory.
    fn name(&self) -> &str;
}

pub trait ClientBuilder: Send + Sync {
    fn user_agent(&self) -> &str;
}

pub trait Authorizer: Send + Sync {
    /// Tiny stub — the real cave authorizer lives in `rbac.rs`. The
    /// initializer chain only needs to hand a typed handle to plugins.
    fn name(&self) -> &str;
}

// ─────────────────────────────────────────────────────────────────────────────
// Plugin shape. The chain walks `Plugin` trait objects; plugins
// downcast to capability traits via `dyn Any` if they advertise one.
// ─────────────────────────────────────────────────────────────────────────────

/// Lightweight admission plugin handle. Real plugins implement
/// `Plugin` plus one or more capability traits.
pub trait AdmissionPlugin: Send + Sync {
    /// Stable plugin name (e.g. "NamespaceLifecycle", "ResourceQuota").
    fn name(&self) -> &str;

    /// Final validation hook — called once after every capability has
    /// been offered. Plugins use this to assert their config is now
    /// complete. Default: ok.
    fn validate_initialization(&self) -> Result<(), String> {
        Ok(())
    }
}

/// The unified plugin entry the chain operates on. The capability
/// methods return `Option<&dyn …>`; a `None` answer means the plugin
/// does not opt into that capability.
pub trait AdmissionInitializer: AdmissionPlugin {
    fn as_wants_informer(&self) -> Option<&dyn WantsSharedInformerFactory> {
        None
    }
    fn as_wants_client(&self) -> Option<&dyn WantsClientBuilder> {
        None
    }
    fn as_wants_audiences(&self) -> Option<&dyn WantsAudiences> {
        None
    }
    fn as_wants_authorizer(&self) -> Option<&dyn WantsAuthorizer> {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ChainInitializer — registers cluster-wide capabilities and offers
// each one to every plugin during `initialize`. Builder pattern.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ChainInitializer {
    informer_factory: Option<Arc<dyn SharedInformerFactory>>,
    client_builder: Option<Arc<dyn ClientBuilder>>,
    audiences: Option<Vec<String>>,
    authorizer: Option<Arc<dyn Authorizer>>,
}

impl ChainInitializer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_informer_factory(mut self, f: Arc<dyn SharedInformerFactory>) -> Self {
        self.informer_factory = Some(f);
        self
    }

    pub fn with_client_builder(mut self, c: Arc<dyn ClientBuilder>) -> Self {
        self.client_builder = Some(c);
        self
    }

    pub fn with_audiences<I: IntoIterator<Item = impl Into<String>>>(mut self, aud: I) -> Self {
        self.audiences = Some(aud.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_authorizer(mut self, a: Arc<dyn Authorizer>) -> Self {
        self.authorizer = Some(a);
        self
    }

    /// True iff every required capability has been registered. The
    /// minimum cave requires for a plugin chain to function is an
    /// informer factory; the rest are optional.
    pub fn is_minimal_ready(&self) -> bool {
        self.informer_factory.is_some()
    }

    /// Walk each plugin once, offering it every advertised capability.
    /// Returns the list of `(plugin_name, error)` pairs raised by
    /// `validate_initialization` so the caller can fail-stop cleanly.
    pub fn initialize(&self, plugins: &[Arc<dyn AdmissionInitializer>]) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        for p in plugins {
            if let (Some(w), Some(f)) = (p.as_wants_informer(), self.informer_factory.as_ref()) {
                w.set_informer_factory(f.clone());
            }
            if let (Some(w), Some(c)) = (p.as_wants_client(), self.client_builder.as_ref()) {
                w.set_client_builder(c.clone());
            }
            if let (Some(w), Some(aud)) = (p.as_wants_audiences(), self.audiences.as_ref()) {
                w.set_audiences(aud.clone());
            }
            if let (Some(w), Some(a)) = (p.as_wants_authorizer(), self.authorizer.as_ref()) {
                w.set_authorizer(a.clone());
            }
            if let Err(e) = p.validate_initialization() {
                errors.push((p.name().to_string(), e));
            }
        }
        errors
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — exercise the capability dispatcher with two realistic plugin
// shapes: a NamespaceLifecycle stand-in that wants informer + authorizer,
// and a ServiceAccountToken stand-in that wants audiences.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::RwLock;

    struct FakeFactory(&'static str);
    impl SharedInformerFactory for FakeFactory {
        fn name(&self) -> &str {
            self.0
        }
    }

    struct FakeClient(&'static str);
    impl ClientBuilder for FakeClient {
        fn user_agent(&self) -> &str {
            self.0
        }
    }

    struct FakeAuthz(&'static str);
    impl Authorizer for FakeAuthz {
        fn name(&self) -> &str {
            self.0
        }
    }

    /// Plugin that wants informer + authorizer (mirrors NamespaceLifecycle).
    #[derive(Default)]
    struct NamespacePlugin {
        seen_factory: RwLock<Option<String>>,
        seen_authz: RwLock<Option<String>>,
    }

    impl AdmissionPlugin for NamespacePlugin {
        fn name(&self) -> &str {
            "NamespaceLifecycle"
        }
        fn validate_initialization(&self) -> Result<(), String> {
            if self.seen_factory.try_read().unwrap().is_none() {
                return Err("informer factory not wired".into());
            }
            Ok(())
        }
    }

    impl WantsSharedInformerFactory for NamespacePlugin {
        fn set_informer_factory(&self, factory: Arc<dyn SharedInformerFactory>) {
            *self.seen_factory.try_write().unwrap() = Some(factory.name().to_string());
        }
    }

    impl WantsAuthorizer for NamespacePlugin {
        fn set_authorizer(&self, authz: Arc<dyn Authorizer>) {
            *self.seen_authz.try_write().unwrap() = Some(authz.name().to_string());
        }
    }

    impl AdmissionInitializer for NamespacePlugin {
        fn as_wants_informer(&self) -> Option<&dyn WantsSharedInformerFactory> {
            Some(self)
        }
        fn as_wants_authorizer(&self) -> Option<&dyn WantsAuthorizer> {
            Some(self)
        }
    }

    /// Plugin that wants audiences only (mirrors ServiceAccountToken).
    #[derive(Default)]
    struct SaTokenPlugin {
        seen_audiences: RwLock<Vec<String>>,
    }

    impl AdmissionPlugin for SaTokenPlugin {
        fn name(&self) -> &str {
            "ServiceAccountToken"
        }
    }

    impl WantsAudiences for SaTokenPlugin {
        fn set_audiences(&self, audiences: Vec<String>) {
            *self.seen_audiences.try_write().unwrap() = audiences;
        }
    }

    impl AdmissionInitializer for SaTokenPlugin {
        fn as_wants_audiences(&self) -> Option<&dyn WantsAudiences> {
            Some(self)
        }
    }

    /// Plugin that opts into nothing — must still pass through cleanly.
    struct NoopPlugin;
    impl AdmissionPlugin for NoopPlugin {
        fn name(&self) -> &str {
            "Noop"
        }
    }
    impl AdmissionInitializer for NoopPlugin {}

    #[test]
    fn chain_initializer_fans_informer_and_authorizer_to_namespace_plugin() {
        let ns = Arc::new(NamespacePlugin::default());
        let chain = ChainInitializer::new()
            .with_informer_factory(Arc::new(FakeFactory("primary")))
            .with_authorizer(Arc::new(FakeAuthz("rbac")));
        let plugins: Vec<Arc<dyn AdmissionInitializer>> = vec![ns.clone()];
        let errors = chain.initialize(&plugins);
        assert!(errors.is_empty(), "no plugin should fail: {:?}", errors);
        assert_eq!(
            ns.seen_factory.try_read().unwrap().clone(),
            Some("primary".to_string())
        );
        assert_eq!(
            ns.seen_authz.try_read().unwrap().clone(),
            Some("rbac".to_string())
        );
    }

    #[test]
    fn chain_initializer_fans_audiences_only_to_sa_token_plugin() {
        let sa = Arc::new(SaTokenPlugin::default());
        let chain = ChainInitializer::new()
            .with_informer_factory(Arc::new(FakeFactory("f")))
            .with_audiences(["https://kubernetes.default.svc", "https://kafka"]);
        let plugins: Vec<Arc<dyn AdmissionInitializer>> = vec![sa.clone()];
        let errors = chain.initialize(&plugins);
        assert!(errors.is_empty());
        assert_eq!(
            sa.seen_audiences.try_read().unwrap().clone(),
            vec![
                "https://kubernetes.default.svc".to_string(),
                "https://kafka".to_string()
            ]
        );
    }

    #[test]
    fn validate_initialization_failure_is_collected_not_panicked() {
        // No informer factory wired ⇒ NamespacePlugin::validate fails.
        let ns = Arc::new(NamespacePlugin::default());
        let chain = ChainInitializer::new(); // empty
        let plugins: Vec<Arc<dyn AdmissionInitializer>> = vec![ns.clone()];
        let errors = chain.initialize(&plugins);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "NamespaceLifecycle");
        assert!(errors[0].1.contains("informer"));
    }

    #[test]
    fn noop_plugin_passes_through_without_touching_any_capability() {
        let p = Arc::new(NoopPlugin);
        let chain = ChainInitializer::new()
            .with_informer_factory(Arc::new(FakeFactory("f")))
            .with_client_builder(Arc::new(FakeClient("cave-runtime/0.1")))
            .with_audiences(["aud"])
            .with_authorizer(Arc::new(FakeAuthz("rbac")));
        let plugins: Vec<Arc<dyn AdmissionInitializer>> = vec![p];
        let errors = chain.initialize(&plugins);
        assert!(errors.is_empty());
    }

    #[test]
    fn is_minimal_ready_requires_informer_factory() {
        let chain = ChainInitializer::new();
        assert!(!chain.is_minimal_ready());
        let chain = chain.with_informer_factory(Arc::new(FakeFactory("f")));
        assert!(chain.is_minimal_ready());
    }

    #[test]
    fn chain_initializer_handles_mixed_plugin_set() {
        let ns = Arc::new(NamespacePlugin::default());
        let sa = Arc::new(SaTokenPlugin::default());
        let noop = Arc::new(NoopPlugin);
        let chain = ChainInitializer::new()
            .with_informer_factory(Arc::new(FakeFactory("primary")))
            .with_audiences(["https://example.test"])
            .with_authorizer(Arc::new(FakeAuthz("rbac")));
        let plugins: Vec<Arc<dyn AdmissionInitializer>> =
            vec![ns.clone(), sa.clone(), noop.clone()];
        let errors = chain.initialize(&plugins);
        assert!(errors.is_empty());
        // ns got both capabilities; sa got audiences; noop got nothing.
        assert!(ns.seen_factory.try_read().unwrap().is_some());
        assert!(ns.seen_authz.try_read().unwrap().is_some());
        assert_eq!(sa.seen_audiences.try_read().unwrap().len(), 1);
    }

    #[test]
    fn capability_set_calls_replace_previous_value() {
        // Re-running the chain must overwrite prior wiring, not append.
        let ns = Arc::new(NamespacePlugin::default());
        let chain_a = ChainInitializer::new()
            .with_informer_factory(Arc::new(FakeFactory("first")))
            .with_authorizer(Arc::new(FakeAuthz("rbac")));
        let chain_b = ChainInitializer::new()
            .with_informer_factory(Arc::new(FakeFactory("second")))
            .with_authorizer(Arc::new(FakeAuthz("rbac")));
        let plugins: Vec<Arc<dyn AdmissionInitializer>> = vec![ns.clone()];
        chain_a.initialize(&plugins);
        chain_b.initialize(&plugins);
        assert_eq!(
            ns.seen_factory.try_read().unwrap().clone(),
            Some("second".to_string())
        );
    }
}
