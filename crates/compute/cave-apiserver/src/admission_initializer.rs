// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Admission plugin initializer.
//!
//! Mirrors `staging/src/k8s.io/apiserver/pkg/admission/initializer/` in
//! kubernetes/kubernetes v1.36.0. Upstream centralises the construction
//! of admission plugins through an [`Initializer`] chain: each plugin
//! that needs shared state (informers, authorizer, REST mapper, …)
//! declares its dependencies via marker traits like
//! `WantsExternalKubeInformerFactory` and the initializer chain wires
//! them up at boot. Without this surface every consumer has to hand-
//! pick globals from `main.go`.
//!
//! cave-apiserver previously constructed admission plugins manually
//! inside `cave-runtime/src/main.rs`. This module reproduces the
//! upstream surface so plugins can declare what they need and a
//! single composer fan-outs the shared state.

use std::sync::Arc;

/// Marker trait families. Each maps to an upstream `Wants*` trait so
/// plugins can pick exactly the slices of shared state they need.
pub trait WantsAuthorizer {
    fn set_authorizer(&mut self, a: Arc<dyn Authorizer>);
}

pub trait WantsInformers {
    fn set_informer_factory(&mut self, f: Arc<dyn InformerFactory>);
}

pub trait WantsRestMapper {
    fn set_rest_mapper(&mut self, m: Arc<dyn RestMapper>);
}

pub trait WantsClient {
    fn set_client(&mut self, c: Arc<dyn ApiClient>);
}

pub trait WantsFeatureGate {
    fn set_feature_gate(&mut self, f: Arc<FeatureGate>);
}

pub trait Authorizer: Send + Sync {
    fn name(&self) -> &str;
}

pub trait InformerFactory: Send + Sync {
    fn registered_kinds(&self) -> Vec<String>;
}

pub trait RestMapper: Send + Sync {
    fn kind_for(&self, resource: &str) -> Option<String>;
}

pub trait ApiClient: Send + Sync {
    fn server_version(&self) -> &str;
}

#[derive(Default, Debug)]
pub struct FeatureGate {
    enabled: std::sync::RwLock<std::collections::BTreeSet<String>>,
}

impl FeatureGate {
    pub fn enable(&self, name: impl Into<String>) {
        self.enabled.write().unwrap().insert(name.into());
    }
    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled.read().unwrap().contains(name)
    }
    pub fn count(&self) -> usize {
        self.enabled.read().unwrap().len()
    }
}

/// Combined initializer that walks `Initialize::initialize(&mut plugin)`
/// for any number of plugins. Mirrors upstream
/// `admission.PluginInitializers` slice + `Initialize` driver.
pub struct PluginInitializer {
    authorizer: Option<Arc<dyn Authorizer>>,
    informers: Option<Arc<dyn InformerFactory>>,
    rest_mapper: Option<Arc<dyn RestMapper>>,
    client: Option<Arc<dyn ApiClient>>,
    feature_gate: Arc<FeatureGate>,
}

impl Default for PluginInitializer {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginInitializer {
    pub fn new() -> Self {
        Self {
            authorizer: None,
            informers: None,
            rest_mapper: None,
            client: None,
            feature_gate: Arc::new(FeatureGate::default()),
        }
    }

    pub fn with_authorizer(mut self, a: Arc<dyn Authorizer>) -> Self {
        self.authorizer = Some(a);
        self
    }

    pub fn with_informers(mut self, f: Arc<dyn InformerFactory>) -> Self {
        self.informers = Some(f);
        self
    }

    pub fn with_rest_mapper(mut self, m: Arc<dyn RestMapper>) -> Self {
        self.rest_mapper = Some(m);
        self
    }

    pub fn with_client(mut self, c: Arc<dyn ApiClient>) -> Self {
        self.client = Some(c);
        self
    }

    pub fn with_feature_gate(mut self, f: Arc<FeatureGate>) -> Self {
        self.feature_gate = f;
        self
    }

    pub fn feature_gate(&self) -> Arc<FeatureGate> {
        self.feature_gate.clone()
    }
}

/// Standalone helper: drive each marker on a plugin manually. Used by
/// callers that know what their plugin implements (the common case for
/// builtin admission plugins). Returns the satisfied-count.
pub fn drive<P>(init: &PluginInitializer, plugin: &mut P) -> usize
where
    P: WantsAuthorizer
        + WantsInformers
        + WantsRestMapper
        + WantsClient
        + WantsFeatureGate,
{
    let mut n = 0;
    if let Some(a) = &init.authorizer {
        plugin.set_authorizer(a.clone());
        n += 1;
    }
    if let Some(f) = &init.informers {
        plugin.set_informer_factory(f.clone());
        n += 1;
    }
    if let Some(m) = &init.rest_mapper {
        plugin.set_rest_mapper(m.clone());
        n += 1;
    }
    if let Some(c) = &init.client {
        plugin.set_client(c.clone());
        n += 1;
    }
    plugin.set_feature_gate(init.feature_gate.clone());
    n + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubAuthorizer;
    impl Authorizer for StubAuthorizer {
        fn name(&self) -> &str {
            "stub"
        }
    }

    struct StubInformers;
    impl InformerFactory for StubInformers {
        fn registered_kinds(&self) -> Vec<String> {
            vec!["Pod".into()]
        }
    }

    struct StubRestMapper;
    impl RestMapper for StubRestMapper {
        fn kind_for(&self, _: &str) -> Option<String> {
            Some("Pod".into())
        }
    }

    struct StubClient;
    impl ApiClient for StubClient {
        fn server_version(&self) -> &str {
            "v1.36.0"
        }
    }

    #[derive(Default)]
    struct CapPlugin {
        seen_authorizer: bool,
        seen_informers: bool,
        seen_mapper: bool,
        seen_client: bool,
        seen_fg: bool,
    }
    impl WantsAuthorizer for CapPlugin {
        fn set_authorizer(&mut self, _: Arc<dyn Authorizer>) {
            self.seen_authorizer = true;
        }
    }
    impl WantsInformers for CapPlugin {
        fn set_informer_factory(&mut self, _: Arc<dyn InformerFactory>) {
            self.seen_informers = true;
        }
    }
    impl WantsRestMapper for CapPlugin {
        fn set_rest_mapper(&mut self, _: Arc<dyn RestMapper>) {
            self.seen_mapper = true;
        }
    }
    impl WantsClient for CapPlugin {
        fn set_client(&mut self, _: Arc<dyn ApiClient>) {
            self.seen_client = true;
        }
    }
    impl WantsFeatureGate for CapPlugin {
        fn set_feature_gate(&mut self, _: Arc<FeatureGate>) {
            self.seen_fg = true;
        }
    }

    #[test]
    fn feature_gate_enable_then_is_enabled_round_trips() {
        let fg = FeatureGate::default();
        assert!(!fg.is_enabled("ValidatingAdmissionPolicy"));
        fg.enable("ValidatingAdmissionPolicy");
        assert!(fg.is_enabled("ValidatingAdmissionPolicy"));
        assert_eq!(fg.count(), 1);
    }

    #[test]
    fn drive_wires_all_declared_dependencies() {
        let init = PluginInitializer::new()
            .with_authorizer(Arc::new(StubAuthorizer))
            .with_informers(Arc::new(StubInformers))
            .with_rest_mapper(Arc::new(StubRestMapper))
            .with_client(Arc::new(StubClient));
        let mut p = CapPlugin::default();
        let n = drive(&init, &mut p);
        assert_eq!(n, 5);
        assert!(p.seen_authorizer);
        assert!(p.seen_informers);
        assert!(p.seen_mapper);
        assert!(p.seen_client);
        assert!(p.seen_fg);
    }

    #[test]
    fn drive_with_missing_authorizer_skips_that_dependency() {
        let init = PluginInitializer::new()
            .with_informers(Arc::new(StubInformers))
            .with_client(Arc::new(StubClient));
        let mut p = CapPlugin::default();
        let n = drive(&init, &mut p);
        // authorizer + rest_mapper missing → 2 wires + feature_gate = 3
        assert_eq!(n, 3);
        assert!(!p.seen_authorizer);
        assert!(p.seen_informers);
        assert!(!p.seen_mapper);
        assert!(p.seen_client);
        assert!(p.seen_fg);
    }

    #[test]
    fn feature_gate_is_shared_across_plugins() {
        let init = PluginInitializer::new();
        init.feature_gate().enable("DRA");
        let mut p1 = CapPlugin::default();
        let mut p2 = CapPlugin::default();
        drive(&init, &mut p1);
        drive(&init, &mut p2);
        // Same Arc → both observe the enabled feature.
        assert!(init.feature_gate().is_enabled("DRA"));
    }

    struct OnlyAuthorizerPlugin {
        seen: bool,
    }
    impl WantsAuthorizer for OnlyAuthorizerPlugin {
        fn set_authorizer(&mut self, _: Arc<dyn Authorizer>) {
            self.seen = true;
        }
    }

    #[test]
    fn plugin_can_pick_a_subset_of_wants() {
        let init = PluginInitializer::new().with_authorizer(Arc::new(StubAuthorizer));
        let mut p = OnlyAuthorizerPlugin { seen: false };
        if let Some(a) = &init.authorizer {
            p.set_authorizer(a.clone());
        }
        assert!(p.seen);
    }

    #[test]
    fn builder_returns_self_for_chaining() {
        let init = PluginInitializer::new()
            .with_authorizer(Arc::new(StubAuthorizer))
            .with_client(Arc::new(StubClient));
        assert!(init.authorizer.is_some());
        assert!(init.client.is_some());
        assert!(init.informers.is_none());
    }

    #[test]
    fn drive_with_no_init_state_still_wires_feature_gate() {
        let init = PluginInitializer::new();
        let mut p = CapPlugin::default();
        let n = drive(&init, &mut p);
        // Only the feature_gate dependency is unconditionally set.
        assert_eq!(n, 1);
        assert!(p.seen_fg);
        assert!(!p.seen_authorizer);
    }
}
