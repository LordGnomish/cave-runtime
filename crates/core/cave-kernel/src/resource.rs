// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

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