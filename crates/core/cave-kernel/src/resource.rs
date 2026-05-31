// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubernetes-compatible resource plumbing — the canonical
//! `TypeMeta` / `ObjectMeta` / `GroupVersionKind` shapes plus a
//! `Resource` trait that every CAVE API object can implement.
//!
//! Many CAVE modules (cave-apiserver, cave-controller-manager,
//! cave-crossplane, the operators) speak a Kubernetes-style object
//! model: each object carries an `apiVersion`/`kind` header
//! (`TypeMeta`) and per-object identity/metadata (`ObjectMeta`).
//! Each crate used to re-declare these by hand, which made cross-crate
//! plumbing (watch caches, admission, the reconcile loop) re-marshal
//! between subtly-different definitions. This module gives the kernel
//! one faithful copy of `k8s.io/apimachinery`'s `TypeMeta`,
//! `ObjectMeta`, and `GroupVersionKind`.
//!
//! The central rule, lifted directly from apimachinery's
//! `GroupVersion.String()`, is how the `(group, version)` pair folds
//! into the on-the-wire `apiVersion` string:
//!
//! - **core group** (empty group, e.g. Pod/Service) → bare version,
//!   `"v1"`.
//! - **named group** (e.g. `apps`) → `"group/version"`,
//!   `"apps/v1"`.
//!
//! `Resource::api_version()` composes that for you from `gvk()`, so an
//! implementor only declares its `GroupVersionKind` once and never
//! hand-writes the split-and-join logic.
//!
//! Adopters: cave-apiserver (object envelope), cave-controller-manager
//! + the operators (reconcile-loop object identity), cave-crossplane
//!   (composed-resource GVKs).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Per-message type header — the `apiVersion` + `kind` that prefix every
/// Kubernetes object. Mirrors `metav1.TypeMeta`.
///
/// `BTreeMap` is intentionally *not* used here; `TypeMeta` is two flat
/// strings. The `kind` is the un-pluralized resource kind (`"Pod"`,
/// not `"pods"`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeMeta {
    /// e.g. `"v1"` (core) or `"apps/v1"` (named group). Empty for an
    /// unset/embedded object.
    #[serde(default, rename = "apiVersion", skip_serializing_if = "String::is_empty")]
    pub api_version: String,
    /// e.g. `"Pod"`, `"Deployment"`. Empty for an unset object.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

impl TypeMeta {
    pub fn new(api_version: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            api_version: api_version.into(),
            kind: kind.into(),
        }
    }
}

/// Per-object identity + metadata. A faithful (single-node) subset of
/// `metav1.ObjectMeta` — the fields CAVE controllers actually read.
///
/// `labels` and `annotations` use `BTreeMap` so serialization is
/// deterministic (key-sorted), which keeps resource hashes and
/// `apply`/diff comparisons stable across runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMeta {
    /// Object name, unique within its namespace.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Namespace; empty for cluster-scoped objects.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub namespace: String,
    /// Selectable identifying key/value pairs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Non-identifying side metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
    /// Opaque optimistic-concurrency token. Server-assigned; callers
    /// must echo it unchanged on update.
    #[serde(
        default,
        rename = "resourceVersion",
        skip_serializing_if = "String::is_empty"
    )]
    pub resource_version: String,
    /// Cluster-unique identity for this object across its whole
    /// lifetime (survives delete + recreate of the same name).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub uid: String,
}

impl ObjectMeta {
    /// A bare named object — the common controller case.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// A namespaced named object.
    pub fn namespaced(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            ..Self::default()
        }
    }

    /// Builder: add a label.
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// Builder: add an annotation.
    pub fn with_annotation(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.annotations.insert(key.into(), value.into());
        self
    }
}

/// The `(group, version, kind)` triple — apimachinery's
/// `schema.GroupVersionKind`. This is the *unique type coordinate* of a
/// resource; `TypeMeta.api_version` is its serialized projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupVersionKind {
    /// API group; empty string for the legacy "core" group.
    pub group: String,
    /// API version, e.g. `"v1"`, `"v1beta1"`.
    pub version: String,
    /// Resource kind, e.g. `"Pod"`, `"Deployment"`.
    pub kind: String,
}

impl GroupVersionKind {
    /// Construct a GVK. Use `""` for `group` to denote the core group.
    pub fn new(
        group: impl Into<String>,
        version: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            kind: kind.into(),
        }
    }

    /// Compose `group`/`version` into the on-the-wire `apiVersion`
    /// string, matching `schema.GroupVersion.String()`:
    ///
    /// - empty group → bare `version` (`"v1"`)
    /// - named group → `"group/version"` (`"apps/v1"`)
    pub fn to_api_version(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }

    /// Project this GVK into a `TypeMeta` (apiVersion composed + kind).
    pub fn to_type_meta(&self) -> TypeMeta {
        TypeMeta::new(self.to_api_version(), self.kind.clone())
    }
}

impl fmt::Display for GroupVersionKind {
    /// `apimachinery` renders a GVK as `"{group}/{version}, Kind={kind}"`
    /// (with a leading `/` when the group is empty), which is what
    /// `GroupVersionKind.String()` emits and what shows up in
    /// controller/admission errors.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}, Kind={}", self.group, self.version, self.kind)
    }
}

/// Implemented by every CAVE API object. The associated type metadata
/// (`type_meta`, `gvk`) is intrinsic to the *type*, so those are
/// associated functions; `object_meta` is per-instance.
///
/// Implementors only declare `gvk()` (and expose their stored
/// `ObjectMeta`); the provided methods derive everything else, so the
/// `(group, version) → apiVersion` folding rule lives in exactly one
/// place.
pub trait Resource {
    /// The static type coordinate of this resource.
    fn gvk() -> GroupVersionKind
    where
        Self: Sized;

    /// Per-instance metadata (name/namespace/labels/...).
    fn object_meta(&self) -> &ObjectMeta;

    /// The `apiVersion`/`kind` header for this type. Provided: derived
    /// from [`Self::gvk`].
    fn type_meta() -> TypeMeta
    where
        Self: Sized,
    {
        Self::gvk().to_type_meta()
    }

    /// The composed `apiVersion` string for this type. Provided:
    /// derived from [`Self::gvk`] via
    /// [`GroupVersionKind::to_api_version`].
    fn api_version() -> String
    where
        Self: Sized,
    {
        Self::gvk().to_api_version()
    }

    /// The resource kind for this type. Provided: derived from
    /// [`Self::gvk`].
    fn kind() -> String
    where
        Self: Sized,
    {
        Self::gvk().kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- GroupVersionKind::to_api_version composing rule --------------

    #[test]
    fn core_group_api_version_is_bare_version() {
        let gvk = GroupVersionKind::new("", "v1", "Pod");
        assert_eq!(gvk.to_api_version(), "v1");
    }

    #[test]
    fn named_group_api_version_composes_group_slash_version() {
        let gvk = GroupVersionKind::new("apps", "v1", "Deployment");
        assert_eq!(gvk.to_api_version(), "apps/v1");
    }

    #[test]
    fn named_group_beta_version_composes() {
        let gvk = GroupVersionKind::new("networking.k8s.io", "v1beta1", "Ingress");
        assert_eq!(gvk.to_api_version(), "networking.k8s.io/v1beta1");
    }

    // --- GroupVersionKind projections ---------------------------------

    #[test]
    fn to_type_meta_carries_composed_api_version_and_kind() {
        let tm = GroupVersionKind::new("apps", "v1", "Deployment").to_type_meta();
        assert_eq!(tm.api_version, "apps/v1");
        assert_eq!(tm.kind, "Deployment");
    }

    #[test]
    fn gvk_display_matches_apimachinery_format() {
        let core = GroupVersionKind::new("", "v1", "Pod");
        assert_eq!(core.to_string(), "/v1, Kind=Pod");
        let named = GroupVersionKind::new("apps", "v1", "Deployment");
        assert_eq!(named.to_string(), "apps/v1, Kind=Deployment");
    }

    // --- TypeMeta -----------------------------------------------------

    #[test]
    fn type_meta_new_sets_fields() {
        let tm = TypeMeta::new("v1", "Pod");
        assert_eq!(tm.api_version, "v1");
        assert_eq!(tm.kind, "Pod");
    }

    // --- ObjectMeta defaults & builders -------------------------------

    #[test]
    fn object_meta_default_has_empty_labels_and_annotations() {
        let m = ObjectMeta::default();
        assert!(m.labels.is_empty());
        assert!(m.annotations.is_empty());
        assert_eq!(m.name, "");
        assert_eq!(m.namespace, "");
        assert_eq!(m.resource_version, "");
        assert_eq!(m.uid, "");
    }

    #[test]
    fn object_meta_named_sets_name_only() {
        let m = ObjectMeta::named("my-pod");
        assert_eq!(m.name, "my-pod");
        assert_eq!(m.namespace, "");
        assert!(m.labels.is_empty());
    }

    #[test]
    fn object_meta_namespaced_sets_name_and_namespace() {
        let m = ObjectMeta::namespaced("kube-system", "coredns");
        assert_eq!(m.name, "coredns");
        assert_eq!(m.namespace, "kube-system");
    }

    #[test]
    fn object_meta_builders_add_labels_and_annotations() {
        let m = ObjectMeta::named("x")
            .with_label("app", "web")
            .with_annotation("note", "hi");
        assert_eq!(m.labels.get("app").map(String::as_str), Some("web"));
        assert_eq!(m.annotations.get("note").map(String::as_str), Some("hi"));
    }

    // --- Resource trait: composed type metadata -----------------------

    /// Core-group test object (apiVersion should be bare "v1").
    struct CorePod {
        meta: ObjectMeta,
    }
    impl Resource for CorePod {
        fn gvk() -> GroupVersionKind {
            GroupVersionKind::new("", "v1", "Pod")
        }
        fn object_meta(&self) -> &ObjectMeta {
            &self.meta
        }
    }

    /// Named-group test object (apiVersion should be "apps/v1").
    struct AppsDeployment {
        meta: ObjectMeta,
    }
    impl Resource for AppsDeployment {
        fn gvk() -> GroupVersionKind {
            GroupVersionKind::new("apps", "v1", "Deployment")
        }
        fn object_meta(&self) -> &ObjectMeta {
            &self.meta
        }
    }

    #[test]
    fn resource_core_group_api_version_is_bare_version() {
        assert_eq!(CorePod::api_version(), "v1");
    }

    #[test]
    fn resource_named_group_api_version_composes() {
        assert_eq!(AppsDeployment::api_version(), "apps/v1");
    }

    #[test]
    fn resource_type_meta_kind_matches_gvk_kind() {
        assert_eq!(CorePod::type_meta().kind, "Pod");
        assert_eq!(AppsDeployment::type_meta().kind, "Deployment");
        // and the convenience kind() accessor agrees
        assert_eq!(CorePod::kind(), "Pod");
        assert_eq!(AppsDeployment::kind(), "Deployment");
    }

    #[test]
    fn resource_type_meta_carries_composed_api_version() {
        assert_eq!(CorePod::type_meta().api_version, "v1");
        assert_eq!(AppsDeployment::type_meta().api_version, "apps/v1");
    }

    #[test]
    fn resource_object_meta_labels_annotations_default_empty() {
        let p = CorePod {
            meta: ObjectMeta::named("p"),
        };
        assert!(p.object_meta().labels.is_empty());
        assert!(p.object_meta().annotations.is_empty());
        assert_eq!(p.object_meta().name, "p");
    }

    // --- serde round-trips -------------------------------------------

    #[test]
    fn object_meta_round_trips_through_serde_with_renamed_fields() {
        let m = ObjectMeta {
            name: "web".into(),
            namespace: "prod".into(),
            resource_version: "42".into(),
            uid: "abc-123".into(),
            ..Default::default()
        }
        .with_label("app", "web");
        let json = serde_json::to_string(&m).unwrap();
        // camelCase wire field names must be emitted
        assert!(json.contains("\"resourceVersion\":\"42\""), "got {json}");
        assert!(json.contains("\"uid\":\"abc-123\""), "got {json}");
        let back: ObjectMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn empty_object_meta_serializes_to_empty_object() {
        // every field is skip_serializing_if-empty, so a default meta
        // is "{}" — no noisy nulls on the wire.
        let json = serde_json::to_string(&ObjectMeta::default()).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn type_meta_serializes_api_version_as_camel_case() {
        let json = serde_json::to_string(&TypeMeta::new("apps/v1", "Deployment")).unwrap();
        assert!(json.contains("\"apiVersion\":\"apps/v1\""), "got {json}");
        assert!(json.contains("\"kind\":\"Deployment\""), "got {json}");
    }

    #[test]
    fn labels_serialize_in_deterministic_sorted_order() {
        let m = ObjectMeta::named("x")
            .with_label("zeta", "1")
            .with_label("alpha", "2");
        let json = serde_json::to_string(&m).unwrap();
        // BTreeMap => alpha before zeta regardless of insertion order
        let a = json.find("alpha").unwrap();
        let z = json.find("zeta").unwrap();
        assert!(a < z, "labels not key-sorted: {json}");
    }
}