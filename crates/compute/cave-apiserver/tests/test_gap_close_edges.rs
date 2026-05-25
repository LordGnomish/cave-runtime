// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge / failure / boundary coverage for cave-apiserver — error, selectors,
//! ResourceStore CRUD + watch, Resource enum dispatch, ObjectMeta defaults.

use cave_apiserver::error::{ApiError, ApiResult};
use cave_apiserver::resources::{
    ConfigMap, Namespace, NamespaceStatus, ObjectMeta, ObjectReference, Pod, PodSpec, PodStatus,
    Resource, Secret,
};
use cave_apiserver::selectors::{FieldSelector, LabelOperator, LabelSelector};
use cave_apiserver::store::{ResourceStore, WatchEventType};
use std::collections::{BTreeMap, HashMap};

fn cm(name: &str, ns: &str) -> Resource {
    Resource::ConfigMap(ConfigMap {
        api_version: "v1".into(),
        kind: "ConfigMap".into(),
        metadata: ObjectMeta::new(name, ns),
        data: HashMap::new(),
    })
}

fn secret(name: &str, ns: &str, t: &str) -> Resource {
    Resource::Secret(Secret {
        api_version: "v1".into(),
        kind: "Secret".into(),
        metadata: ObjectMeta::new(name, ns),
        data: HashMap::new(),
        secret_type: t.into(),
    })
}

// ---------------------------------------------------------------------------
// ApiError display
// ---------------------------------------------------------------------------

#[test]
fn api_error_display_includes_kind_and_name() {
    let e = ApiError::NotFound { kind: "Pod".into(), name: "nginx".into() };
    let s = e.to_string();
    assert!(s.contains("Pod"));
    assert!(s.contains("nginx"));
}

#[test]
fn api_error_already_exists_includes_context() {
    let e = ApiError::AlreadyExists { kind: "ConfigMap".into(), name: "settings".into() };
    let s = e.to_string();
    assert!(s.contains("ConfigMap"));
    assert!(s.contains("settings"));
}

#[test]
fn api_error_other_variants_print_with_message() {
    assert!(ApiError::Conflict("rv mismatch".into()).to_string().contains("rv"));
    assert!(ApiError::Invalid("bad-spec".into()).to_string().contains("bad-spec"));
    assert!(ApiError::Forbidden("rbac".into()).to_string().contains("rbac"));
    assert!(ApiError::Internal("boom".into()).to_string().contains("boom"));
}

#[test]
fn api_result_alias_pattern_matches() {
    fn ok() -> ApiResult<u32> { Ok(7) }
    fn err() -> ApiResult<u32> { Err(ApiError::Forbidden("x".into())) }
    assert_eq!(ok().unwrap(), 7);
    assert!(matches!(err(), Err(ApiError::Forbidden(_))));
}

// ---------------------------------------------------------------------------
// ObjectMeta defaults
// ---------------------------------------------------------------------------

#[test]
fn object_meta_new_starts_resource_version_at_1() {
    let m = ObjectMeta::new("a", "default");
    assert_eq!(m.resource_version, 1);
    assert!(m.labels.is_empty());
    assert!(m.annotations.is_empty());
    assert!(m.owner_references.is_empty());
    assert!(m.finalizers.is_empty());
    assert!(m.deletion_timestamp.is_none());
}

#[test]
fn object_meta_new_assigns_unique_uids() {
    let a = ObjectMeta::new("x", "ns");
    let b = ObjectMeta::new("x", "ns");
    assert_ne!(a.uid, b.uid);
}

#[test]
fn object_reference_default_has_empty_fields() {
    let r = ObjectReference::default();
    assert!(r.kind.is_empty());
    assert!(r.name.is_empty());
    assert!(r.namespace.is_empty());
    assert!(r.api_version.is_none());
    assert!(r.uid.is_none());
}

// ---------------------------------------------------------------------------
// LabelSelector parse + match edge cases
// ---------------------------------------------------------------------------

fn lbls(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn label_selector_empty_parse_matches_anything() {
    let sel = LabelSelector::parse("").unwrap();
    assert!(sel.matches(&BTreeMap::new()));
    assert!(sel.matches(&lbls(&[("env", "prod")])));
}

#[test]
fn label_selector_parse_whitespace_only_acts_like_empty() {
    let sel = LabelSelector::parse("    ").unwrap();
    assert!(sel.requirements.is_empty());
}

#[test]
fn label_selector_in_with_empty_value_set_never_matches() {
    let sel = LabelSelector::parse("env in ()").unwrap();
    assert_eq!(sel.requirements.len(), 1);
    // Empty set → no value can satisfy `in (...)`
    assert!(!sel.matches(&lbls(&[("env", "prod")])));
}

#[test]
fn label_selector_negation_requires_absent_key() {
    let sel = LabelSelector::parse("!flag").unwrap();
    assert!(sel.matches(&BTreeMap::new()));
    assert!(!sel.matches(&lbls(&[("flag", "true")])));
}

#[test]
fn label_selector_operator_enum_round_trip_from_requirements() {
    let sel = LabelSelector::parse("env=prod, tier in (web, api), !canary, region != us-west").unwrap();
    let ops: Vec<&LabelOperator> = sel.requirements.iter().map(|r| &r.op).collect();
    assert!(matches!(ops[0], LabelOperator::Equals(_)));
    assert!(matches!(ops[1], LabelOperator::In(_)));
    assert!(matches!(ops[2], LabelOperator::DoesNotExist));
    assert!(matches!(ops[3], LabelOperator::NotEquals(_)));
}

// ---------------------------------------------------------------------------
// FieldSelector
// ---------------------------------------------------------------------------

#[test]
fn field_selector_empty_matches_anything() {
    let sel = FieldSelector::parse("").unwrap();
    assert!(sel.matches(&BTreeMap::new()));
}

#[test]
fn field_selector_rejects_unknown_operators() {
    let res = FieldSelector::parse("metadata.name in (a,b)");
    assert!(res.is_err());
    let res = FieldSelector::parse("nonsense-without-operator");
    assert!(res.is_err());
}

#[test]
fn field_selector_not_equals_passes_when_field_absent() {
    let sel = FieldSelector::parse("status.phase!=Failed").unwrap();
    assert!(sel.matches(&BTreeMap::new()), "absent field passes !=");
    let fm: BTreeMap<String, String> =
        [("status.phase".to_string(), "Failed".to_string())].into_iter().collect();
    assert!(!sel.matches(&fm));
}

// ---------------------------------------------------------------------------
// ResourceStore CRUD edges
// ---------------------------------------------------------------------------

#[test]
fn store_create_returns_stored_resource() {
    let s = ResourceStore::new();
    let res = s.create(cm("c1", "default")).unwrap();
    assert_eq!(res.name(), "c1");
    assert_eq!(res.namespace(), "default");
    assert_eq!(res.kind(), "ConfigMap");
}

#[test]
fn store_create_duplicate_errors_already_exists() {
    let s = ResourceStore::new();
    s.create(cm("c1", "default")).unwrap();
    let err = s.create(cm("c1", "default")).unwrap_err();
    assert!(matches!(err, ApiError::AlreadyExists { .. }));
}

#[test]
fn store_get_missing_returns_not_found() {
    let s = ResourceStore::new();
    let err = s.get("ConfigMap", "default", "absent").unwrap_err();
    assert!(matches!(err, ApiError::NotFound { .. }));
}

#[test]
fn store_update_missing_returns_not_found() {
    let s = ResourceStore::new();
    let err = s.update(cm("nope", "default")).unwrap_err();
    assert!(matches!(err, ApiError::NotFound { .. }));
}

#[test]
fn store_update_overwrites_existing() {
    let s = ResourceStore::new();
    s.create(cm("c", "ns")).unwrap();
    let mut updated = cm("c", "ns");
    if let Resource::ConfigMap(ref mut c) = updated {
        c.data.insert("k".into(), "v".into());
    }
    s.update(updated).unwrap();
    let got = s.get("ConfigMap", "ns", "c").unwrap();
    if let Resource::ConfigMap(c) = got {
        assert_eq!(c.data.get("k"), Some(&"v".to_string()));
    } else {
        panic!("expected ConfigMap");
    }
}

#[test]
fn store_delete_returns_removed_resource() {
    let s = ResourceStore::new();
    s.create(cm("d", "ns")).unwrap();
    let removed = s.delete("ConfigMap", "ns", "d").unwrap();
    assert_eq!(removed.name(), "d");
    assert!(s.get("ConfigMap", "ns", "d").is_err());
}

#[test]
fn store_delete_missing_returns_not_found() {
    let s = ResourceStore::new();
    let err = s.delete("ConfigMap", "ns", "missing").unwrap_err();
    assert!(matches!(err, ApiError::NotFound { .. }));
}

#[test]
fn store_list_filters_by_kind_and_namespace() {
    let s = ResourceStore::new();
    s.create(cm("a", "ns1")).unwrap();
    s.create(cm("b", "ns1")).unwrap();
    s.create(cm("c", "ns2")).unwrap();
    s.create(secret("s1", "ns1", "Opaque")).unwrap();

    let ns1_cms = s.list("ConfigMap", "ns1");
    assert_eq!(ns1_cms.len(), 2);
    let ns2_cms = s.list("ConfigMap", "ns2");
    assert_eq!(ns2_cms.len(), 1);
    // Empty namespace → all namespaces
    let all_cms = s.list("ConfigMap", "");
    assert_eq!(all_cms.len(), 3);
}

#[test]
fn store_count_per_kind() {
    let s = ResourceStore::new();
    s.create(cm("a", "ns")).unwrap();
    s.create(cm("b", "ns")).unwrap();
    s.create(secret("s", "ns", "Opaque")).unwrap();
    assert_eq!(s.count("ConfigMap"), 2);
    assert_eq!(s.count("Secret"), 1);
    assert_eq!(s.count("Pod"), 0);
}

#[test]
fn store_list_all_returns_every_resource() {
    let s = ResourceStore::new();
    s.create(cm("a", "ns")).unwrap();
    s.create(secret("s", "ns", "Opaque")).unwrap();
    let all = s.list_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn store_upsert_replaces_without_alreadyexists_error() {
    let s = ResourceStore::new();
    s.upsert(cm("u", "ns"));
    s.upsert(cm("u", "ns")); // No error
    assert_eq!(s.count("ConfigMap"), 1);
}

#[tokio::test]
async fn store_watch_observes_added_event() {
    let s = ResourceStore::new();
    let mut sub = s.subscribe();
    s.create(cm("x", "ns")).unwrap();
    let ev = sub.recv().await.expect("must receive added event");
    assert!(matches!(ev.event_type, WatchEventType::Added));
    assert_eq!(ev.resource.name(), "x");
}

#[tokio::test]
async fn store_watch_observes_modified_and_deleted_events() {
    let s = ResourceStore::new();
    s.create(cm("y", "ns")).unwrap();
    let mut sub = s.subscribe();
    s.update(cm("y", "ns")).unwrap();
    let ev = sub.recv().await.unwrap();
    assert!(matches!(ev.event_type, WatchEventType::Modified));
    s.delete("ConfigMap", "ns", "y").unwrap();
    let ev = sub.recv().await.unwrap();
    assert!(matches!(ev.event_type, WatchEventType::Deleted));
}

// ---------------------------------------------------------------------------
// Resource enum dispatch
// ---------------------------------------------------------------------------

#[test]
fn resource_kind_dispatches_per_variant() {
    let c = cm("c", "ns");
    let s = secret("s", "ns", "Opaque");
    assert_eq!(c.kind(), "ConfigMap");
    assert_eq!(s.kind(), "Secret");
}

#[test]
fn resource_metadata_returns_underlying_meta() {
    let c = cm("the-name", "the-ns");
    let m = c.metadata();
    assert_eq!(m.name, "the-name");
    assert_eq!(m.namespace, "the-ns");
}

#[test]
fn resource_pod_kind_and_namespace() {
    let pod = Resource::Pod(Pod {
        api_version: "v1".into(),
        kind: "Pod".into(),
        metadata: ObjectMeta::new("p", "default"),
        spec: PodSpec::default(),
        status: PodStatus::default(),
    });
    assert_eq!(pod.kind(), "Pod");
    assert_eq!(pod.namespace(), "default");
}

#[test]
fn resource_namespace_is_cluster_scoped_via_empty_ns() {
    let ns = Resource::Namespace(Namespace {
        api_version: "v1".into(),
        kind: "Namespace".into(),
        metadata: ObjectMeta::new("kube-system", ""),
        status: NamespaceStatus::default(),
    });
    assert_eq!(ns.kind(), "Namespace");
    assert_eq!(ns.name(), "kube-system");
    assert_eq!(ns.namespace(), "", "Namespace is cluster-scoped");
}

#[test]
fn pod_spec_default_restart_policy_is_always() {
    let s = PodSpec::default();
    assert_eq!(s.restart_policy, "Always");
    assert!(s.containers.is_empty());
    assert!(s.node_name.is_none());
}
